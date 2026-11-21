//! One worker thread per Rex host. It owns the control connection, keeps a model of the
//! host's sessions current, and streams the terminals the UI shows. The UI reads the
//! model under a lock and sends commands; every change wakes the UI.
use super::{
    activity::{self, BlockEvent, Process, StatusRecord},
    client::{self, Control, Event, Identity},
    data::{self, PROTOCOL, TERMINAL_CREATOR},
    endpoint::{self, Endpoint},
    prompt,
    stream::{CursorShape, Frame, SelectMode, TermEvent, TerminalState, TerminalTheme, copy_mode},
    view::{SessionSummary, View},
};
use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::oneshot;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Connection {
    #[default]
    Connecting,
    Connected,
    /// Waiting to retry, with the reason the last attempt failed.
    Disconnected(String),
}

/// What the client knows about one block, streamed or not.
#[derive(Clone, Debug, Default)]
pub struct BlockState {
    pub session_id: String,
    pub records: BTreeMap<String, StatusRecord>,
    pub process: Process,
    /// The title a program set, and the pid that set it.
    pub title: Option<(String, Option<u64>)>,
    pub pwd: Option<String>,
    pub progress: Option<(String, Option<u8>)>,
    pub exited: Option<i64>,
    /// Bumped on each bell.
    pub bells: u64,
}

impl BlockState {
    /// The title while it's current: shells rarely clear a title when the program that set
    /// it exits, so it counts only while that program is still in the foreground.
    pub fn current_title(&self) -> Option<&str> {
        let (text, owner) = self.title.as_ref()?;
        match owner {
            Some(pid) if !self.process.pids.is_empty() && !self.process.pids.contains(pid) => None,
            _ => Some(text),
        }
    }

    pub fn status(&self) -> Option<&StatusRecord> {
        activity::summary(&self.records)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notification {
    pub session_id: String,
    pub block_id: String,
    pub title: String,
    pub body: String,
}

/// Another client asked this one to perform an action (`client.perform`).
#[derive(Clone, Debug, PartialEq)]
pub struct PerformRequest {
    pub call_id: String,
    pub action: String,
    pub args: Value,
    pub session_id: Option<String>,
    pub block_id: Option<String>,
    pub origin: String,
}

#[derive(Clone, Debug, Default)]
pub struct HostModel {
    pub connection: Connection,
    pub api_revision: u32,
    pub client_id: Option<String>,
    pub server: Value,
    pub sessions: Vec<SessionSummary>,
    pub views: HashMap<String, View>,
    pub selected: Option<String>,
    /// The preceding session shown by this client on this host, not Rex's
    /// principal-wide focus-history cursor. Only successful changes replace it.
    pub last_selected: Option<String>,
    pub panes: HashMap<String, PaneHandle>,
    pub blocks: HashMap<String, BlockState>,
    /// Errors from actions, for the UI to show and clear.
    pub notices: Vec<String>,
    pub notifications: Vec<Notification>,
    pub performs: Vec<PerformRequest>,
    pub prompt: Option<prompt::Request>,
    pub keymap: Option<Value>,
    pub hosts: Vec<Value>,
    /// Bumped on every reconnect, when activity is re-read from scratch.
    pub epoch: u64,
    /// Tabs this client shows instead of the session's active one, by session. The server
    /// keeps one active tab per session, so a window that split off a tab pins its own.
    pub pins: HashMap<String, String>,
    /// Onboarding walkthroughs' steps by block, from `step_changed` events:
    /// (generation, step).
    pub onboarding: HashMap<String, (u64, String)>,
    /// Bumped on every `tailscale_status_changed` event, which updates `server`.
    pub tailscale_revision: u64,
}

impl HostModel {
    fn select(&mut self, id: String) {
        if self.selected.as_ref() != Some(&id) {
            self.last_selected = self.selected.replace(id);
        }
    }

    /// Show the pinned tab as the session's active one, while it still exists.
    fn apply_pin(&mut self, session: &str) {
        let Some(pinned) = self.pins.get(session).cloned() else {
            return;
        };
        match self.views.get_mut(session) {
            Some(view) if view.window(&pinned).is_some() => {
                view.active_window_id = Some(pinned);
            }
            // The pinned tab closed: follow the session again.
            Some(_) => {
                self.pins.remove(session);
            }
            None => {}
        }
    }

    pub fn pin(&mut self, session: String, window: Option<String>) {
        match window {
            Some(window) => {
                self.pins.insert(session.clone(), window);
                self.apply_pin(&session);
            }
            None => {
                self.pins.remove(&session);
            }
        }
    }

    pub fn view(&self) -> Option<&View> {
        self.views.get(self.selected.as_ref()?)
    }

    pub fn session(&self, id: &str) -> Option<&SessionSummary> {
        self.sessions.iter().find(|s| s.session_id == id)
    }

    pub fn session_of_block(&self, block_id: &str) -> Option<&str> {
        self.views
            .iter()
            .find(|(_, view)| view.window_of(block_id).is_some())
            .map(|(id, _)| id.as_str())
    }

    fn forget_block(&mut self, block_id: &str) {
        self.blocks.remove(block_id);
        self.panes.remove(block_id);
    }

    fn apply_block_event(&mut self, session: &str, block: String, event: BlockEvent) {
        if let BlockEvent::Notification { title, body } = &event {
            if self.notifications.len() < 32 {
                self.notifications.push(Notification {
                    session_id: session.into(),
                    block_id: block.clone(),
                    title: title.clone(),
                    body: body.clone(),
                });
            }
            return;
        }
        let state = self.blocks.entry(block).or_default();
        state.session_id = session.into();
        match event {
            BlockEvent::StatusChanged { id, record } => {
                state.records.insert(id, record);
            }
            BlockEvent::StatusRemoved { ids } => {
                for id in ids {
                    state.records.remove(&id);
                }
            }
            BlockEvent::Process(process) => state.process = process,
            BlockEvent::Title { text, owner } => {
                state.title = (!text.is_empty()).then_some((text, owner));
            }
            BlockEvent::Pwd(pwd) => state.pwd = Some(pwd),
            BlockEvent::Progress {
                state: kind,
                progress,
            } => {
                state.progress = (kind != "remove").then_some((kind, progress));
            }
            BlockEvent::Bell => state.bells += 1,
            BlockEvent::Exited { code } => state.exited = Some(code),
            BlockEvent::Notification { .. } => unreachable!("handled above"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PaneStatus {
    #[default]
    Restoring,
    Live,
    Reconnecting(String),
    /// The block is gone on the server.
    Closed,
}

#[derive(Clone, Default)]
pub struct PaneModel {
    pub frame: Option<Arc<Frame>>,
    pub status: PaneStatus,
    /// This client owns the block, so its size follows this client.
    pub owner: bool,
    pub revision: u64,
    /// Text a program copied (OSC 52), for the UI to put on the clipboard.
    pub clipboard: Option<String>,
    pub bells: u64,
    pub exit_code: Option<i64>,
    // Keyboard routing needs to see the transition before rendering catches up.
    copy_mode: bool,
}

pub enum Scroll {
    Lines(isize),
    Top,
    Bottom,
    Row(usize),
}

pub enum PaneCommand {
    Visible(bool),
    Scroll(Scroll),
    Select {
        x: u16,
        y: u16,
        mode: SelectMode,
    },
    SelectTo {
        x: u16,
        y: u16,
    },
    SelectAll,
    ClearSelection,
    Copy(oneshot::Sender<Option<String>>),
    CopyMode(copy_mode::Command),
    YankCopyMode {
        to_line_end: bool,
        reply: oneshot::Sender<Option<String>>,
    },
    Text(oneshot::Sender<Option<String>>),
    Search(Option<String>),
    SearchStep {
        older: bool,
    },
    Prompt(i32),
    /// Reflow the local terminal to a new size now, before the server answers a resize.
    Reflow {
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
    },
    Theme(Option<TerminalTheme>),
    DefaultCursor(Option<CursorShape>),
    Reconnect,
    Stop,
}

#[derive(Clone)]
pub struct PaneHandle {
    pub model: Arc<Mutex<PaneModel>>,
    commands: flume::Sender<PaneCommand>,
}

impl std::fmt::Debug for PaneHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaneHandle").finish_non_exhaustive()
    }
}

impl PaneHandle {
    pub fn in_copy_mode(&self) -> bool {
        self.model.lock().expect("pane model").copy_mode
    }

    pub fn copy_mode(&self, command: copy_mode::Command) -> bool {
        let mut model = self.model.lock().expect("pane model");
        match command {
            copy_mode::Command::Enter if model.status != PaneStatus::Live => return false,
            copy_mode::Command::Enter if model.copy_mode => return true,
            copy_mode::Command::Enter => model.copy_mode = true,
            _ if !model.copy_mode => return true,
            copy_mode::Command::Exit => model.copy_mode = false,
            _ => {}
        }
        self.send(PaneCommand::CopyMode(command));
        true
    }

    pub fn yank_copy_mode(&self, to_line_end: bool) -> Option<oneshot::Receiver<Option<String>>> {
        let mut model = self.model.lock().expect("pane model");
        if !model.copy_mode {
            return None;
        }
        let (reply, answer) = oneshot::channel();
        model.copy_mode = false;
        self.send(PaneCommand::YankCopyMode { to_line_end, reply });
        Some(answer)
    }

    pub fn send(&self, command: PaneCommand) {
        let _ = self.commands.send(command);
    }

    pub fn snapshot(&self) -> PaneModel {
        self.model.lock().expect("pane model").clone()
    }

    /// Ask the stream something and wait for the answer.
    pub async fn ask<T>(&self, build: impl FnOnce(oneshot::Sender<T>) -> PaneCommand) -> Option<T> {
        let (reply, answer) = oneshot::channel();
        self.send(build(reply));
        answer.await.ok()
    }
}

pub type Reply = oneshot::Sender<Result<Value, String>>;

pub enum HostCommand {
    /// Show this host's selected session, or stop streaming while it's hidden.
    Visible(bool),
    Select(String),
    /// Load a session's tabs without changing the selection or streaming its panes.
    LoadSession(String),
    CreateSession {
        label: Option<String>,
        reply: Option<Reply>,
    },
    /// Any control method. Session methods refresh the view through the events they cause.
    Call {
        method: String,
        payload: Value,
        reply: Option<Reply>,
    },
    /// Typing, pasting and clicking in a pane: claims the block first if another client has it.
    Input {
        session_id: String,
        block_id: String,
        method: &'static str,
        payload: Value,
        claim: bool,
    },
    Resize {
        session_id: String,
        block_id: String,
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
    },
    /// Fetch the keymap with the client's own layers and actions (`client.keymap`).
    Keymap(Value),
    /// Answer a `client_perform_requested` event.
    PerformReply {
        call_id: String,
        result: Result<Value, (String, String)>,
    },
    PromptReply {
        call_id: String,
        result: Value,
    },
    SetLabel(Option<String>),
    /// Show this tab of the session in this client whatever the session's active tab is,
    /// or (None) follow the session again.
    Pin {
        session: String,
        window: Option<String>,
    },
    TerminalTheme(Option<TerminalTheme>),
    /// The cursor shape terminals use until their program picks one.
    DefaultCursor(Option<CursorShape>),
    Reconnect,
    Shutdown,
}

/// The UI's side of a host worker.
#[derive(Clone)]
pub struct HostHandle {
    pub endpoint: Endpoint,
    pub model: Arc<Mutex<HostModel>>,
    commands: flume::Sender<HostCommand>,
    pub wake: flume::Receiver<()>,
}

impl HostHandle {
    pub fn spawn(endpoint: Endpoint, identity: Identity, preferred: Option<String>) -> Self {
        let model = Arc::new(Mutex::new(HostModel::default()));
        let (commands, receiver) = flume::unbounded();
        let (wake_tx, wake) = flume::bounded(1);
        let shared = Shared {
            model: model.clone(),
            wake: wake_tx,
        };
        let worker_endpoint = endpoint.clone();
        std::thread::Builder::new()
            .name("rex-host".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("host runtime");
                let local = tokio::task::LocalSet::new();
                local.block_on(
                    &runtime,
                    run(worker_endpoint, identity, preferred, receiver, shared),
                );
            })
            .expect("start host worker");
        Self {
            endpoint,
            model,
            commands,
            wake,
        }
    }

    pub fn send(&self, command: HostCommand) {
        let _ = self.commands.send(command);
    }

    pub fn call(&self, method: &str, payload: Value) {
        self.send(HostCommand::Call {
            method: method.into(),
            payload,
            reply: None,
        });
    }

    /// Call a method and wait for its result.
    pub async fn request(&self, method: &str, payload: Value) -> Result<Value, String> {
        let (reply, answer) = oneshot::channel();
        self.send(HostCommand::Call {
            method: method.into(),
            payload,
            reply: Some(reply),
        });
        answer
            .await
            .unwrap_or_else(|_| Err("the host stopped".into()))
    }

    /// Inherit the source process's current directory, even when its shell emits no OSC 7.
    pub async fn shell_layout_from(&self, session: &str, source: Option<&str>) -> Value {
        let cwd = if let Some(block) = source {
            self.process_directory(session, block).await
        } else {
            None
        };
        shell_layout_for(self.endpoint.is_local(), cwd.as_deref())
    }

    /// The process's live directory, falling back to the most recent reported directory.
    pub async fn process_directory(&self, session: &str, block: &str) -> Option<String> {
        let cached = self
            .model
            .lock()
            .expect("host model")
            .blocks
            .get(block)
            .and_then(|b| {
                b.pwd
                    .as_ref()
                    .filter(|p| !p.is_empty())
                    .or(b.process.cwd.as_ref().filter(|p| !p.is_empty()))
                    .cloned()
            });
        self.request(
            &format!("{TERMINAL_CREATOR}.process"),
            json!({"session_id": session, "block_id": block, "args": {}}),
        )
        .await
        .ok()
        .and_then(|reply| Process::parse(&reply).cwd)
        .filter(|path| !path.is_empty())
        .or(cached)
    }

    pub fn snapshot(&self) -> HostModel {
        self.model.lock().expect("host model").clone()
    }

    pub fn pane(&self, block_id: &str) -> Option<PaneHandle> {
        self.model
            .lock()
            .expect("host model")
            .panes
            .get(block_id)
            .cloned()
    }
}

impl Drop for HostHandle {
    fn drop(&mut self) {
        // Only the last handle stops the worker.
        if self.commands.sender_count() == 1 {
            let _ = self.commands.send(HostCommand::Shutdown);
        }
    }
}

#[derive(Clone)]
struct Shared {
    model: Arc<Mutex<HostModel>>,
    wake: flume::Sender<()>,
}

impl Shared {
    fn update<R>(&self, change: impl FnOnce(&mut HostModel) -> R) -> R {
        let result = change(&mut self.model.lock().expect("host model"));
        let _ = self.wake.try_send(());
        result
    }

    fn read<R>(&self, read: impl FnOnce(&HostModel) -> R) -> R {
        read(&self.model.lock().expect("host model"))
    }

    fn notice(&self, message: impl Into<String>) {
        let message = message.into();
        self.update(|m| {
            if m.notices.len() < 8 {
                m.notices.push(message);
            }
        });
    }
}

struct Ticket {
    token: String,
    expires: Instant,
}

/// The control connection and the tickets a data worker can use. Shared with workers,
/// which call `data.connect` themselves when the bundled ticket is gone.
#[derive(Default)]
struct Link {
    control: Option<Control>,
    tickets: HashMap<String, Ticket>,
}

type SharedLink = Arc<Mutex<Link>>;

struct Worker {
    session_id: String,
    pane: PaneHandle,
    size: Option<(u16, u16, u16, u16)>,
    visible: bool,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Event sequence floors: an event at or below the floor is already in a snapshot.
#[derive(Default)]
struct Floors {
    bootstrap: u64,
    list: u64,
    views: HashMap<String, u64>,
    /// Blocks whose program status and title were read after this connection started.
    queried: HashSet<String>,
}

impl Floors {
    fn admits(&self, event: &Event) -> bool {
        let mut floor = self.bootstrap;
        if event.name.starts_with("session_") && event.name != "session_view_changed" {
            floor = floor.max(self.list);
        }
        if event.name != "session_destroyed"
            && let Some(id) = &event.session_id
        {
            floor = floor.max(self.views.get(id).copied().unwrap_or(0));
        }
        event.seq > floor
    }
}

struct Host {
    endpoint: Endpoint,
    identity: Identity,
    shared: Shared,
    link: SharedLink,
    workers: HashMap<String, Worker>,
    visible: bool,
    preferred: Option<String>,
    keymap_request: Option<Value>,
    theme: Option<TerminalTheme>,
    cursor: Option<CursorShape>,
}

async fn run(
    endpoint: Endpoint,
    identity: Identity,
    preferred: Option<String>,
    commands: flume::Receiver<HostCommand>,
    shared: Shared,
) {
    let mut host = Host {
        endpoint,
        identity,
        shared,
        link: Arc::default(),
        workers: HashMap::new(),
        visible: true,
        preferred,
        keymap_request: None,
        theme: None,
        cursor: None,
    };
    let mut token: Option<String> = None;
    let mut retry = 0u32;
    // The same refusal from ssh, and how many times in a row.
    let mut refused: Option<(endpoint::Refusal, u32)> = None;
    loop {
        let outcome = host.session(&commands, &mut token).await;
        host.link.lock().expect("link").control = None;
        host.shared.update(|m| m.prompt = None);
        let mut stopped = false;
        match outcome {
            Ok(Stop::Shutdown) => return,
            Ok(Stop::Reconnect) => {
                retry = 0;
                refused = None;
            }
            Err(error) => {
                let failure = format!("{error:#}");
                refused = match (endpoint::Refusal::of(&failure), refused) {
                    (Some(now), Some((before, n))) if now == before => Some((now, n + 1)),
                    (Some(now), _) => Some((now, 1)),
                    (None, _) => None,
                };
                let message = match refused {
                    Some((refusal, n)) if n >= endpoint::Refusal::LIMIT => {
                        stopped = true;
                        refusal.advice(&host.endpoint)
                    }
                    _ => failure,
                };
                host.shared.update(|m| {
                    m.connection = Connection::Disconnected(message);
                });
                retry += 1;
            }
        }
        // Back off 1, 2, 4, 8 s; a Reconnect command retries at once. After repeated
        // refusals only a Reconnect (Retry) does: trying again can't fix them.
        let backoff = Duration::from_secs(1 << retry.min(3)).min(Duration::from_secs(8));
        let delay = async move {
            if stopped {
                std::future::pending::<()>().await
            } else {
                tokio::time::sleep(backoff).await
            }
        };
        tokio::pin!(delay);
        loop {
            tokio::select! {
                _ = &mut delay => break,
                command = commands.recv_async() => match command {
                    Ok(HostCommand::Shutdown) | Err(_) => return,
                    Ok(HostCommand::Reconnect) => { retry = 0; refused = None; break }
                    Ok(HostCommand::Select(id)) => {
                        host.preferred = Some(id.clone());
                        host.shared.update(|m| m.select(id));
                    }
                    Ok(HostCommand::Visible(visible)) => host.visible = visible,
                    Ok(HostCommand::Keymap(request)) => host.keymap_request = Some(request),
                    Ok(HostCommand::TerminalTheme(theme)) => host.set_theme(theme),
                    Ok(HostCommand::DefaultCursor(shape)) => host.set_cursor(shape),
                    Ok(HostCommand::SetLabel(label)) => host.identity.label = label,
                    Ok(HostCommand::Pin { session, window }) => {
                        host.shared.update(|m| m.pin(session, window));
                    }
                    Ok(HostCommand::Resize { block_id, cols, rows, cell_width, cell_height, .. }) => {
                        if let Some(worker) = host.workers.get_mut(&block_id) {
                            worker.size = Some((cols, rows, cell_width, cell_height));
                        }
                    }
                    Ok(HostCommand::Call { reply: Some(reply), .. } | HostCommand::CreateSession { reply: Some(reply), .. }) => {
                        let _ = reply.send(Err("Not connected. Try again once the host reconnects.".into()));
                    }
                    // Input typed while disconnected is dropped, never replayed later.
                    Ok(_) => {}
                }
            }
        }
        host.shared
            .update(|m| m.connection = Connection::Connecting);
    }
}

enum Stop {
    Shutdown,
    Reconnect,
}

impl Host {
    fn start_promptable(
        &self,
        control: &Control,
        method: &str,
        payload: Value,
        reply: Option<Reply>,
    ) -> Result<()> {
        let answer = control.start_call(method, payload, client::ACTION_TIMEOUT)?;
        let shared = self.shared.clone();
        tokio::spawn(async move {
            let result = answer.await;
            if let Some(reply) = reply {
                let _ = reply.send(
                    result
                        .map(|reply| reply.value)
                        .map_err(|err| format!("{err:#}")),
                );
            } else if let Err(error) = result
                && !client::is_disconnect(&error)
            {
                shared.notice(format!("{error:#}"));
            }
        });
        Ok(())
    }

    async fn session(
        &mut self,
        commands: &flume::Receiver<HostCommand>,
        token: &mut Option<String>,
    ) -> Result<Stop> {
        self.shared
            .update(|m| m.connection = Connection::Connecting);
        let (control, mut events) =
            Control::connect(&self.endpoint, &self.identity, token.as_deref()).await?;
        *token = control.reconnect_token().map(str::to_owned);
        let mut floors = Floors::default();
        self.bootstrap(&control, &mut floors)
            .await
            .inspect_err(|_| {
                // A partially applied setup must not be resumed under the same identity.
                *token = None;
                control.close();
            })?;
        self.link.lock().expect("link").control = Some(control.clone());
        self.shared.update(|m| m.connection = Connection::Connected);
        // Sizes chosen while disconnected apply now.
        let sizes: Vec<_> = self
            .workers
            .iter()
            .filter_map(|(id, w)| w.size.map(|s| (w.session_id.clone(), id.clone(), s)))
            .collect();
        for (session, block, (cols, rows, cw, ch)) in sizes {
            let _ = resize(&control, &session, &block, cols, rows, cw, ch).await;
        }
        if let Some(request) = self.keymap_request.clone() {
            self.fetch_keymap(&control, request).await;
        }
        self.refresh_hosts(&control).await;
        self.reconcile();
        self.query_activity(&control, &mut floors).await;

        loop {
            tokio::select! {
                event = events.recv() => {
                    let Some(event) = event else {
                        return Err(anyhow!("the connection to Rex ended"));
                    };
                    if matches!(event.name.as_str(), "client_prompt_requested" | "client_prompt_ended")
                        || floors.admits(&event)
                    {
                        self.handle_event(&control, &mut floors, event).await?;
                    }
                }
                command = commands.recv_async() => {
                    let Ok(command) = command else {
                        control.close();
                        return Ok(Stop::Shutdown);
                    };
                    match command {
                        HostCommand::Shutdown => {
                            control.close();
                            return Ok(Stop::Shutdown);
                        }
                        HostCommand::Reconnect => {
                            control.close();
                            return Ok(Stop::Reconnect);
                        }
                        command => {
                            if let Err(error) = self.handle_command(&control, &mut floors, command).await {
                                if client::is_disconnect(&error) {
                                    return Err(error);
                                }
                                self.shared.notice(format!("{error:#}"));
                            }
                        }
                    }
                }
            }
            self.reconcile();
            self.query_activity(&control, &mut floors).await;
        }
    }

    async fn bootstrap(&mut self, control: &Control, floors: &mut Floors) -> Result<()> {
        let (selected, retained) = self.shared.read(|m| {
            (
                self.preferred.clone().or_else(|| m.selected.clone()),
                m.views.keys().cloned().collect::<Vec<_>>(),
            )
        });
        let mut request = json!({
            "event_scope": "all",
            "data_tickets": [{"creator_name": TERMINAL_CREATOR, "data_ref_id": "terminal", "subprotocol": PROTOCOL}],
        });
        let mut attach: Vec<String> = retained;
        if let Some(id) = &selected {
            attach.push(id.clone());
        }
        attach.sort();
        attach.dedup();
        if !attach.is_empty() {
            request["attach_session_ids"] = json!(attach);
        }
        let started = Instant::now();
        let reply = control.call("session.bootstrap", request).await?;
        floors.bootstrap = reply.seq;
        floors.list = reply.seq;
        let boot = reply.value;
        let sessions: Vec<SessionSummary> =
            serde_json::from_value(boot["attach_all"]["sessions"].clone()).unwrap_or_default();
        let mut views = HashMap::new();
        let attached = boot["attached_sessions"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .chain(
                boot["selected_session"]
                    .is_object()
                    .then(|| boot["selected_session"].clone()),
            );
        for entry in attached {
            if let Ok(view) = serde_json::from_value::<View>(entry["view"].clone()) {
                let id = entry["session"]["session_id"]
                    .as_str()
                    .map(str::to_owned)
                    .or(view.session_id.clone())
                    .unwrap_or_default();
                floors.views.insert(id.clone(), reply.seq);
                views.insert(id, view);
            }
        }
        {
            let mut link = self.link.lock().expect("link");
            link.tickets.clear();
            for ticket in boot["data_tickets"].as_array().into_iter().flatten() {
                if ticket["subprotocol"]
                    .as_str()
                    .is_some_and(|p| p != PROTOCOL)
                {
                    continue;
                }
                let (Some(block), Some(token)) =
                    (ticket["block_id"].as_str(), ticket["ticket"].as_str())
                else {
                    continue;
                };
                let lifetime = Duration::from_millis(ticket["expires_in_ms"].as_u64().unwrap_or(0));
                link.tickets.insert(
                    block.into(),
                    Ticket {
                        token: token.into(),
                        expires: started + lifetime,
                    },
                );
            }
        }
        let client_id = control.client_id().map(str::to_owned);
        let server = boot["server_status"].clone();
        let selected = selected
            .filter(|id| sessions.iter().any(|s| &s.session_id == id))
            .or_else(|| views.keys().next().cloned())
            .or_else(|| sessions.first().map(|s| s.session_id.clone()));
        self.preferred = None;
        let live: HashSet<_> = sessions.iter().map(|s| s.session_id.clone()).collect();
        self.workers.retain(|_, w| live.contains(&w.session_id));
        self.shared.update(|m| {
            m.api_revision = control.welcome.api_revision;
            m.client_id = client_id;
            m.server = server;
            m.sessions = sessions;
            m.views = views;
            let pinned: Vec<String> = m.pins.keys().cloned().collect();
            for session in pinned {
                m.apply_pin(&session);
            }
            // Reconnection/initial bootstrap is not an explicit switch. If the
            // selected session disappeared, the new default is a fresh attach.
            if m.selected
                .as_ref()
                .is_some_and(|id| !m.sessions.iter().any(|s| &s.session_id == id))
            {
                m.last_selected = None;
            }
            m.selected = selected.clone();
            // Activity is re-read from scratch: reports made while away were missed.
            for block in m.blocks.values_mut() {
                block.records.clear();
            }
            m.epoch += 1;
        });
        if let Some(id) = selected
            && !floors.views.contains_key(&id)
        {
            self.attach(control, floors, &id).await?;
        }
        self.update_owners();
        Ok(())
    }

    async fn attach(&mut self, control: &Control, floors: &mut Floors, id: &str) -> Result<()> {
        let reply = control
            .call("session.attach", json!({"session_id": id}))
            .await?;
        floors.views.insert(id.into(), reply.seq);
        let view: View = serde_json::from_value(reply.value["view"].clone())
            .context("unreadable session view")?;
        self.apply_view(id, view);
        Ok(())
    }

    async fn refresh_view(
        &mut self,
        control: &Control,
        floors: &mut Floors,
        id: &str,
    ) -> Result<()> {
        match control
            .call("session.view", json!({"session_id": id}))
            .await
        {
            Ok(reply) => {
                floors.views.insert(id.into(), reply.seq);
                let view: View =
                    serde_json::from_value(reply.value).context("unreadable session view")?;
                self.apply_view(id, view);
                Ok(())
            }
            Err(error) if client::error_code(&error) == Some("session_not_found") => {
                self.remove_session(id);
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    fn apply_view(&mut self, id: &str, view: View) {
        self.shared.update(|m| {
            if m.views
                .get(id)
                .is_some_and(|old| old.revision > view.revision)
            {
                return;
            }
            if let Some(session) = m.sessions.iter_mut().find(|s| s.session_id == id)
                && !view.label.is_empty()
            {
                session.label = view.label.clone();
            }
            m.views.insert(id.into(), view);
            m.apply_pin(id);
        });
        self.update_owners();
    }

    fn update_owners(&self) {
        let model = self.shared.model.lock().expect("host model");
        let Some(client) = model.client_id.as_deref() else {
            return;
        };
        for view in model.views.values() {
            for block in view.windows.iter().flat_map(|w| w.blocks()) {
                if let Some(pane) = block.block_id.as_ref().and_then(|id| model.panes.get(id)) {
                    pane.model.lock().expect("pane model").owner =
                        block.owner.as_deref() == Some(client);
                }
            }
        }
    }

    fn remove_session(&mut self, id: &str) {
        self.workers.retain(|_, w| w.session_id != id);
        self.shared.update(|m| {
            m.sessions.retain(|s| s.session_id != id);
            if let Some(view) = m.views.remove(id) {
                for block in view.windows.iter().flat_map(|w| w.blocks()) {
                    if let Some(block) = &block.block_id {
                        m.forget_block(block);
                    }
                }
            }
            if m.selected.as_deref() == Some(id) {
                // tmux clears the register when destroying the attached session;
                // an implicit fallback is not a switch away from a live session.
                m.last_selected = None;
                m.selected = m.sessions.first().map(|s| s.session_id.clone());
            }
        });
    }

    async fn list_sessions(&mut self, control: &Control, floors: &mut Floors) -> Result<()> {
        let reply = control.call("session.list", json!({})).await?;
        floors.list = floors.list.max(reply.seq);
        let sessions: Vec<SessionSummary> =
            serde_json::from_value(reply.value["sessions"].clone()).unwrap_or_default();
        let gone: Vec<String> = self.shared.read(|m| {
            m.sessions
                .iter()
                .filter(|s| !sessions.iter().any(|n| n.session_id == s.session_id))
                .map(|s| s.session_id.clone())
                .collect()
        });
        self.shared.update(|m| m.sessions = sessions);
        for id in gone {
            self.remove_session(&id);
        }
        Ok(())
    }

    async fn refresh_hosts(&self, control: &Control) {
        if let Ok(reply) = control.value("hosts.list", json!({})).await {
            let hosts = reply["hosts"].as_array().cloned().unwrap_or_default();
            self.shared.update(|m| m.hosts = hosts);
        }
    }

    async fn fetch_keymap(&mut self, control: &Control, request: Value) {
        self.keymap_request = Some(request.clone());
        match control.value("client.keymap", request).await {
            Ok(keymap) => self.shared.update(|m| m.keymap = Some(keymap)),
            Err(error) => self
                .shared
                .notice(format!("Couldn't read key bindings: {error:#}")),
        }
    }

    async fn handle_event(
        &mut self,
        control: &Control,
        floors: &mut Floors,
        event: Event,
    ) -> Result<()> {
        let session = event.session_id.clone();
        match event.name.as_str() {
            "session_created" => {
                if let Ok(summary) =
                    serde_json::from_value::<SessionSummary>(event.payload["session"].clone())
                {
                    self.shared.update(|m| {
                        if m.session(&summary.session_id).is_none() {
                            m.sessions.push(summary);
                        }
                    });
                } else {
                    self.list_sessions(control, floors).await?;
                }
            }
            "session_moved" => self.list_sessions(control, floors).await?,
            "session_destroyed" => {
                if let Some(id) = event.payload["session_id"].as_str().or(session.as_deref()) {
                    self.remove_session(id);
                }
            }
            "session_label_changed" => {
                if let (Some(id), Some(label)) =
                    (session.as_deref(), event.payload["label"].as_str())
                {
                    self.shared.update(|m| {
                        if let Some(s) = m.sessions.iter_mut().find(|s| s.session_id == id) {
                            s.label = label.into();
                        }
                        if let Some(view) = m.views.get_mut(id) {
                            view.label = label.into();
                        }
                    });
                }
            }
            "session_view_changed" => {
                if let Some(id) = session.as_deref() {
                    let stale = self.shared.read(|m| {
                        m.views.get(id).map(|v| v.revision).is_some_and(|current| {
                            event.payload["revision"]
                                .as_u64()
                                .is_none_or(|new| new > current)
                        })
                    });
                    if stale {
                        self.refresh_view(control, floors, id).await?;
                    }
                }
            }
            "block_closed" => {
                if let Some(block) = event.payload["block_id"].as_str() {
                    if let Some(worker) = self.workers.remove(block) {
                        worker.pane.send(PaneCommand::Stop);
                    }
                    self.shared.update(|m| m.forget_block(block));
                }
            }
            "block_owner_changed" => {
                if let Some(block) = event.payload["block_id"].as_str() {
                    self.shared.update(|m| {
                        let owner = event.payload["owner"].as_str() == m.client_id.as_deref()
                            && m.client_id.is_some();
                        if let Some(pane) = m.panes.get(block) {
                            pane.model.lock().expect("pane model").owner = owner;
                        }
                    });
                }
            }
            "block_event"
                if event.payload["creator_name"] == super::onboarding::CREATOR
                    && event.payload["name"] == "step_changed" =>
            {
                if let Some(block) = event.payload["block_id"].as_str() {
                    let inner = &event.payload["payload"];
                    let state = (
                        inner["generation"].as_u64().unwrap_or(0),
                        inner["step"].as_str().unwrap_or_default().to_owned(),
                    );
                    self.shared.update(|m| {
                        let known = m.onboarding.entry(block.to_owned()).or_default();
                        if state.0 >= known.0 {
                            *known = state.clone();
                        }
                    });
                }
            }
            "block_event" => {
                if let (Some(id), Some((block, change))) = (
                    session.as_deref(),
                    activity::parse_block_event(&event.payload),
                ) {
                    self.shared
                        .update(|m| m.apply_block_event(id, block, change));
                }
            }
            "client_perform_requested" => {
                let p = &event.payload;
                let request = PerformRequest {
                    call_id: p["call_id"].as_str().unwrap_or_default().into(),
                    action: p["action"].as_str().unwrap_or_default().into(),
                    args: p["args"].clone(),
                    session_id: p["session_id"].as_str().map(str::to_owned),
                    block_id: p["block_id"].as_str().map(str::to_owned),
                    origin: p["origin"].as_str().unwrap_or("api").into(),
                };
                if !request.call_id.is_empty() {
                    self.shared.update(|m| m.performs.push(request));
                }
            }
            "client_prompt_requested" => {
                match serde_json::from_value::<prompt::Request>(event.payload.clone()) {
                    Ok(request) if self.shared.read(|m| m.prompt.is_none()) => {
                        self.shared.update(|m| m.prompt = Some(request));
                    }
                    _ => {
                        if let Some(call_id) = event.payload["call_id"].as_str() {
                            let _ = control.call("client.reply", json!({"call_id": call_id,
                                "error": {"code": "not_supported", "message": "Rexia can't show this prompt"}})).await;
                        }
                    }
                }
            }
            "client_prompt_ended" => {
                if let Some(id) = event.payload["call_id"].as_str() {
                    self.shared.update(|m| {
                        if m.prompt.as_ref().is_some_and(|p| p.call_id == id) {
                            m.prompt = None;
                        }
                    });
                }
            }
            "keymap_changed" => {
                let changed = self.shared.read(|m| {
                    m.keymap
                        .as_ref()
                        .is_none_or(|k| k["config"]["hash"] != event.payload["hash"])
                });
                if changed && let Some(request) = self.keymap_request.clone() {
                    self.fetch_keymap(control, request).await;
                }
            }
            "hosts_changed" => self.refresh_hosts(control).await,
            "tailscale_status_changed" => {
                // The payload is the new status, or holds it under `tailscale`.
                let status = if event.payload.get("state").is_some() {
                    event.payload.clone()
                } else {
                    event.payload["tailscale"].clone()
                };
                if status.is_object() {
                    self.shared.update(|m| {
                        if !m.server.is_object() {
                            m.server = json!({});
                        }
                        m.server["tailscale"] = status.clone();
                        m.tailscale_revision += 1;
                    });
                }
            }
            "client_call_requested" => {
                // This client doesn't connect Lua actions to other servers.
                if let Some(call_id) = event.payload["call_id"].as_str() {
                    let _ = control
                        .call(
                            "client.reply",
                            json!({"call_id": call_id, "error": {"code": "not_supported", "message": "rexia can't reach other servers for actions"}}),
                        )
                        .await;
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_command(
        &mut self,
        control: &Control,
        floors: &mut Floors,
        command: HostCommand,
    ) -> Result<()> {
        match command {
            HostCommand::Visible(visible) => self.visible = visible,
            HostCommand::LoadSession(id) => {
                if !floors.views.contains_key(&id)
                    && let Err(error) = self.attach(control, floors, &id).await
                    && client::error_code(&error) != Some("session_not_found")
                {
                    return Err(error);
                }
            }
            HostCommand::Select(id) => {
                // A selection queued before a destroy event must not resurrect
                // a removed session from an old cached view.
                if self.shared.read(|m| m.session(&id).is_none()) {
                    self.shared
                        .notice(format!("Session {id} is no longer available"));
                    return Ok(());
                }
                if !floors.views.contains_key(&id) {
                    self.attach(control, floors, &id).await?;
                }
                self.shared.update(|m| m.select(id));
            }
            HostCommand::CreateSession { label, reply } => {
                let layout = shell_layout_for(self.endpoint.is_local(), None);
                let mut payload = json!({"initial_windows": [{"layout": layout}]});
                if let Some(label) = label.filter(|l| !l.trim().is_empty()) {
                    payload["label"] = json!(label.trim());
                }
                let result = control.value("session.create", payload).await;
                match result {
                    Ok(created) => {
                        let id = created["session_id"]
                            .as_str()
                            .context("new session ID")?
                            .to_owned();
                        self.list_sessions(control, floors).await?;
                        self.attach(control, floors, &id).await?;
                        self.shared.update(|m| m.select(id));
                        if let Some(reply) = reply {
                            let _ = reply.send(Ok(created));
                        }
                    }
                    Err(error) => {
                        let message = format!("{error:#}");
                        if let Some(reply) = reply {
                            let _ = reply.send(Err(message));
                        } else {
                            return Err(error);
                        }
                    }
                }
            }
            HostCommand::Call {
                method,
                payload,
                reply,
            } if method == "client.action"
                && control.welcome.api_revision >= client::PROMPT_REVISION =>
            {
                self.start_promptable(control, &method, payload, reply)?;
            }
            HostCommand::Call {
                method,
                payload,
                reply,
            } => {
                let result = control.value(&method, payload).await;
                match (result, reply) {
                    (Ok(value), Some(reply)) => {
                        let _ = reply.send(Ok(value));
                    }
                    (Ok(_), None) => {}
                    (Err(error), Some(reply)) => {
                        if client::is_disconnect(&error) {
                            let _ = reply.send(Err(format!("{error:#}")));
                            return Err(error);
                        }
                        let _ = reply.send(Err(format!("{error:#}")));
                    }
                    (Err(error), None) => return Err(error),
                }
            }
            HostCommand::Input {
                session_id,
                block_id,
                method,
                payload,
                claim,
            } => {
                let owner = self.shared.read(|m| {
                    m.panes
                        .get(&block_id)
                        .is_some_and(|p| p.model.lock().expect("pane model").owner)
                });
                if claim && !owner {
                    self.claim(control, &session_id, &block_id).await;
                }
                if method == "client.key" && control.welcome.api_revision >= client::PROMPT_REVISION
                {
                    self.start_promptable(control, method, payload, None)?;
                } else {
                    control.call(method, payload).await?;
                }
            }
            HostCommand::Resize {
                session_id,
                block_id,
                cols,
                rows,
                cell_width,
                cell_height,
            } => {
                if let Some(worker) = self.workers.get_mut(&block_id) {
                    if worker.size == Some((cols, rows, cell_width, cell_height)) {
                        return Ok(());
                    }
                    worker.size = Some((cols, rows, cell_width, cell_height));
                }
                let owner = self.shared.read(|m| {
                    m.panes
                        .get(&block_id)
                        .is_some_and(|p| p.model.lock().expect("pane model").owner)
                });
                // Resizing takes the pane back, so it follows this client's size.
                if !owner {
                    self.claim(control, &session_id, &block_id).await;
                }
                resize(
                    control,
                    &session_id,
                    &block_id,
                    cols,
                    rows,
                    cell_width,
                    cell_height,
                )
                .await?;
            }
            HostCommand::Keymap(request) => self.fetch_keymap(control, request).await,
            HostCommand::PerformReply { call_id, result } => {
                self.shared
                    .update(|m| m.performs.retain(|p| p.call_id != call_id));
                let payload = match result {
                    Ok(value) => json!({"call_id": call_id, "result": value}),
                    Err((code, message)) => {
                        json!({"call_id": call_id, "error": {"code": code, "message": message}})
                    }
                };
                control.call("client.reply", payload).await?;
            }
            HostCommand::PromptReply { call_id, result } => {
                if !self
                    .shared
                    .read(|m| m.prompt.as_ref().is_some_and(|p| p.call_id == call_id))
                {
                    return Ok(());
                }
                let reply = control
                    .call(
                        "client.reply",
                        json!({"call_id": call_id, "result": result}),
                    )
                    .await;
                if reply.is_ok()
                    || reply.as_ref().err().and_then(client::error_code) == Some("not_found")
                {
                    self.shared.update(|m| m.prompt = None);
                } else {
                    reply?;
                }
            }
            HostCommand::SetLabel(label) => {
                self.identity.label = label.clone();
                control
                    .call(
                        "client.set_label",
                        json!({"label": label.unwrap_or_default()}),
                    )
                    .await?;
            }
            HostCommand::TerminalTheme(theme) => self.set_theme(theme),
            HostCommand::DefaultCursor(shape) => self.set_cursor(shape),
            HostCommand::Pin { session, window } => {
                self.shared.update(|m| m.pin(session, window));
                self.reconcile();
            }
            HostCommand::Reconnect | HostCommand::Shutdown => unreachable!("handled by the loop"),
        }
        Ok(())
    }

    async fn claim(&self, control: &Control, session_id: &str, block_id: &str) {
        let Ok(result) = control
            .value(
                "session.own_block",
                json!({"session_id": session_id, "block_ids": [block_id], "mode": "take"}),
            )
            .await
        else {
            return;
        };
        self.shared.update(|m| {
            let owner = result["blocks"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|b| b["block_id"] == block_id)
                .is_some_and(|b| b["owner"].as_str() == m.client_id.as_deref());
            if let Some(pane) = m.panes.get(block_id) {
                pane.model.lock().expect("pane model").owner = owner;
            }
        });
    }

    fn set_theme(&mut self, theme: Option<TerminalTheme>) {
        self.theme = theme.clone();
        for worker in self.workers.values() {
            worker.pane.send(PaneCommand::Theme(theme.clone()));
        }
    }

    fn set_cursor(&mut self, shape: Option<CursorShape>) {
        self.cursor = shape;
        for worker in self.workers.values() {
            worker.pane.send(PaneCommand::DefaultCursor(shape));
        }
    }

    /// Prepare the selected session's tabs before switching to them. Hidden
    /// terminals publish at a lower cadence than visible ones.
    fn reconcile(&mut self) {
        let (visible, warm, known) = self.shared.read(|m| {
            let mut known = HashMap::new();
            for (id, view) in &m.views {
                for block in view.terminal_blocks() {
                    if let Some(block) = &block.block_id {
                        known.insert(block.clone(), id.clone());
                    }
                }
            }
            let visible = if self.visible {
                m.view()
                    .and_then(View::active_window)
                    .map(|w| {
                        w.panes()
                            .into_iter()
                            .map(|p| p.block_id)
                            .filter(|id| known.contains_key(id))
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                HashSet::new()
            };
            // Pre-warm a few neighboring tabs, without opening an unbounded number
            // of data connections in a session with many tabs.
            let mut warm = visible.clone();
            if self.visible
                && let Some(view) = m.view()
            {
                let shown = view.active_window().and_then(|active| {
                    view.windows
                        .iter()
                        .position(|window| window.id() == active.id())
                });
                if let Some(shown) = shown {
                    for offset in [-1_isize, 1, -2, 2] {
                        if let Some(index) = shown.checked_add_signed(offset)
                            && let Some(window) = view.windows.get(index)
                        {
                            warm.extend(
                                window
                                    .panes()
                                    .into_iter()
                                    .map(|pane| pane.block_id)
                                    .filter(|block| known.contains_key(block)),
                            );
                        }
                    }
                }
            }
            (visible, warm, known)
        });
        let stale: Vec<String> = self
            .workers
            .keys()
            .filter(|id| !known.contains_key(*id))
            .cloned()
            .collect();
        for id in stale {
            self.workers.remove(&id);
            self.shared.update(|m| {
                m.panes.remove(&id);
            });
        }
        for block in &warm {
            if self.workers.contains_key(block) {
                continue;
            }
            let session_id = known[block].clone();
            let (commands, receiver) = flume::unbounded();
            let pane = PaneHandle {
                model: Arc::default(),
                commands,
            };
            if let Some(theme) = &self.theme {
                pane.send(PaneCommand::Theme(Some(theme.clone())));
            }
            if self.cursor.is_some() {
                pane.send(PaneCommand::DefaultCursor(self.cursor));
            }
            pane.send(PaneCommand::Visible(visible.contains(block)));
            self.shared.update(|m| {
                m.panes.insert(block.clone(), pane.clone());
            });
            let task = tokio::task::spawn_local(stream(
                self.endpoint.clone(),
                self.link.clone(),
                session_id.clone(),
                block.clone(),
                pane.clone(),
                receiver,
                self.shared.wake.clone(),
            ));
            self.workers.insert(
                block.clone(),
                Worker {
                    session_id,
                    pane,
                    size: None,
                    visible: visible.contains(block),
                    task,
                },
            );
        }
        self.update_owners();
        for (id, worker) in &mut self.workers {
            let shown = visible.contains(id);
            if worker.visible != shown {
                worker.pane.send(PaneCommand::Visible(shown));
                worker.visible = shown;
            }
        }
    }

    /// Read what programs last reported in each attached session's terminals, once per
    /// connection. Live changes arrive as events afterwards.
    async fn query_activity(&mut self, control: &Control, floors: &mut Floors) {
        let pending: Vec<(String, String)> = self.shared.read(|m| {
            m.views
                .iter()
                .flat_map(|(session, view)| {
                    view.terminal_blocks()
                        .filter_map(|b| b.block_id.clone())
                        .map(move |block| (session.clone(), block))
                })
                .filter(|(_, block)| !floors.queried.contains(block))
                .collect()
        });
        for (session, block) in pending {
            floors.queried.insert(block.clone());
            let target = json!({"session_id": session, "block_id": block, "args": {}});
            let [status, title, process, pwd] =
                ["program_status", "title", "process", "pwd"].map(|method| {
                    let method = format!("{TERMINAL_CREATOR}.{method}");
                    let target = target.clone();
                    let control = control.clone();
                    async move { control.value(&method, target).await }
                });
            let (status, title, process, pwd) = tokio::join!(status, title, process, pwd);
            self.shared.update(|m| {
                let state = m.blocks.entry(block.clone()).or_default();
                state.session_id = session.clone();
                if let Ok(reply) = status {
                    state.records = activity::parse_status(&reply);
                }
                if let Ok(reply) = title {
                    let text = reply["title"].as_str().unwrap_or_default();
                    state.title = (!text.is_empty())
                        .then(|| (text.to_owned(), reply["owner"]["pid"].as_u64()));
                }
                if let Ok(reply) = process {
                    state.process = Process::parse(&reply);
                    if let Some(code) = reply["last_exit"]["exit_code"].as_i64() {
                        state.exited = Some(code);
                    }
                }
                if let Ok(reply) = pwd
                    && let Some(path) = reply["pwd"].as_str().filter(|p| !p.is_empty())
                {
                    state.pwd = Some(activity::pwd_path(path));
                }
            });
        }
    }
}

/// One new shell block, the server user's login shell.
pub fn shell_layout() -> Value {
    json!({"block": {"flavor": data::SHELL_FLAVOR}})
}

/// A new shell that starts in `cwd`.
pub fn shell_layout_in(cwd: &str) -> Value {
    json!({"block": {"flavor": data::SHELL_FLAVOR, "options": {"cwd": cwd}}})
}

/// The command new local terminals run instead of the login shell, chosen in Settings.
static STARTUP: std::sync::RwLock<Option<Vec<String>>> = std::sync::RwLock::new(None);

/// Run `command` (a program and its arguments) in new local terminals, or the default
/// login shell with `None`.
pub fn set_startup_command(command: Option<Vec<String>>) {
    *STARTUP.write().expect("startup command") = command.filter(|c| !c.is_empty());
}

/// The terminal options for the startup command: a program alone starts as the shell,
/// so a shell such as fish reads its startup files as a login shell; a program with
/// arguments runs directly.
pub fn startup_options(command: &[String]) -> Value {
    match command {
        [] => json!({}),
        [program] => json!({"shell": program}),
        [program, args @ ..] => {
            let mut all = vec![program.clone()];
            all.extend(args.iter().cloned());
            json!({"shell": "none", "command": all})
        }
    }
}

/// A new shell for a host: on a local host, the startup command chosen in Settings.
pub fn shell_layout_for(local: bool, cwd: Option<&str>) -> Value {
    let mut options = match STARTUP.read().expect("startup command").as_deref() {
        Some(command) if local => startup_options(command),
        _ => json!({}),
    };
    if let Some(cwd) = cwd {
        options["cwd"] = json!(cwd);
    }
    if options.as_object().is_some_and(|o| o.is_empty()) {
        return shell_layout();
    }
    json!({"block": {"flavor": data::SHELL_FLAVOR, "options": options}})
}

/// A startup command as typed in Settings, split into a program and its arguments.
/// Quotes group words, as a shell's do; a backslash escapes the next character.
pub fn split_command(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '\'' | '"') => {
                quote = Some(c);
                started = true;
            }
            (Some('"') | None, '\\') => {
                if let Some(next) = chars.next() {
                    word.push(next);
                    started = true;
                }
            }
            (None, c) if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            (_, c) => {
                word.push(c);
                started = true;
            }
        }
    }
    if started {
        words.push(word);
    }
    words
}

async fn resize(
    control: &Control,
    session: &str,
    block: &str,
    cols: u16,
    rows: u16,
    cell_width: u16,
    cell_height: u16,
) -> Result<()> {
    control
        .call(
            &format!("{TERMINAL_CREATOR}.resize"),
            json!({"session_id": session, "block_id": block,
                "args": {"columns": cols, "rows": rows, "cell_width_px": cell_width, "cell_height_px": cell_height}}),
        )
        .await?;
    Ok(())
}

async fn ticket(link: &SharedLink, session: &str, block: &str) -> Result<(String, bool)> {
    let control = {
        let mut link = link.lock().expect("link");
        if let Some(ticket) = link.tickets.remove(block)
            && Instant::now() < ticket.expires
        {
            return Ok((ticket.token, true));
        }
        link.control.clone().context("not connected")?
    };
    let reply = control
        .value(
            "data.connect",
            json!({"session_id": session, "block_id": block, "data_ref_id": "terminal", "subprotocol": PROTOCOL}),
        )
        .await?;
    let token = reply["ticket"]
        .as_str()
        .context("Rex sent no data ticket")?;
    Ok((token.to_owned(), false))
}

enum StreamEnd {
    Stop,
    Retry,
}

/// Stream one terminal: open with a ticket, resume after drops at the same byte offset,
/// and publish frames at most once per display frame while visible.
async fn stream(
    endpoint: Endpoint,
    link: SharedLink,
    session: String,
    block: String,
    pane: PaneHandle,
    commands: flume::Receiver<PaneCommand>,
    wake: flume::Sender<()>,
) {
    let publish = |change: &dyn Fn(&mut PaneModel)| {
        let mut model = pane.model.lock().expect("pane model");
        change(&mut model);
        model.revision += 1;
        drop(model);
        let _ = wake.try_send(());
    };
    let mut state: Option<TerminalState> = None;
    let mut welcome: Option<data::Welcome> = None;
    let mut visible = true;
    let mut theme: Option<TerminalTheme> = None;
    let mut cursor: Option<CursorShape> = None;
    let mut failures = 0u32;
    loop {
        let attempt: Result<StreamEnd> = async {
            let mut socket = None;
            if let (Some(previous), Some(current)) = (&welcome, &state) {
                let mut next = endpoint::connect(&endpoint, "/api/data", PROTOCOL).await?;
                if let Ok(resumed) = data::resume(&mut next, &previous.token, current.received).await {
                    anyhow::ensure!(resumed.nonce == previous.nonce, "Rex changed the stream during resume");
                    welcome = Some(resumed);
                    socket = Some(next);
                }
            }
            if socket.is_none() {
                let (next, fresh) = loop {
                    let (token, bundled) = ticket(&link, &session, &block).await?;
                    let mut next = endpoint::connect(&endpoint, "/api/data", PROTOCOL).await?;
                    match data::handshake(&mut next, &token).await {
                        Ok(fresh) => break (next, fresh),
                        // A bundled ticket may have been used up; fetch a fresh one.
                        Err(error) if bundled && error.is::<data::Rejected>() => continue,
                        Err(error) => return Err(error),
                    }
                };
                let mut fresh_state = TerminalState::new(fresh.nonce)?;
                fresh_state.set_local(endpoint.is_local())?;
                fresh_state.set_local_theme(theme.clone())?;
                fresh_state.set_default_cursor(cursor)?;
                state = Some(fresh_state);
                welcome = Some(fresh);
                publish(&|m| {
                    m.copy_mode = false;
                    m.status = PaneStatus::Restoring;
                });
                socket = Some(next);
            }
            let mut socket = socket.expect("data socket");
            let terminal = state.as_mut().expect("terminal state");
            failures = 0;
            let mut dirty = true;
            let mut tick = tokio::time::interval(Duration::from_millis(16));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut background_tick = tokio::time::interval(Duration::from_millis(150));
            background_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                // A playing Kitty animation draws its next frame when it's due.
                let animation = terminal.animation_wait();
                tokio::select! {
                    _ = tokio::time::sleep(animation.unwrap_or_default()), if animation.is_some() && visible => {
                        dirty = true;
                    }
                    bytes = data::next_binary(&mut socket) => {
                        terminal.feed(&bytes?)?;
                        dirty = true;
                        for event in terminal.take_events() {
                            match event {
                                TermEvent::Clipboard(text) => publish(&|m| m.clipboard = Some(text.clone())),
                                TermEvent::Bell => publish(&|m| m.bells += 1),
                                TermEvent::ChildExited(code) => publish(&|m| m.exit_code = Some(code)),
                                _ => {}
                            }
                        }
                    }
                    command = commands.recv_async() => {
                        let Ok(command) = command else { return Ok(StreamEnd::Stop) };
                        dirty = true;
                        match command {
                            PaneCommand::Visible(value) => visible = value,
                            PaneCommand::Scroll(scroll) => terminal.scroll(match scroll {
                                Scroll::Lines(n) => ghostty_vt::terminal::ScrollViewport::Delta(n),
                                Scroll::Top => ghostty_vt::terminal::ScrollViewport::Top,
                                Scroll::Bottom => ghostty_vt::terminal::ScrollViewport::Bottom,
                                Scroll::Row(row) => ghostty_vt::terminal::ScrollViewport::Row(row),
                            }),
                            PaneCommand::Select { x, y, mode } => terminal.select_start(x, y, mode)?,
                            PaneCommand::SelectTo { x, y } => terminal.select_to(x, y)?,
                            PaneCommand::SelectAll => terminal.select_all()?,
                            PaneCommand::ClearSelection => terminal.clear_selection()?,
                            PaneCommand::Copy(reply) => { let _ = reply.send(terminal.selection_text()?); }
                            PaneCommand::CopyMode(command) => terminal.copy_mode(command)?,
                            PaneCommand::YankCopyMode { to_line_end, reply } => {
                                let _ = reply.send(terminal.copy_mode_yank(to_line_end)?);
                            }
                            PaneCommand::Text(reply) => { let _ = reply.send(terminal.text().ok()); }
                            PaneCommand::Search(needle) => terminal.search(needle.as_deref())?,
                            PaneCommand::SearchStep { older } => terminal.search_step(older)?,
                            PaneCommand::Prompt(direction) => { terminal.jump_to_prompt(direction)?; }
                            PaneCommand::Reflow { cols, rows, cell_width, cell_height } => terminal.reflow(cols, rows, cell_width, cell_height)?,
                            PaneCommand::Theme(next) => { theme = next.clone(); terminal.set_local_theme(next)?; }
                            PaneCommand::DefaultCursor(next) => { cursor = next; terminal.set_default_cursor(next)?; }
                            PaneCommand::Reconnect => return Ok(StreamEnd::Retry),
                            PaneCommand::Stop => return Ok(StreamEnd::Stop),
                        }
                    }
                    _ = async {
                        if visible { tick.tick().await } else { background_tick.tick().await }
                    }, if dirty => {
                        dirty = false;
                        if let Some(frame) = terminal.frame()? {
                            let frame = Arc::new(frame);
                            let exit = terminal.exit_code;
                            publish(&|m| {
                                m.frame = Some(frame.clone());
                                m.status = PaneStatus::Live;
                                if exit.is_some() { m.exit_code = exit; }
                            });
                        }
                    }
                }
            }
        }
        .await;
        match attempt {
            Ok(StreamEnd::Stop) => return,
            Ok(StreamEnd::Retry) => {}
            Err(error) => {
                if error
                    .downcast_ref::<data::Closed>()
                    .is_some_and(data::Closed::is_final)
                {
                    publish(&|m| {
                        m.copy_mode = false;
                        m.status = PaneStatus::Closed;
                    });
                    return;
                }
                // A failed resume or corrupt stream starts over from a fresh snapshot.
                if error.downcast_ref::<data::Closed>().is_none() {
                    welcome = None;
                }
                failures += 1;
                let message = format!("{error:#}");
                publish(&|m| m.status = PaneStatus::Reconnecting(message.clone()));
                tokio::time::sleep(Duration::from_millis(250 * u64::from(failures.min(16)))).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_session_is_one_register_per_client_not_focus_history() {
        let mut client = HostModel::default();
        let mut other_client = HostModel::default();
        client.select("A".into());
        assert_eq!(client.last_selected, None); // initial attach
        client.select("B".into());
        assert_eq!(client.last_selected.as_deref(), Some("A"));
        client.select("A".into()); // l
        client.select("B".into()); // l
        client.select("C".into()); // picker selecting an existing session
        assert_eq!(client.last_selected.as_deref(), Some("B"));
        client.select("C".into()); // same session does not clobber last
        assert_eq!(client.last_selected.as_deref(), Some("B"));
        client.select("B".into()); // l
        assert_eq!(client.last_selected.as_deref(), Some("C"));
        assert_eq!(other_client.last_selected, None);
        other_client.select("A".into());
        assert_eq!(other_client.last_selected, None);
    }

    #[tokio::test]
    async fn new_shell_directory_prefers_live_process_and_handles_missing_data() {
        let (commands, receiver) = flume::unbounded();
        let (_, wake) = flume::bounded(1);
        let host = HostHandle {
            endpoint: Endpoint::parse("ssh://example.test").unwrap(),
            model: Arc::default(),
            commands,
            wake,
        };
        for (response, reported, cached, expected) in [
            (
                Ok(json!({"foreground": {"cwd": "/live path"}})),
                Some("/reported"),
                Some("/cached"),
                Some("/live path"),
            ),
            (
                Ok(json!({})),
                Some("/reported"),
                Some("/cached"),
                Some("/reported"),
            ),
            (
                Err("not available".to_owned()),
                None,
                Some("/cached"),
                Some("/cached"),
            ),
            (
                Ok(json!({"foreground": {"cwd": ""}})),
                Some(""),
                Some(""),
                None,
            ),
        ] {
            host.model.lock().unwrap().blocks.insert(
                "source".into(),
                BlockState {
                    pwd: reported.map(str::to_owned),
                    process: Process {
                        cwd: cached.map(str::to_owned),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            );
            let answer = async {
                let HostCommand::Call {
                    method,
                    payload,
                    reply: Some(reply),
                } = receiver.recv_async().await.unwrap()
                else {
                    panic!("expected directory request")
                };
                assert_eq!(method, format!("{TERMINAL_CREATOR}.process"));
                assert_eq!(payload["session_id"], "session");
                assert_eq!(payload["block_id"], "source");
                reply.send(response).unwrap();
            };
            let (layout, ()) =
                tokio::join!(host.shell_layout_from("session", Some("source")), answer);
            assert_eq!(layout["block"]["options"]["cwd"].as_str(), expected);
        }
        assert_eq!(
            host.shell_layout_from("session", None).await,
            shell_layout()
        );
        assert!(
            receiver.try_recv().is_err(),
            "no source needs no process query"
        );
    }

    #[test]
    fn startup_commands_split_and_become_terminal_options() {
        assert_eq!(split_command("fish"), ["fish"]);
        assert_eq!(
            split_command(r#"nu --config "/home/me/my config.nu" -e 'ls'"#),
            ["nu", "--config", "/home/me/my config.nu", "-e", "ls"]
        );
        assert_eq!(split_command(r"a\ b ''"), ["a b", ""]);
        assert!(split_command("   ").is_empty());
        assert_eq!(startup_options(&["fish".into()]), json!({"shell": "fish"}));
        assert_eq!(
            startup_options(&["htop".into(), "-d".into(), "5".into()]),
            json!({"shell": "none", "command": ["htop", "-d", "5"]})
        );
        // Only local hosts get it; a remote host's new terminals keep their login shell.
        set_startup_command(Some(vec!["fish".into()]));
        assert_eq!(
            shell_layout_for(false, Some("/tmp")),
            shell_layout_in("/tmp")
        );
        assert_eq!(
            shell_layout_for(true, Some("/tmp"))["block"]["options"],
            json!({"shell": "fish", "cwd": "/tmp"})
        );
        set_startup_command(None);
        assert_eq!(shell_layout_for(true, None), shell_layout());
    }

    #[test]
    fn floors_skip_events_already_in_snapshots() {
        let mut floors = Floors {
            bootstrap: 10,
            list: 12,
            ..Default::default()
        };
        floors.views.insert("s".into(), 15);
        let event = |seq, name: &str, session: Option<&str>| Event {
            seq,
            session_id: session.map(str::to_owned),
            name: name.into(),
            payload: Value::Null,
        };
        assert!(!floors.admits(&event(10, "hosts_changed", None)));
        assert!(floors.admits(&event(11, "hosts_changed", None)));
        assert!(!floors.admits(&event(12, "session_created", None)));
        assert!(!floors.admits(&event(14, "block_event", Some("s"))));
        assert!(floors.admits(&event(16, "block_event", Some("s"))));
        assert!(floors.admits(&event(13, "session_destroyed", Some("s"))));
    }

    #[test]
    fn titles_expire_with_the_program_that_set_them() {
        let mut state = BlockState {
            title: Some(("vim notes".into(), Some(42))),
            process: Process {
                pids: vec![42, 7],
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(state.current_title(), Some("vim notes"));
        state.process.pids = vec![7];
        assert_eq!(state.current_title(), None);
    }
}

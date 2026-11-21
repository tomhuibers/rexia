//! Palettes, dialogs and the find bar, drawn over the workspace like Noctalia's panels.
use super::{Target, Workspace};
use crate::{
    keymap,
    rex::{
        activity::ProgramState,
        data::TERMINAL_CREATOR,
        endpoint::Endpoint,
        host::{HostCommand, shell_layout_for},
        view::View,
    },
    ui::{
        actions,
        components::{dot, icon_button},
        fuzzy,
        input::{InputEvent, TextInput},
        theme::Theme,
        titles,
    },
};
use gpui::{
    AnyElement, Context, Entity, FontWeight, ScrollHandle, SharedString, Subscription, Window, div,
    prelude::*, px,
};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKind {
    Commands,
    Sessions,
    GoTo,
    FloatingPaneActions,
    Activity,
    MoveToTab,
    Hosts,
    RemoveHost,
    Forwards,
    /// Choose an action to give a new key.
    Shortcuts,
    /// Choose the effect cursor shaders draw.
    CursorShaders,
    /// Browse directories: open a tab there, or (`insert`) type the path into the pane.
    Directory {
        insert: bool,
    },
}

impl ListKind {
    fn placeholder(self) -> &'static str {
        match self {
            Self::Commands => "Run a command",
            Self::Sessions => "Go to a session, or name a new one",
            Self::GoTo => "Sessions, tabs and panes",
            Self::FloatingPaneActions => "Choose a floating pane",
            Self::Activity => "Programs that reported a status",
            Self::MoveToTab => "Move the pane to a tab",
            Self::Hosts => "Switch to a host",
            Self::RemoveHost => "Remove a host",
            Self::Forwards => "Port, host:port, or URL",
            Self::Shortcuts => "Choose an action to give a new key",
            Self::CursorShaders => "Choose a cursor shader",
            Self::Directory { .. } => "Path",
        }
    }

    /// The action whose key also closes the list.
    pub fn action(self) -> &'static str {
        match self {
            Self::Commands => "client.palette.toggle",
            Self::Sessions => "session.switch",
            Self::GoTo => "pane.go_to",
            Self::FloatingPaneActions => "pane.floating_actions",
            Self::Activity => "pane.view_activity",
            Self::MoveToTab => "pane.move_to_tab",
            Self::Hosts => "client.host.switch",
            Self::RemoveHost => "client.host.remove",
            Self::Forwards => "client.host.forward_port",
            Self::Shortcuts => "client.shortcuts.edit",
            Self::CursorShaders => "client.cursor_shader.change",
            Self::Directory { insert: false } => "pane.go_to_directory",
            Self::Directory { insert: true } => "pane.insert_path",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Choice {
    Action {
        name: String,
        args: Value,
    },
    Session {
        host: usize,
        id: String,
    },
    CreateSession {
        label: String,
    },
    Window {
        host: usize,
        session: String,
        window: String,
    },
    Block {
        host: usize,
        session: String,
        block: String,
    },
    FloatingPaneActions {
        host: usize,
        session: String,
        block: String,
    },
    MoveToWindow {
        block: String,
        anchor: String,
    },
    Host(usize),
    CursorShader(crate::settings::CursorShader),
    RemoveHost {
        id: String,
        label: String,
    },
    Shortcut {
        name: String,
        title: String,
    },
    Path {
        path: String,
        directory: bool,
    },
    /// A host address typed in the command palette: open Add Host with it.
    AddTypedHost {
        label: String,
        address: String,
    },
    Setting(String),
    OpenForward(u64),
}

#[derive(Clone, Debug)]
pub struct Item {
    pub title: String,
    pub detail: Option<String>,
    pub keys: Option<String>,
    pub icon: &'static str,
    pub status: Option<(ProgramState, &'static str)>,
    pub current: bool,
    pub choice: Choice,
}

pub struct ListOverlay {
    pub kind: ListKind,
    pub input: Entity<TextInput>,
    pub(super) items: Vec<Item>,
    pub(super) shown: Vec<usize>,
    selected: usize,
    scroll: ScrollHandle,
    /// Bumped with each directory listing request, so a slow reply can't replace a newer one.
    generation: u64,
    forward_host: Option<String>,
    _subscription: Subscription,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RenameTarget {
    Session(String),
    Window(String),
    Block(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Confirm {
    /// Reset a shortcut, taking its default key back from the action that has it now.
    ResetShortcut {
        action: String,
        from: String,
    },
    /// Close a pane or tab with busy programs in it.
    Close(super::close_guard::CloseTarget),
    /// Paste text that could run a command before it's read.
    Paste {
        block: String,
        text: String,
    },
    CloseSession {
        id: String,
    },
    RemoveHost {
        id: String,
    },
    RestartServer,
    OpenFile(String),
}

impl Confirm {
    /// The button that leaves things as they are.
    fn keep_label(&self) -> &'static str {
        match self {
            Self::Paste { .. }
            | Self::Close(_)
            | Self::ResetShortcut { .. }
            | Self::OpenFile(_) => "Cancel",
            Self::RemoveHost { .. } => "Keep Host",
            Self::CloseSession { .. } | Self::RestartServer => "Keep Running",
        }
    }
}

pub enum Dialog {
    Rename {
        target: RenameTarget,
        input: Entity<TextInput>,
        _subscription: Subscription,
    },
    /// Open a link that launches another app? Buttons: Cancel, Always Open, Open.
    OpenLink {
        link: String,
        focused: usize,
    },
    ForwardPort {
        request: super::forwards::ForwardRequest,
        error: Option<String>,
        authorize: bool,
        focused: usize,
    },
    Confirm {
        title: String,
        body: String,
        confirm: String,
        action: Confirm,
        /// Which button Enter presses: 0 keeps things as they are, 1 confirms.
        focused: usize,
        /// Busy programs in what's closing; clicking one goes to its pane.
        programs: Vec<super::close_guard::Busy>,
    },
    AddHost {
        label: Entity<TextInput>,
        address: Entity<TextInput>,
        error: Option<String>,
        _subscriptions: [Subscription; 2],
    },
    Diagnostics {
        text: String,
    },
    /// Change Shortcut, waiting for the next key.
    Shortcut {
        action: String,
        title: String,
        previous: Option<Box<ListOverlay>>,
    },
}

pub enum Overlay {
    List(ListOverlay),
    Dialog(Dialog),
    Settings(Box<super::preferences::SettingsPage>),
    ScriptPrompt(Box<super::script_prompt::ScriptPrompt>),
}

fn forward_port_title(port: u16) -> String {
    format!("Local port {port} requires administrator approval")
}

pub struct FindBar {
    pub input: Entity<TextInput>,
    pub block: String,
    _subscription: Subscription,
}

fn command_items(ws: &Workspace) -> Vec<Item> {
    let mut items: Vec<Item> = actions::ACTIONS
        .iter()
        // Actions that need arguments run from keys and other clients, not from a list.
        .filter(|a| {
            a.mode.is_none() && a.args.is_none_or(|schema| !schema.contains("\"required\""))
        })
        .map(|a| Item {
            title: a.title.to_owned(),
            detail: Some(a.category.to_owned()),
            keys: ws.keymap.keys_for(a.name, None).map(keymap::display),
            icon: "command",
            status: None,
            current: false,
            choice: Choice::Action {
                name: a.name.to_owned(),
                args: Value::Null,
            },
        })
        .collect();
    items.extend(crate::settings_index::entries().into_iter().map(|entry| {
        Item {
            title: entry.title.clone(),
            detail: Some(format!(
                "Settings › {} · {}",
                entry.section,
                crate::settings_index::display(&crate::settings_index::value(
                    &ws.settings,
                    &entry.path
                ))
            )),
            keys: actions::setting_action(&entry.path)
                .and_then(|action| ws.keymap.keys_for(action, None))
                .map(keymap::display),
            icon: "settings",
            status: None,
            current: false,
            choice: Choice::Setting(entry.path),
        }
    }));
    for action in &ws.keymap.actions {
        // Block methods without a title are for bindings, not menus.
        let Some(title) = action.title.clone() else {
            continue;
        };
        items.push(Item {
            title,
            detail: Some(action.category.clone().unwrap_or_else(|| "Actions".into())),
            keys: ws.keymap.keys_for(&action.name, None).map(keymap::display),
            icon: if action.kind == "lua" {
                "file-code"
            } else {
                "terminal-2"
            },
            status: None,
            current: false,
            choice: Choice::Action {
                name: action.name.clone(),
                args: Value::Null,
            },
        });
    }
    items
}

fn session_items(ws: &Workspace) -> Vec<Item> {
    let mut items = Vec::new();
    for (index, host) in ws.hosts.iter().enumerate() {
        let model = host.handle.snapshot();
        for session in &model.sessions {
            let status = ws.session_mark(index, &session.session_id, &model);
            items.push(Item {
                title: session.label.clone(),
                detail: Some(host.config.label.clone()),
                keys: None,
                icon: "layout-columns",
                status,
                current: index == ws.active && model.selected.as_ref() == Some(&session.session_id),
                choice: Choice::Session {
                    host: index,
                    id: session.session_id.clone(),
                },
            });
        }
    }
    items
}

fn go_to_items(ws: &Workspace) -> Vec<Item> {
    let mut items = session_items(ws);
    for (index, host) in ws.hosts.iter().enumerate() {
        let model = host.handle.snapshot();
        for (session, view) in &model.views {
            let session_label = view.label.clone();
            for window in &view.windows {
                items.push(Item {
                    title: titles::tab_title(window, &model),
                    detail: Some(format!("Tab in {session_label}")),
                    keys: None,
                    icon: "layout-sidebar",
                    status: ws.tab_mark(index, window, &model),
                    current: false,
                    choice: Choice::Window {
                        host: index,
                        session: session.clone(),
                        window: window.id().to_owned(),
                    },
                });
                for block in window.blocks() {
                    let Some(id) = block.block_id.clone() else {
                        continue;
                    };
                    let state = model.blocks.get(&id);
                    let pwd = state.and_then(|s| s.pwd.clone().or(s.process.cwd.clone()));
                    items.push(Item {
                        title: titles::pane_title(block, state),
                        detail: Some(match pwd {
                            Some(pwd) => format!("{pwd} · {session_label}"),
                            None => format!("Pane in {session_label}"),
                        }),
                        keys: None,
                        icon: titles::program_icon(state),
                        status: state
                            .and_then(|s| s.status())
                            .filter(|r| r.state != ProgramState::Idle)
                            .map(|r| (r.state, r.badge())),
                        current: window.focused_block_id.as_ref() == Some(&id),
                        choice: Choice::Block {
                            host: index,
                            session: session.clone(),
                            block: id,
                        },
                    });
                }
            }
        }
    }
    items
}

fn floating_pane_items(ws: &Workspace) -> Vec<Item> {
    let model = ws.model();
    let (Some(session), Some(tab)) = (
        model.selected.as_ref(),
        model.view().and_then(View::active_window),
    ) else {
        return Vec::new();
    };
    tab.panes()
        .into_iter()
        .filter(|pane| pane.floating.is_some())
        .filter_map(|pane| {
            let block = tab.block(&pane.block_id)?;
            Some((pane.block_id, block))
        })
        .enumerate()
        .map(|(index, (id, block))| {
            let state = model.blocks.get(&id);
            Item {
                title: titles::pane_title(block, state),
                detail: Some(format!("Pane {}", index + 1)),
                keys: None,
                icon: titles::program_icon(state),
                status: None,
                current: tab.focused_block_id.as_ref() == Some(&id),
                choice: Choice::FloatingPaneActions {
                    host: ws.active,
                    session: session.clone(),
                    block: id,
                },
            }
        })
        .collect()
}

/// The new quick switcher: every program reporting a status (each of a pane's reporting
/// programs on its own row, most urgent first), then every session, then every pane.
fn quick_switcher_items(ws: &Workspace) -> Vec<Item> {
    let mut processes: Vec<(ProgramState, Item)> = Vec::new();
    let mut panes = Vec::new();
    for (index, host) in ws.hosts.iter().enumerate() {
        let model = host.handle.snapshot();
        for (block, state) in &model.blocks {
            let session = model
                .session(&state.session_id)
                .map(|s| s.label.clone())
                .unwrap_or_default();
            let view_block = model
                .views
                .get(&state.session_id)
                .and_then(|v| v.windows.iter().find_map(|w| w.block(block)).cloned())
                .unwrap_or_default();
            let pane = titles::pane_title(&view_block, Some(state));
            let choice = Choice::Block {
                host: index,
                session: state.session_id.clone(),
                block: block.clone(),
            };
            for record in state.records.values() {
                processes.push((
                    record.state,
                    Item {
                        title: record
                            .title
                            .clone()
                            .or_else(|| record.app.clone())
                            .unwrap_or_else(|| pane.clone()),
                        detail: Some(
                            [record.msg.as_deref(), Some(&pane), Some(&session)]
                                .into_iter()
                                .flatten()
                                .collect::<Vec<_>>()
                                .join(" · "),
                        ),
                        keys: None,
                        icon: titles::program_icon(Some(state)),
                        status: Some((record.state, record.badge())),
                        current: false,
                        choice: choice.clone(),
                    },
                ));
            }
            let pwd = state.pwd.clone().or(state.process.cwd.clone());
            panes.push(Item {
                title: pane,
                detail: Some(match pwd {
                    Some(pwd) => format!("{pwd} · {session}"),
                    None => format!("Pane in {session}"),
                }),
                keys: None,
                icon: titles::program_icon(Some(state)),
                status: None,
                current: false,
                choice,
            });
        }
    }
    processes.sort_by_key(|p| std::cmp::Reverse(p.0));
    let mut items: Vec<Item> = processes.into_iter().map(|(_, item)| item).collect();
    items.extend(session_items(ws));
    items.extend(panes);
    items
}

fn activity_items(ws: &Workspace) -> Vec<Item> {
    let mut items: Vec<(ProgramState, Item)> = Vec::new();
    for (index, host) in ws.hosts.iter().enumerate() {
        let model = host.handle.snapshot();
        for (block, state) in &model.blocks {
            let Some(record) = state.status().filter(|r| r.state != ProgramState::Idle) else {
                continue;
            };
            let session = model
                .session(&state.session_id)
                .map(|s| s.label.clone())
                .unwrap_or_default();
            let view_block = model
                .views
                .get(&state.session_id)
                .and_then(|v| v.windows.iter().find_map(|w| w.block(block)).cloned())
                .unwrap_or_default();
            let mut title = titles::pane_title(&view_block, Some(state));
            if let Some(app) = &record.app {
                title = format!("{app} · {title}");
            }
            items.push((
                record.state,
                Item {
                    title,
                    detail: Some(match (&record.msg, &record.title) {
                        (Some(msg), _) => format!("{msg} · {session}"),
                        (None, Some(t)) => format!("{t} · {session}"),
                        _ => session,
                    }),
                    keys: None,
                    icon: titles::program_icon(Some(state)),
                    status: Some((record.state, record.badge())),
                    current: false,
                    choice: Choice::Block {
                        host: index,
                        session: state.session_id.clone(),
                        block: block.clone(),
                    },
                },
            ));
        }
    }
    items.sort_by_key(|i| std::cmp::Reverse(i.0));
    items.into_iter().map(|(_, item)| item).collect()
}

fn move_items(ws: &Workspace) -> Vec<Item> {
    let model = ws.model();
    let Some(view) = model.view() else {
        return Vec::new();
    };
    let Some(block) = view
        .active_window()
        .and_then(|w| w.focused_block_id.clone())
    else {
        return Vec::new();
    };
    view.windows
        .iter()
        .filter(|w| w.window_id != view.active_window_id)
        .filter_map(|w| {
            let anchor = w
                .focused_block_id
                .clone()
                .or_else(|| w.blocks().find_map(|b| b.block_id.clone()))?;
            Some(Item {
                title: titles::tab_title(w, &model),
                detail: None,
                keys: None,
                icon: "layout-sidebar",
                status: None,
                current: false,
                choice: Choice::MoveToWindow {
                    block: block.clone(),
                    anchor,
                },
            })
        })
        .collect()
}

fn host_items(ws: &Workspace) -> Vec<Item> {
    ws.hosts
        .iter()
        .enumerate()
        .map(|(index, host)| Item {
            title: host.config.label.clone(),
            detail: Some(host.handle.endpoint.to_string()),
            keys: None,
            icon: "server",
            status: None,
            current: index == ws.active,
            choice: Choice::Host(index),
        })
        .collect()
}

fn shader_items(ws: &Workspace) -> Vec<Item> {
    let terminal = &ws.settings.terminal;
    crate::settings::CursorShader::ALL
        .into_iter()
        .map(|shader| Item {
            title: shader.title().to_owned(),
            detail: Some(shader.description().to_owned()),
            keys: None,
            icon: "eye",
            status: None,
            current: terminal.cursor_shaders && terminal.cursor_shader == shader,
            choice: Choice::CursorShader(shader),
        })
        .collect()
}

fn shortcut_items(ws: &Workspace) -> Vec<Item> {
    actions::all()
        .filter(|a| actions::rebindable(a) && a.name != "client.key.forward")
        .map(|a| Item {
            title: a.title.to_owned(),
            detail: Some(a.category.to_owned()),
            keys: ws.keymap.keys_for(a.name, None).map(keymap::display),
            icon: "keyboard",
            status: None,
            current: false,
            choice: Choice::Shortcut {
                name: a.name.to_owned(),
                title: a.title.to_owned(),
            },
        })
        .collect()
}

fn removable_hosts(ws: &Workspace) -> Vec<Item> {
    ws.hosts
        .iter()
        .filter_map(|host| {
            let id = host.server_id.clone()?;
            Some(Item {
                title: host.config.label.clone(),
                detail: Some(host.handle.endpoint.to_string()),
                keys: None,
                icon: "server",
                status: None,
                current: false,
                choice: Choice::RemoveHost {
                    id,
                    label: host.config.label.clone(),
                },
            })
        })
        .collect()
}

/// Quote a path for a shell when it has characters the shell would read.
pub fn shell_quote(path: &str) -> String {
    if path
        .chars()
        .all(|c| c.is_alphanumeric() || "/._-~+@%:,".contains(c))
    {
        path.to_owned()
    } else {
        format!("'{}'", path.replace('\'', "'\\''"))
    }
}

/// Split a typed path into the directory to list and the name being typed.
pub fn split_path(typed: &str) -> (String, String) {
    match typed.rfind('/') {
        Some(0) => ("/".into(), typed[1..].into()),
        Some(i) => (typed[..i].into(), typed[i + 1..].into()),
        None if typed == "~" => ("~".into(), String::new()),
        None => (String::new(), typed.into()),
    }
}

fn focus_input(input: &Entity<TextInput>, window: &mut Window, cx: &mut Context<Workspace>) {
    let handle = gpui::Focusable::focus_handle(input.read(cx), cx);
    window.focus(&handle, cx);
}

impl Workspace {
    /// What's typed into an open list's field.
    pub fn list_query(&self, cx: &gpui::App) -> Option<String> {
        match &self.overlay {
            Some(Overlay::List(list)) => Some(list.input.read(cx).text().to_owned()),
            _ => None,
        }
    }

    /// The titles a list shows, in order.
    pub fn list_titles(&self) -> Vec<String> {
        match &self.overlay {
            Some(Overlay::List(list)) => list
                .shown
                .iter()
                .filter_map(|i| list.items.get(*i).map(|item| item.title.clone()))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Whether the selected result fits inside the list's last drawn viewport.
    pub fn list_selection_visible(&self) -> bool {
        let Some(Overlay::List(list)) = &self.overlay else {
            return false;
        };
        let viewport = list.scroll.bounds();
        list.scroll
            .bounds_for_item(list.selected)
            .is_some_and(|row| {
                let offset = list.scroll.offset().y;
                row.top() + offset >= viewport.top() && row.bottom() + offset <= viewport.bottom()
            })
    }

    /// The problem an open dialog reports, such as an address it can't use.
    pub fn dialog_error(&self) -> Option<String> {
        match &self.overlay {
            Some(Overlay::Dialog(Dialog::AddHost { error, .. })) => error.clone(),
            _ => None,
        }
    }

    pub fn has_dialog(&self) -> bool {
        matches!(self.overlay, Some(Overlay::Dialog(_)))
    }

    pub fn has_find(&self) -> bool {
        self.find.is_some()
    }

    /// The kind of list open over the workspace, if any.
    pub fn list_kind(&self) -> Option<ListKind> {
        match &self.overlay {
            Some(Overlay::List(list)) => Some(list.kind),
            _ => None,
        }
    }

    pub fn open_list(&mut self, kind: ListKind, window: &mut Window, cx: &mut Context<Self>) {
        let host = (kind == ListKind::Forwards).then(|| self.host().config.id.clone());
        self.open_host_list(kind, host, window, cx);
    }

    pub(super) fn open_host_list(
        &mut self,
        kind: ListKind,
        host: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items = match kind {
            ListKind::Commands => command_items(self),
            ListKind::Sessions => session_items(self),
            ListKind::GoTo if self.settings.interface.quick_switcher => quick_switcher_items(self),
            ListKind::GoTo => go_to_items(self),
            ListKind::FloatingPaneActions => floating_pane_items(self),
            ListKind::Activity => activity_items(self),
            ListKind::MoveToTab => move_items(self),
            ListKind::Hosts => host_items(self),
            ListKind::RemoveHost => removable_hosts(self),
            ListKind::Forwards => host
                .as_deref()
                .map(|host| self.forward_items(host))
                .unwrap_or_default(),
            ListKind::Shortcuts => shortcut_items(self),
            ListKind::CursorShaders => shader_items(self),
            ListKind::Directory { .. } => Vec::new(),
        };
        let placeholder = if kind == ListKind::GoTo && self.settings.interface.quick_switcher {
            "Sessions, panes and reporting programs"
        } else {
            kind.placeholder()
        };
        let input = cx.new(|cx| TextInput::new(placeholder, cx).list_navigation());
        let subscription =
            cx.subscribe_in(&input, window, |ws, _, event: &InputEvent, window, cx| {
                ws.on_list_input(event, window, cx);
            });
        let shown = (0..items.len()).collect();
        let selected = items.iter().position(|i| i.current).unwrap_or(0);
        let scroll = ScrollHandle::new();
        scroll.scroll_to_item(selected);
        focus_input(&input, window, cx);
        self.overlay = Some(Overlay::List(ListOverlay {
            kind,
            input,
            items,
            shown,
            selected,
            scroll,
            generation: 0,
            forward_host: host,
            _subscription: subscription,
        }));
        if let ListKind::Directory { .. } = kind {
            self.list_directory(String::new(), cx);
        }
        cx.notify();
    }

    pub fn close_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(&self.overlay, Some(Overlay::ScriptPrompt(_))) {
            self.answer_script_prompt(serde_json::json!({"cancelled": true}), window, cx);
            return;
        }
        self.overlay = None;
        self.refocus(window, cx);
        // A question asked from the Shortcuts page goes back to it, either way.
        if self.shortcut_return && self.recording_shortcut().is_none() {
            self.back_to_shortcuts(window, cx);
        }
        cx.notify();
    }

    /// Give focus back to the focused pane.
    pub fn refocus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focused = None;
        if self.pending_tab.is_some() {
            window.focus(&self.focus, cx);
            return;
        }
        if let Some(block) = self
            .model()
            .view()
            .and_then(View::active_window)
            .and_then(|w| w.focused_block_id.clone())
            && let Some(pane) = self.panes.get(&block)
        {
            let handle = gpui::Focusable::focus_handle(pane.read(cx), cx);
            window.focus(&handle, cx);
            self.focused = Some(block);
        }
    }

    fn on_list_input(&mut self, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        let known: Vec<String> = self
            .hosts
            .iter()
            .map(|h| h.config.endpoint.clone())
            .collect();
        let Some(Overlay::List(list)) = &mut self.overlay else {
            return;
        };
        match event {
            InputEvent::Changed(query) => {
                if let ListKind::Directory { .. } = list.kind {
                    let query = query.clone();
                    self.list_directory(query, cx);
                    return;
                }
                list.shown = fuzzy::rank(query, list.items.iter().map(|i| i.title.as_str()));
                // Matches on details (such as the host or directory) come after title matches.
                let mut extra = fuzzy::rank(
                    query,
                    list.items
                        .iter()
                        .map(|i| i.detail.as_deref().unwrap_or_default()),
                );
                extra.retain(|i| !list.shown.contains(i));
                list.shown.extend(extra);
                list.selected = 0;
                list.scroll.set_offset(gpui::Point::default());
                list.scroll.scroll_to_item(0);
                if list.kind == ListKind::Commands {
                    Self::offer_typed_host(list, query, &known);
                }
                if list.kind == ListKind::Sessions && !query.trim().is_empty() {
                    let name = query.trim().to_owned();
                    if !list
                        .items
                        .iter()
                        .any(|i| i.title.eq_ignore_ascii_case(&name))
                    {
                        list.items
                            .retain(|i| !matches!(i.choice, Choice::CreateSession { .. }));
                        list.items.push(Item {
                            title: format!("New session “{name}”"),
                            detail: None,
                            keys: None,
                            icon: "plus",
                            status: None,
                            current: false,
                            choice: Choice::CreateSession { label: name },
                        });
                        let index = list.items.len() - 1;
                        list.shown.retain(|i| *i != index);
                        list.shown.push(index);
                    }
                }
            }
            InputEvent::Navigate(step) => {
                let count = list.shown.len().max(1) as i32;
                list.selected = (list.selected as i32 + step).rem_euclid(count) as usize;
                list.scroll.scroll_to_item(list.selected);
            }
            InputEvent::Tab => {
                // In the path browser, Tab completes the selected entry.
                if let ListKind::Directory { .. } = list.kind
                    && let Some(item) = list
                        .shown
                        .get(list.selected)
                        .and_then(|i| list.items.get(*i))
                    && let Choice::Path { path, directory } = &item.choice
                {
                    let completed = if *directory {
                        format!("{path}/")
                    } else {
                        path.clone()
                    };
                    let input = list.input.clone();
                    input.update(cx, |input, cx| input.set_text(completed, cx));
                    input.update(cx, |input, cx| {
                        // Typing continues after the completion rather than replacing it.
                        let text = input.text().to_owned();
                        input.set_text(text, cx);
                    });
                }
            }
            InputEvent::Cancel => self.close_overlay(window, cx),
            InputEvent::Submit { alt, .. } => {
                let typed = list.input.read(cx).text().to_owned();
                let choice = list
                    .shown
                    .get(list.selected)
                    .and_then(|i| list.items.get(*i))
                    .map(|i| i.choice.clone());
                let kind = list.kind;
                if kind == ListKind::Forwards {
                    let host = list.forward_host.clone();
                    if *alt {
                        if let Some(Choice::OpenForward(id)) = choice {
                            self.close_forward(id, cx);
                            self.open_host_list(kind, host, window, cx);
                        }
                    } else if !typed.trim().is_empty() {
                        self.close_overlay(window, cx);
                        if let Some(host) = host {
                            self.forward_on_host(&host, &typed, cx);
                        }
                    } else if let Some(Choice::OpenForward(id)) = choice {
                        self.close_overlay(window, cx);
                        self.open_forward(id, cx);
                    }
                    return;
                }
                let choice = match (choice, kind) {
                    (Some(choice), _) => Some(choice),
                    // A path typed in full needs no matching entry.
                    (None, ListKind::Directory { .. }) if !typed.trim().is_empty() => {
                        Some(Choice::Path {
                            path: typed.trim().trim_end_matches('/').to_owned(),
                            directory: true,
                        })
                    }
                    _ => None,
                };
                if let Some(choice) = choice {
                    self.choose(choice, kind, window, cx);
                } else {
                    self.close_overlay(window, cx);
                }
            }
        }
        cx.notify();
    }

    /// The paths chosen last on this host, as list items: directories for Go to Directory,
    /// files too for Insert Path.
    fn recent_path_items(&self, files: bool) -> Vec<Item> {
        let host = &self.host().config.id;
        self.settings
            .recent_paths
            .get(host)
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                let directory = entry.ends_with('/');
                (directory || files).then(|| {
                    let path = entry.trim_end_matches('/').to_owned();
                    Item {
                        title: entry.clone(),
                        detail: Some("Recent".into()),
                        keys: None,
                        icon: "arrow-back-up",
                        status: None,
                        current: false,
                        choice: Choice::Path { path, directory },
                    }
                })
            })
            .take(8)
            .collect()
    }

    /// Put `path` first among this host's recent paths.
    fn remember_path(&mut self, path: &str, directory: bool, cx: &mut Context<Self>) {
        let host = self.host().config.id.clone();
        let entry = if directory {
            format!("{}/", path.trim_end_matches('/'))
        } else {
            path.to_owned()
        };
        self.update_settings(
            move |s| {
                let recent = s.recent_paths.entry(host).or_default();
                recent.retain(|p| *p != entry);
                recent.insert(0, entry);
                recent.truncate(20);
            },
            cx,
        );
    }

    fn list_directory(&mut self, typed: String, cx: &mut Context<Self>) {
        let Some(Overlay::List(list)) = &mut self.overlay else {
            return;
        };
        let ListKind::Directory { insert } = list.kind else {
            return;
        };
        list.generation += 1;
        let generation = list.generation;
        let model = self.model();
        let (Some(session), Some(block)) = (
            model.selected.clone(),
            model
                .view()
                .and_then(View::active_window)
                .and_then(|w| w.focused_block_id.clone()),
        ) else {
            return;
        };
        let (directory, prefix) = split_path(&typed);
        let host = self.host().handle.clone();
        cx.spawn(async move |this, cx| {
            let reply = host
                .request(
                    &format!("{TERMINAL_CREATOR}.list_dir"),
                    json!({"session_id": session, "block_id": block,
                        "args": {"path": directory, "prefix": prefix, "limit": 200, "files": insert}}),
                )
                .await;
            let _ = this.update(cx, |ws, cx| {
                // With nothing typed yet, the paths chosen last on this host come first.
                let recent = if typed.is_empty() && ws.settings.terminal.recent_paths {
                    ws.recent_path_items(insert)
                } else {
                    Vec::new()
                };
                let Some(Overlay::List(list)) = &mut ws.overlay else { return };
                if list.generation != generation {
                    return;
                }
                let Ok(reply) = reply else {
                    list.items.clear();
                    list.shown.clear();
                    cx.notify();
                    return;
                };
                let base = reply["path"].as_str().unwrap_or_default().trim_end_matches('/').to_owned();
                let home = reply["home"].as_str().unwrap_or_default().trim_end_matches('/').to_owned();
                let mut entries: Vec<(bool, String)> = reply["entries"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|e| Some((!e["file"].as_bool().unwrap_or(false), e["name"].as_str()?.to_owned())))
                    .filter(|(_, name)| !name.starts_with('.') || prefix.starts_with('.'))
                    .collect();
                entries.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.to_lowercase().cmp(&b.1.to_lowercase())));
                list.items = recent;
                list.items.extend(entries
                    .into_iter()
                    .map(|(directory, name)| {
                        let path = format!("{base}/{name}");
                        let shown = match path.strip_prefix(&home) {
                            Some(rest) if !home.is_empty() => format!("~{rest}"),
                            _ => path.clone(),
                        };
                        Item {
                            title: if directory { format!("{name}/") } else { name },
                            detail: Some(shown.clone()),
                            keys: None,
                            icon: if directory { "folder" } else { "file-code" },
                            status: None,
                            current: false,
                            choice: Choice::Path { path: shown, directory },
                        }
                    }));
                list.shown = (0..list.items.len()).collect();
                list.selected = 0;
                list.scroll.set_offset(gpui::Point::default());
                list.scroll.scroll_to_item(0);
                cx.notify();
            });
        })
        .detach();
    }

    fn choose(
        &mut self,
        choice: Choice,
        kind: ListKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(choice, Choice::Shortcut { .. }) {
            self.close_overlay(window, cx);
        }
        match choice {
            Choice::OpenForward(id) => self.open_forward(id, cx),
            Choice::Setting(path) => {
                if let Err(error) = self.open_indexed_setting(&path, window, cx) { self.notice = Some(error); }
            }
            Choice::Action { name, args } => {
                if let Err((_, message)) = self.run_action(&name, args, "palette", Target::default(), window, cx) {
                    self.notice = Some(message);
                }
            }
            Choice::Session { host, id } => self.select_session(host, id, cx),
            Choice::AddTypedHost { label, address } => {
                self.open_add_host(window, cx);
                if let Some(Overlay::Dialog(Dialog::AddHost {
                    label: name,
                    address: target,
                    ..
                })) = &self.overlay
                {
                    name.update(cx, |input, cx| input.set_text(label, cx));
                    target.update(cx, |input, cx| input.set_text(address, cx));
                }
            }
            Choice::CreateSession { label } => {
                self.hosts[self.active].handle.send(HostCommand::CreateSession {
                    label: Some(label),
                    reply: None,
                });
            }
            Choice::Window { host, session, window: id } => {
                self.select_session(host, session.clone(), cx);
                self.focus_tab(host, session, id);
            }
            Choice::Block { host, session, block } => {
                self.select_session(host, session.clone(), cx);
                self.hosts[host]
                    .handle
                    .call("session.focus_block", json!({"session_id": session, "block_id": block}));
            }
            Choice::FloatingPaneActions { host, session, block } => {
                let model = self.model();
                if self.active == host && model.selected.as_ref() == Some(&session)
                    && model.view().and_then(View::active_window).is_some_and(|tab| {
                        tab.panes().iter().any(|pane| pane.block_id == block && pane.floating.is_some())
                    })
                {
                    let at = self.element_bounds("floating-pane-actions").map_or_else(
                        || gpui::point(self.area.right(), self.area.bottom()),
                        |bounds| bounds.center(),
                    );
                    self.open_floating_header_menu(block, at, cx);
                }
            }
            Choice::MoveToWindow { block, anchor } => {
                if let Some(session) = self.model().selected {
                    self.host().handle.call(
                        "session.move_block",
                        json!({"session_id": session, "block_id": block, "anchor_block_id": anchor}),
                    );
                }
            }
            Choice::Host(index) => self.switch_host(index, cx),
            Choice::CursorShader(shader) => self.choose_cursor_shader(shader, cx),
            Choice::Shortcut { name, title } => self.record_shortcut(name, title, window, cx),
            Choice::RemoveHost { id, label } => self.open_confirm(
                format!("Remove {label}?"),
                "Every device that uses this home server stops listing it. Its sessions keep running.".into(),
                "Remove Host".into(),
                Confirm::RemoveHost { id },
                cx,
            ),
            Choice::Path { path, directory } => {
                self.remember_path(&path, directory, cx);
                let model = self.model();
                let Some(session) = model.selected.clone() else { return };
                if kind == (ListKind::Directory { insert: true }) {
                    if let Some(block) = model.view().and_then(View::active_window).and_then(|w| w.focused_block_id.clone())
                        && let Some(pane) = self.panes.get(&block)
                    {
                        let text = format!("{} ", shell_quote(&path));
                        pane.update(cx, |pane, _| pane.paste_text(text));
                    }
                } else {
                    self.host().handle.call(
                        "session.new_window",
                        json!({"session_id": session, "layout": shell_layout_for(self.host().handle.endpoint.is_local(), Some(&path)), "focus": true}),
                    );
                }
            }
        }
        cx.notify();
    }

    pub fn open_rename(
        &mut self,
        target: RenameTarget,
        current: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| TextInput::new("Leave empty to use the default name", cx));
        input.update(cx, |input, cx| input.set_text(current, cx));
        let subscription = cx.subscribe_in(
            &input,
            window,
            |ws, input, event: &InputEvent, window, cx| match event {
                InputEvent::Submit { .. } => {
                    let label = input.read(cx).text().trim().to_owned();
                    ws.submit_rename(label, window, cx);
                }
                InputEvent::Cancel => ws.close_overlay(window, cx),
                _ => {}
            },
        );
        focus_input(&input, window, cx);
        self.overlay = Some(Overlay::Dialog(Dialog::Rename {
            target,
            input,
            _subscription: subscription,
        }));
        cx.notify();
    }

    fn submit_rename(&mut self, label: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Overlay::Dialog(Dialog::Rename { target, .. })) = &self.overlay else {
            return;
        };
        let target = target.clone();
        let Some(session) = self.model().selected else {
            return;
        };
        let (method, payload) = match target {
            RenameTarget::Session(id) => (
                "session.set_label",
                json!({"session_id": id, "label": label}),
            ),
            RenameTarget::Window(id) => (
                "session.set_window_label",
                json!({"session_id": session, "window_id": id, "label": label}),
            ),
            RenameTarget::Block(id) => (
                "session.set_block_label",
                json!({"session_id": session, "block_id": id, "label": label}),
            ),
        };
        self.host().handle.call(method, payload);
        self.close_overlay(window, cx);
    }

    /// What the rename dialog is renaming, while it's open, for tests: `session ID`,
    /// `tab ID` or `pane ID`.
    pub fn renaming(&self) -> Option<String> {
        match &self.overlay {
            Some(Overlay::Dialog(Dialog::Rename { target, .. })) => Some(match target {
                RenameTarget::Session(id) => format!("session {id}"),
                RenameTarget::Window(id) => format!("tab {id}"),
                RenameTarget::Block(id) => format!("pane {id}"),
            }),
            _ => None,
        }
    }

    /// The open confirmation's title, for tests.
    pub fn confirm_title(&self) -> Option<String> {
        match &self.overlay {
            Some(Overlay::Dialog(Dialog::Confirm { title, .. })) => Some(title.clone()),
            Some(Overlay::Dialog(Dialog::ForwardPort { request, .. })) => {
                Some(forward_port_title(request.port()))
            }
            _ => None,
        }
    }

    pub fn open_confirm(
        &mut self,
        title: String,
        body: String,
        confirm: String,
        action: Confirm,
        cx: &mut Context<Self>,
    ) {
        self.overlay = Some(Overlay::Dialog(Dialog::Confirm {
            title,
            body,
            confirm,
            action,
            focused: 0,
            programs: Vec::new(),
        }));
        cx.notify();
    }

    /// A confirmation naming busy programs, where Enter goes ahead.
    pub(super) fn open_confirm_busy(
        &mut self,
        title: String,
        body: String,
        confirm: String,
        action: Confirm,
        programs: Vec<super::close_guard::Busy>,
        cx: &mut Context<Self>,
    ) {
        self.overlay = Some(Overlay::Dialog(Dialog::Confirm {
            title,
            body,
            confirm,
            action,
            focused: 1,
            programs,
        }));
        cx.notify();
    }

    /// The busy programs the open confirmation names, as "title (state)", for tests.
    pub fn confirm_programs(&self) -> Option<Vec<String>> {
        match &self.overlay {
            Some(Overlay::Dialog(Dialog::Confirm { programs, .. })) => Some(
                programs
                    .iter()
                    .map(|p| format!("{} ({})", p.title, p.state))
                    .collect(),
            ),
            _ => None,
        }
    }

    /// Leave the question and go to a busy program's pane.
    pub fn go_to_busy(&mut self, block: String, window: &mut Window, cx: &mut Context<Self>) {
        self.close_overlay(window, cx);
        if let Some(session) = self.session_id() {
            self.call(
                "session.focus_block",
                json!({"session_id": session, "block_id": block}),
            );
        }
    }

    pub fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Overlay::Dialog(Dialog::Confirm { action, .. })) = &self.overlay else {
            return;
        };
        let action = action.clone();
        self.close_overlay(window, cx);
        match action {
            Confirm::Paste { block, text } => {
                if let Some(pane) = self.panes.get(&block) {
                    pane.read(cx).paste_text(text);
                }
            }
            Confirm::Close(target) => self.close_now(target),
            Confirm::ResetShortcut { action, from } => self.reset_now(&action, Some(from), cx),
            Confirm::RestartServer => self.restart_server(cx),
            Confirm::OpenFile(uri) => {
                self.finish_file_open(super::quick_look::Shown::Opener(uri), cx)
            }
            Confirm::CloseSession { id } => self
                .host()
                .handle
                .call("session.destroy", json!({"session_id": id})),
            Confirm::RemoveHost { id } => self.hosts[self.home()]
                .handle
                .call("hosts.remove", json!({"id": id})),
        }
    }

    pub fn open_add_host(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let label = cx.new(|cx| TextInput::new("studio", cx));
        let address =
            cx.new(|cx| TextInput::new("studio.tail1234.ts.net, https://… or ssh://user@host", cx));
        let on_event = |field: usize| {
            move |ws: &mut Workspace,
                  _: &Entity<TextInput>,
                  event: &InputEvent,
                  window: &mut Window,
                  cx: &mut Context<Workspace>| {
                match event {
                    InputEvent::Submit { .. } => ws.submit_add_host(window, cx),
                    InputEvent::Cancel => ws.close_overlay(window, cx),
                    InputEvent::Tab | InputEvent::Navigate(_) => {
                        if let Some(Overlay::Dialog(Dialog::AddHost { label, address, .. })) =
                            &ws.overlay
                        {
                            let next = if field == 0 { address } else { label };
                            focus_input(next, window, cx);
                        }
                    }
                    InputEvent::Changed(_) => {
                        if let Some(Overlay::Dialog(Dialog::AddHost { error, .. })) =
                            &mut ws.overlay
                        {
                            *error = None;
                        }
                    }
                }
                cx.notify();
            }
        };
        let subscriptions = [
            cx.subscribe_in(&label, window, on_event(0)),
            cx.subscribe_in(&address, window, on_event(1)),
        ];
        focus_input(&label, window, cx);
        self.overlay = Some(Overlay::Dialog(Dialog::AddHost {
            label,
            address,
            error: None,
            _subscriptions: subscriptions,
        }));
        cx.notify();
    }

    fn submit_add_host(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Overlay::Dialog(Dialog::AddHost {
            label,
            address,
            error,
            ..
        })) = &mut self.overlay
        else {
            return;
        };
        let name = label.read(cx).text().trim().to_owned();
        let target = address.read(cx).text().trim().to_owned();
        let problem = if name.is_empty() {
            Some("Name the host".to_owned())
        } else if target.is_empty() {
            Some("Enter the host's address".to_owned())
        } else {
            Endpoint::parse(&target).err().map(|e| e.to_string())
        };
        if let Some(problem) = problem {
            *error = Some(problem);
            cx.notify();
            return;
        }
        let home = self.hosts[self.home()].handle.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = home
                .request(
                    "hosts.put",
                    json!({"host": {"label": name, "endpoints": [target]}}),
                )
                .await;
            let _ = this.update_in(cx, |ws, window, cx| match result {
                Ok(_) => ws.close_overlay(window, cx),
                Err(message) => {
                    if let Some(Overlay::Dialog(Dialog::AddHost { error, .. })) = &mut ws.overlay {
                        *error = Some(message);
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub fn open_diagnostics(&mut self, text: String, cx: &mut Context<Self>) {
        self.overlay = Some(Overlay::Dialog(Dialog::Diagnostics { text }));
        cx.notify();
    }

    pub fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(block) = self
            .model()
            .view()
            .and_then(View::active_window)
            .and_then(|w| w.focused_block_id.clone())
        else {
            return;
        };
        if let Some(find) = &self.find
            && find.block == block
        {
            let input = find.input.clone();
            input.update(cx, |input, cx| {
                let text = input.text().to_owned();
                input.set_text(text, cx);
            });
            focus_input(&input, window, cx);
            return;
        }
        let input = cx.new(|cx| TextInput::new("Find", cx));
        let subscription =
            cx.subscribe_in(&input, window, |ws, _, event: &InputEvent, window, cx| {
                let Some(find) = &ws.find else { return };
                let pane = ws.panes.get(&find.block).cloned();
                match event {
                    InputEvent::Changed(text) => {
                        if let Some(pane) = pane {
                            let needle = (!text.is_empty()).then(|| text.clone());
                            pane.update(cx, |pane, _| pane.search(needle));
                        }
                    }
                    // Enter finds the next older match, Shift+Enter the newer one; Up and Down too.
                    InputEvent::Submit { shift, .. } => {
                        let older = !shift;
                        if let Some(pane) = pane {
                            pane.update(cx, |pane, _| pane.search_step(older));
                        }
                    }
                    InputEvent::Navigate(step) => {
                        let older = *step < 0;
                        if let Some(pane) = pane {
                            pane.update(cx, |pane, _| pane.search_step(older));
                        }
                    }
                    InputEvent::Cancel => ws.close_find(window, cx),
                    _ => {}
                }
                cx.notify();
            });
        focus_input(&input, window, cx);
        self.find = Some(FindBar {
            input,
            block,
            _subscription: subscription,
        });
        cx.notify();
    }

    pub fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(find) = self.find.take()
            && let Some(pane) = self.panes.get(&find.block)
        {
            pane.update(cx, |pane, _| pane.search(None));
        }
        self.refocus(window, cx);
        cx.notify();
    }

    pub fn find_step(&mut self, older: bool, window: &mut Window, cx: &mut Context<Self>) {
        match &self.find {
            Some(find) => {
                if let Some(pane) = self.panes.get(&find.block) {
                    pane.update(cx, |pane, _| pane.search_step(older));
                }
            }
            None => self.open_find(window, cx),
        }
    }

    /// Confirm dialogs answer Enter, Esc and Tab themselves.
    pub(super) fn dialog_key(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(Overlay::Dialog(dialog)) = &mut self.overlay else {
            return false;
        };
        match dialog {
            Dialog::ForwardPort {
                authorize, focused, ..
            } => match key {
                "escape" => self.close_overlay(window, cx),
                "enter" => {
                    let choice = *focused;
                    self.answer_forward_port(choice, window, cx);
                }
                "tab" | "right" | "left" => {
                    let count = if *authorize { 3 } else { 2 };
                    *focused = (*focused + if key == "left" { count - 1 } else { 1 }) % count;
                    cx.notify();
                }
                _ => {}
            },
            Dialog::OpenLink { focused, .. } => match key {
                "escape" => self.close_overlay(window, cx),
                "enter" => match *focused {
                    0 => self.close_overlay(window, cx),
                    choice => self.answer_link(choice == 1, window, cx),
                },
                "tab" | "right" => {
                    *focused = (*focused + 1) % 3;
                    cx.notify();
                }
                "left" => {
                    *focused = (*focused + 2) % 3;
                    cx.notify();
                }
                _ => {}
            },
            Dialog::Confirm { focused, .. } => match key {
                "escape" => self.close_overlay(window, cx),
                "enter" => {
                    if *focused == 1 {
                        self.confirm(window, cx);
                    } else {
                        self.close_overlay(window, cx);
                    }
                }
                "tab" | "left" | "right" => {
                    *focused = 1 - *focused;
                    cx.notify();
                }
                _ => {}
            },
            Dialog::Diagnostics { .. } => {
                if matches!(key, "escape" | "enter") {
                    self.close_overlay(window, cx);
                }
            }
            _ => return false,
        }
        true
    }

    pub(super) fn render_overlay(
        &self,
        window: &Window,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let overlay = self.overlay.as_ref()?;
        let panel = div()
            .flex()
            .flex_col()
            .bg(theme.surface)
            .rounded(theme.radius_large())
            .when(theme.popup_borders, |d| {
                d.border(theme.border_width).border_color(theme.border)
            })
            .when(theme.popup_shadows, |d| d.shadow_lg())
            .overflow_hidden()
            .font_family(theme.ui_font.clone())
            .text_color(theme.text);
        let content = match overlay {
            Overlay::List(list) => panel
                .w(px(640.))
                .when(list.kind == ListKind::FloatingPaneActions, |d| {
                    d.max_w(window.viewport_size().width - px(32.))
                })
                .max_h(px(520.))
                .child(self.render_list(list, window, theme, cx))
                .into_any_element(),
            Overlay::Dialog(dialog) => panel
                .w(px(460.))
                .p(px(18.))
                .gap(px(12.))
                .child(self.render_dialog(dialog, theme, cx))
                .into_any_element(),
            Overlay::Settings(page) => panel
                .w(px(820.))
                .h(px(600.))
                .child(self.render_settings(page, theme, cx))
                .into_any_element(),
            Overlay::ScriptPrompt(prompt) => panel
                .w(px(640.).min(window.viewport_size().width - px(32.)))
                .max_h(px(520.).min(window.viewport_size().height - px(96.)))
                .child(self.render_script_prompt(prompt, theme, cx))
                .into_any_element(),
        };
        Some(
            div()
                .id("overlay-scrim")
                // Clicks on Settings, dialogs and lists stay off the panes behind.
                .occlude()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .items_center()
                .pt(px(72.))
                .bg(gpui::hsla(0., 0., 0., 0.35))
                .on_click(cx.listener(|ws, _, window, cx| ws.close_overlay(window, cx)))
                .child(
                    div()
                        .id("overlay-panel")
                        // Clicks inside the panel don't reach the scrim.
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(content),
                )
                .into_any_element(),
        )
    }

    fn render_list(
        &self,
        list: &ListOverlay,
        window: &Window,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rows: Vec<AnyElement> = list
            .shown
            .iter()
            .enumerate()
            .filter_map(|(position, index)| {
                let item = list.items.get(*index)?.clone();
                let selected = position == list.selected;
                let (fg, muted) = if selected {
                    (theme.on_primary, theme.on_primary)
                } else {
                    (theme.text, theme.text_muted)
                };
                let kind = list.kind;
                let choice = item.choice.clone();
                let terminal_colors = match &choice {
                    Choice::Block { host, block, .. }
                    | Choice::FloatingPaneActions { host, block, .. } => {
                        self.hosts.get(*host).and_then(|host| {
                            crate::ui::program_icons::pane_colors(&host.handle.snapshot(), block)
                        })
                    }
                    _ => None,
                };
                Some(
                    div()
                        .id(SharedString::from(format!("item-{position}")))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .px(px(12.))
                        .h(px(40.))
                        .flex_none()
                        .rounded(theme.radius_small())
                        .cursor_pointer()
                        .when(selected, |d| d.bg(theme.primary))
                        .when(!selected, |d| d.hover(|d| d.bg(theme.overlay(0.06))))
                        .text_color(fg)
                        .child(crate::ui::program_icons::render_icon(
                            item.icon,
                            20.,
                            if selected { fg } else { theme.primary },
                            theme,
                            terminal_colors,
                        ))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .text_size(px(14.))
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .whitespace_nowrap()
                                        .child(item.title.clone()),
                                )
                                .children(item.detail.clone().map(|detail| {
                                    div()
                                        .text_size(px(11.))
                                        .text_color(muted)
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .whitespace_nowrap()
                                        .child(detail)
                                })),
                        )
                        .children(item.status.map(|(state, badge)| {
                            let color = if selected {
                                fg
                            } else {
                                super::state_color(state, theme)
                            };
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .child(dot(color, 7.))
                                .child(div().text_size(px(11.)).text_color(color).child(badge))
                        }))
                        .when(item.current, |d| {
                            d.child(div().text_size(px(11.)).text_color(muted).child("Current"))
                        })
                        .children(item.keys.clone().map(|keys| {
                            div()
                                .px(px(6.))
                                .py(px(2.))
                                .rounded(theme.radius_small())
                                .when(theme.button_borders, |d| {
                                    d.border(theme.border_width).border_color(if selected {
                                        fg
                                    } else {
                                        theme.border
                                    })
                                })
                                .text_size(px(11.))
                                .text_color(muted)
                                .child(keys)
                        }))
                        .on_click(cx.listener(move |ws, _, window, cx| {
                            ws.choose(choice.clone(), kind, window, cx);
                        }))
                        .into_any_element(),
                )
            })
            .collect();
        let empty = rows.is_empty().then(|| {
            div()
                .px(px(12.))
                .py(px(14.))
                .text_size(px(13.))
                .text_color(theme.text_muted)
                .child(match list.kind {
                    ListKind::Activity => "No program has reported a status",
                    ListKind::MoveToTab => "This session has no other tab",
                    ListKind::FloatingPaneActions if list.items.is_empty() => {
                        "No visible floating panes"
                    }
                    ListKind::RemoveHost => "No remote hosts are saved on the home server",
                    _ => "No matches",
                })
        });
        div()
            .flex()
            .flex_col()
            .child(div().p(px(10.)).child(list.input.clone()))
            .child(
                div()
                    .id("overlay-list")
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .px(px(8.))
                    .pb(px(8.))
                    .max_h((window.viewport_size().height - px(144.)).clamp(px(40.), px(440.)))
                    .overflow_y_scroll()
                    .track_scroll(&list.scroll)
                    .children(rows)
                    .children(empty),
            )
            .into_any_element()
    }

    fn render_dialog(&self, dialog: &Dialog, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let heading = |text: String| {
            div()
                .text_size(px(16.))
                .font_weight(FontWeight::BOLD)
                .child(text)
        };
        let button =
            |id: &'static str, label: String, filled: Option<gpui::Hsla>, focused: bool| {
                let (bg, fg) = match filled {
                    Some(color) => (color, theme.background),
                    None => (theme.background, theme.text),
                };
                div()
                    .id(id)
                    .flex()
                    .items_center()
                    .h(px(34.))
                    .px(px(14.))
                    .rounded(theme.radius_small())
                    .bg(bg)
                    .text_color(fg)
                    .text_size(px(13.))
                    .cursor_pointer()
                    .when(filled.is_none() && theme.button_borders, |d| {
                        d.border(theme.border_width).border_color(theme.border)
                    })
                    .when(focused, |d| d.border_2().border_color(theme.primary))
                    .child(label)
            };
        let actions_row = |children: Vec<AnyElement>| {
            div()
                .flex()
                .justify_end()
                .gap(px(8.))
                .pt(px(4.))
                .children(children)
        };
        match dialog {
            Dialog::Rename { target, input, .. } => {
                let title = match target {
                    RenameTarget::Session(_) => "Rename Session",
                    RenameTarget::Window(_) => "Rename Tab",
                    RenameTarget::Block(_) => "Rename Pane",
                };
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(heading(title.into()))
                    .child(input.clone())
                    .child(actions_row(vec![
                        button("rename-cancel", "Cancel".into(), None, false)
                            .on_click(cx.listener(|ws, _, window, cx| ws.close_overlay(window, cx)))
                            .into_any_element(),
                        button("rename-save", "Rename".into(), Some(theme.primary), false)
                            .on_click(cx.listener(|ws, _, window, cx| {
                                let label = match &ws.overlay {
                                    Some(Overlay::Dialog(Dialog::Rename { input, .. })) => {
                                        input.read(cx).text().trim().to_owned()
                                    }
                                    _ => return,
                                };
                                ws.submit_rename(label, window, cx);
                            }))
                            .into_any_element(),
                    ]))
                    .into_any_element()
            }
            Dialog::ForwardPort {
                request,
                error,
                authorize,
                focused,
            } => {
                let port = request.port();
                let mut buttons = vec![
                    button("forward-cancel", "Cancel".into(), None, *focused == 0)
                        .on_click(
                            cx.listener(|ws, _, window, cx| ws.answer_forward_port(0, window, cx)),
                        )
                        .into_any_element(),
                    button(
                        "forward-free",
                        "Use a Free Port".into(),
                        Some(theme.primary),
                        *focused == 1,
                    )
                    .on_click(
                        cx.listener(|ws, _, window, cx| ws.answer_forward_port(1, window, cx)),
                    )
                    .into_any_element(),
                ];
                if *authorize {
                    buttons.push(
                        button(
                            "forward-authorize",
                            format!("Authorize Port {port}…"),
                            None,
                            *focused == 2,
                        )
                        .on_click(
                            cx.listener(|ws, _, window, cx| ws.answer_forward_port(2, window, cx)),
                        )
                        .into_any_element(),
                    );
                }
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(heading(forward_port_title(port)))
                    .when_some(
                        error.as_ref().cloned().or_else(|| {
                            (!authorize).then(|| "System port authorization is unavailable.".into())
                        }),
                        |d, message| {
                            d.child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(theme.text_muted)
                                    .child(message),
                            )
                        },
                    )
                    .child(actions_row(buttons))
                    .into_any_element()
            }
            Dialog::OpenLink { link, focused } => div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(heading("Open this link in another app?".into()))
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(theme.text_muted)
                        .child(format!(
                            "{link}\n\nIt opens in whichever app handles {} links.",
                            link.split(':').next().unwrap_or("these")
                        )),
                )
                .child(actions_row(vec![
                    button("link-cancel", "Cancel".into(), None, *focused == 0)
                        .on_click(cx.listener(|ws, _, window, cx| ws.close_overlay(window, cx)))
                        .into_any_element(),
                    button("link-always", "Always Open".into(), None, *focused == 1)
                        .on_click(cx.listener(|ws, _, window, cx| ws.answer_link(true, window, cx)))
                        .into_any_element(),
                    button(
                        "link-open",
                        "Open".into(),
                        Some(theme.primary),
                        *focused == 2,
                    )
                    .on_click(cx.listener(|ws, _, window, cx| ws.answer_link(false, window, cx)))
                    .into_any_element(),
                ]))
                .into_any_element(),
            Dialog::Confirm {
                title,
                body,
                confirm,
                focused,
                action,
                programs,
            } => div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(heading(title.clone()))
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(theme.text_muted)
                        .child(body.clone()),
                )
                .children(programs.iter().map(|program| {
                    let block = program.block.clone();
                    div()
                        .id(gpui::SharedString::from(format!("busy-{}", program.block)))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(12.))
                        .px(px(10.))
                        .py(px(6.))
                        .rounded(theme.radius_small())
                        .bg(theme.overlay(0.05))
                        .cursor_pointer()
                        .hover(|d| d.bg(theme.overlay(0.1)))
                        .on_click(cx.listener(move |ws, _, window, cx| {
                            ws.go_to_busy(block.clone(), window, cx)
                        }))
                        .child(div().text_size(px(13.)).child(program.title.clone()))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(theme.primary)
                                .child(program.state.clone()),
                        )
                }))
                .child(actions_row(vec![
                    button(
                        "confirm-cancel",
                        action.keep_label().into(),
                        None,
                        *focused == 0,
                    )
                    .on_click(cx.listener(|ws, _, window, cx| ws.close_overlay(window, cx)))
                    .into_any_element(),
                    button(
                        "confirm-ok",
                        confirm.clone(),
                        Some(theme.error),
                        *focused == 1,
                    )
                    .on_click(cx.listener(|ws, _, window, cx| ws.confirm(window, cx)))
                    .into_any_element(),
                ]))
                .into_any_element(),
            Dialog::AddHost {
                label,
                address,
                error,
                ..
            } => div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(heading("Add Remote Host".into()))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme.text_muted)
                        .child("Name"),
                )
                .child(label.clone())
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme.text_muted)
                        .child("Address"),
                )
                .child(address.clone())
                .children(
                    error
                        .clone()
                        .map(|e| div().text_size(px(12.)).text_color(theme.error).child(e)),
                )
                .child(actions_row(vec![
                    button("host-cancel", "Cancel".into(), None, false)
                        .on_click(cx.listener(|ws, _, window, cx| ws.close_overlay(window, cx)))
                        .into_any_element(),
                    button("host-add", "Add Host".into(), Some(theme.primary), false)
                        .on_click(cx.listener(|ws, _, window, cx| ws.submit_add_host(window, cx)))
                        .into_any_element(),
                ]))
                .into_any_element(),
            Dialog::Shortcut { action, title, .. } => div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(heading("Change Shortcut".into()))
                .child(
                    div()
                        .text_size(px(14.))
                        .whitespace_normal()
                        .child(format!("Press the new key for {title}.")),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .whitespace_normal()
                        .text_color(theme.text_muted)
                        .child(format!("Now: {}", self.shortcut_of(action))),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .whitespace_normal()
                        .text_color(theme.text_muted)
                        .child("Esc or ← goes back · Backspace removes it"),
                )
                .into_any_element(),
            Dialog::Diagnostics { text } => {
                let copy = text.clone();
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(heading("Rex Diagnostics".into()))
                    .child(
                        div()
                            .id("diagnostics-text")
                            .max_h(px(320.))
                            .overflow_y_scroll()
                            .p(px(10.))
                            .rounded(theme.radius_small())
                            .bg(theme.background)
                            .text_size(px(12.))
                            .font_family("monospace")
                            .children(text.lines().map(|line| div().child(line.to_owned()))),
                    )
                    .child(actions_row(vec![
                        button("diagnostics-copy", "Copy".into(), None, false)
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(copy.clone()))
                            }))
                            .into_any_element(),
                        button(
                            "diagnostics-close",
                            "Done".into(),
                            Some(theme.primary),
                            false,
                        )
                        .on_click(cx.listener(|ws, _, window, cx| ws.close_overlay(window, cx)))
                        .into_any_element(),
                    ]))
                    .into_any_element()
            }
        }
    }

    pub(super) fn render_find(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let find = self.find.as_ref()?;
        let count = self
            .host()
            .handle
            .pane(&find.block)
            .and_then(|p| p.snapshot().frame)
            .and_then(|f| f.search)
            .map(|(total, selected)| match (total, selected) {
                (0, _) => "No matches".to_owned(),
                (total, Some(i)) => format!("{} of {total}", i + 1),
                (total, None) => format!("{total} matches"),
            });
        Some(
            div()
                .absolute()
                .top(px(58.))
                .right(px(16.))
                .w(px(380.))
                .flex()
                .items_center()
                .gap(px(6.))
                .p(px(6.))
                .rounded(theme.radius())
                .bg(theme.surface)
                .when(theme.popup_borders, |d| {
                    d.border(theme.border_width).border_color(theme.border)
                })
                .when(theme.popup_shadows, |d| d.shadow_lg())
                .child(div().flex_1().child(find.input.clone()))
                .children(count.map(|c| {
                    div()
                        .text_size(px(12.))
                        .text_color(theme.text_muted)
                        .child(c)
                }))
                .child(
                    icon_button(
                        "find-newer",
                        "chevron-down",
                        theme,
                        cx.listener(|ws, _, window, cx| ws.find_step(false, window, cx)),
                    )
                    .tooltip(self.tip("client.find.previous", None)),
                )
                .child(
                    icon_button(
                        "find-close",
                        "x",
                        theme,
                        cx.listener(|ws, _, window, cx| ws.close_find(window, cx)),
                    )
                    .tooltip(crate::ui::components::tooltip(
                        "Close Find",
                        Some("Escape".into()),
                    )),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_split_and_quote_for_shells() {
        assert_eq!(split_path("~/dev/re"), ("~/dev".into(), "re".into()));
        assert_eq!(split_path("/et"), ("/".into(), "et".into()));
        assert_eq!(split_path("src"), ("".into(), "src".into()));
        assert_eq!(split_path("~"), ("~".into(), "".into()));
        assert_eq!(shell_quote("~/dev/rexia"), "~/dev/rexia");
        assert_eq!(shell_quote("/tmp/my dir"), "'/tmp/my dir'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }
}

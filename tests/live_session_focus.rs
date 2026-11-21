//! Opt-in: only sessions created by this test are selected or queried for history.
//! Set REXIA_TEST_SERVER to an existing Unix Rex socket exposing focus_history.reverse
//! (API 19). The `rex` CLI on PATH must support that API; the host itself continues
//! negotiating API 18. No server is started and no GUI navigation is performed.
use rexia::rex::{
    client::{Control, Identity},
    endpoint::Endpoint,
    host::{Connection, HostCommand, HostHandle, shell_layout},
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn request(host: &HostHandle, method: &str, payload: Value) -> Value {
    let (reply, answer) = tokio::sync::oneshot::channel();
    host.send(HostCommand::Call {
        method: method.into(),
        payload,
        reply: Some(reply),
    });
    answer.blocking_recv().unwrap().unwrap()
}

fn wait(host: &HostHandle, what: &str, check: impl Fn(&rexia::rex::host::HostModel) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !check(&host.snapshot()) {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}: {:?}",
            host.snapshot().connection
        );
        let _ = host.wake.recv_timeout(Duration::from_millis(50));
    }
}

// Seed via a private control connection, before a host worker can bootstrap and
// auto-select any pre-existing session. No GUI or pre-existing pane is touched.
fn raw(address: &str, action: impl AsyncFnOnce(&Control)) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let (control, _events) = Control::connect(
            &Endpoint::parse(address).unwrap(),
            &Identity::default(),
            None,
        )
        .await
        .unwrap();
        action(&control).await;
        control.close();
    });
}

struct Owned {
    address: String,
    ids: Vec<String>,
}
impl Owned {
    fn create(&mut self) -> String {
        let mut created = None;
        raw(&self.address, async |control| {
            let value = control
                .value(
                    "session.create",
                    json!({
                        "label": "rexia-focus-test", "initial_windows": [{"layout": shell_layout()}]
                    }),
                )
                .await
                .unwrap();
            created = Some(value["session_id"].as_str().unwrap().to_owned());
        });
        let id = created.unwrap();
        self.ids.push(id.clone());
        id
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        raw(&self.address, async |control| {
            for id in &self.ids {
                control
                    .value("session.attach", json!({"session_id": id}))
                    .await
                    .unwrap();
                control
                    .value("session.destroy", json!({"session_id": id}))
                    .await
                    .unwrap();
            }
            let listed = control.value("session.list", json!({})).await.unwrap();
            for id in &self.ids {
                assert!(
                    !listed["sessions"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|s| s["session_id"] == *id),
                    "owned session remains: {id}"
                );
            }
        });
    }
}

fn select(host: &HostHandle, id: &str) {
    host.send(HostCommand::Select(id.into()));
    wait(host, "selection", |m| m.selected.as_deref() == Some(id));
    // Barrier: the worker has queued focus before answering the following request.
    request(host, "session.view", json!({"session_id": id}));
}

#[test]
fn session_switch_reports_existing_focused_block() {
    let Ok(address) = std::env::var("REXIA_TEST_SERVER") else {
        eprintln!("skipped: set REXIA_TEST_SERVER to an existing disposable Unix Rex socket");
        return;
    };
    assert!(
        address.starts_with("unix://"),
        "only Unix sockets are permitted"
    );
    let mut owned = Owned {
        address: address.clone(),
        ids: Vec::new(),
    };
    let first = owned.create();
    let second = owned.create();
    let host = HostHandle::spawn(
        Endpoint::parse(&address).unwrap(),
        Identity::default(),
        Some(first.clone()),
    );
    wait(&host, "preferred private session", |m| {
        m.connection == Connection::Connected
            && m.selected.as_deref() == Some(&first)
            && m.views.contains_key(&first)
    });
    let block = |id: &str| {
        host.snapshot().views[id]
            .active_window()
            .unwrap()
            .focused_block_id
            .clone()
            .unwrap()
    };
    let first_block = block(&first);
    select(&host, &second);
    let second_block = block(&second);
    // Rexia negotiates API 18. The installed API 19 CLI reads this principal's
    // server history; autostart is explicitly disabled and target is owned.
    let history = |target: &str, source: &str| {
        let client = host.snapshot().client_id.unwrap();
        let output = std::process::Command::new("rex")
            .args([
                "--autostart=false",
                "-S",
                &address,
                "-C",
                &client,
                "api",
                "call",
                "focus_history.reverse",
            ])
            .arg(json!({"session_id": target, "not_session_id": source}).to_string())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    // The very first departure must preserve the initially displayed session,
    // without a previous explicit focus call or a return visit to seed history.
    let initial = history(&first, &second);
    assert_eq!(
        initial["reversed"], true,
        "initial session missing: {initial}"
    );
    assert_eq!(initial["block_id"], first_block);
    select(&host, &first);
    assert_eq!(block(&first), first_block);
    assert_eq!(block(&second), second_block);
    let reversed = history(&second, &first);
    assert_eq!(reversed["reversed"], true, "{reversed}");
    assert_eq!(reversed["block_id"], second_block);
    assert_eq!(reversed["session_id"], second);
    // Reversal alone does not select in Rexia; a real Select reports the pane
    // despite its unchanged view focus, and history then reverses to the other.
    select(&host, &second);
    let previous = history(&first, &second);
    assert_eq!(previous["reversed"], true, "{previous}");
    assert_eq!(previous["block_id"], first_block);
    host.send(HostCommand::Shutdown);
}

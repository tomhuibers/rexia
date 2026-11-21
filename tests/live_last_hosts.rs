//! Two Rexia host connections are distinct last-session namespaces.
mod common;

use common::{Harness, Sessions};
use rexia::settings::HostConfig;
use serde_json::json;

#[test]
fn switching_hosts_keeps_each_clients_host_local_last() {
    let Some(address) = common::server() else {
        return;
    };
    let control = common::connect(&address);
    let mut sessions = Sessions::create(&control, "host-last-A");
    let a = sessions.ids[0].clone();
    let b = sessions.add("host-last-B");
    let mut settings = common::test_settings(&address, &a);
    settings.hosts.push(HostConfig {
        id: "second".into(),
        label: "second".into(),
        endpoint: address.clone(),
    });
    let mut h = Harness::open_with(settings, 1200., 760.);
    let first = h.host.clone();
    h.wait("first A", |m| {
        (m.selected.as_ref() == Some(&a) && m.view().is_some()).then_some(())
    });
    h.run("session.select", json!({"session_id": b}));
    h.wait("first B", |m| {
        (m.selected.as_ref() == Some(&b)).then_some(())
    });
    assert_eq!(first.snapshot().last_selected.as_deref(), Some(a.as_str()));
    h.run("client.host.switch", json!({"host": "second"}));
    let second = h.workspace(|ws| ws.host().handle.clone());
    assert!(!std::sync::Arc::ptr_eq(&first.model, &second.model));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while second.snapshot().selected.is_none() || second.snapshot().sessions.len() < 2 {
        assert!(
            std::time::Instant::now() < deadline,
            "second host did not connect"
        );
        h.pump();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let initial = second.snapshot().selected.unwrap();
    let target = if initial == a { &b } else { &a };
    h.run("session.select", json!({"session_id": target}));
    let target = target.clone();
    let initial_for_wait = initial.clone();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while second.snapshot().selected.as_ref() != Some(&target) {
        assert!(
            std::time::Instant::now() < deadline,
            "second host did not select target"
        );
        h.pump();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(
        second.snapshot().last_selected.as_deref(),
        Some(initial_for_wait.as_str())
    );
    h.run("session.last", json!({}));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while second.snapshot().selected.as_ref() != Some(&initial) {
        assert!(
            std::time::Instant::now() < deadline,
            "second host did not toggle"
        );
        h.pump();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(first.snapshot().last_selected.as_deref(), Some(a.as_str()));
    h.run("client.host.switch", json!({"host": "test"}));
    h.run("session.last", json!({}));
    h.wait("first host returns A", |m| {
        (m.selected.as_ref() == Some(&a)).then_some(())
    });
}

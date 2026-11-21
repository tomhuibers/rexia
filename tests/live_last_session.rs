//! Real Workspace + HostHandle + isolated Rex server. Set REXIA_TEST_SERVER.
mod common;

use common::{Harness, Sessions};
use rexia::rex::host::HostCommand;
use serde_json::json;

#[test]
fn per_client_last_session_tracks_actual_selection() {
    let Some(address) = common::server() else {
        return;
    };
    let control = common::connect(&address);
    let mut sessions = Sessions::create(&control, "last-A");
    let a = sessions.ids[0].clone();
    let b = sessions.add("last-B");
    let c = sessions.add("last-C");
    let mut one = Harness::open(&address, &a, 1200., 760.);
    let mut two = Harness::open(&address, &a, 1200., 760.);
    one.wait("A attached", |m| {
        (m.selected.as_ref() == Some(&a) && m.view().is_some()).then_some(())
    });
    two.wait("other client attached", |m| {
        (m.selected.as_ref() == Some(&a) && m.view().is_some()).then_some(())
    });
    assert!(one.host.snapshot().last_selected.is_none());
    let window = one.window;
    let missing_last = one.cx.update(|cx| {
        window
            .update(cx, |ws, window, cx| {
                ws.run_action(
                    "session.recent",
                    json!({}),
                    "palette",
                    Default::default(),
                    window,
                    cx,
                )
            })
            .unwrap()
    });
    assert!(missing_last.is_err()); // no prior destination
    assert_eq!(one.host.snapshot().selected.as_deref(), Some(a.as_str()));
    one.run("session.select", json!({"session_id": b}));
    one.wait("B", |m| (m.selected.as_ref() == Some(&b)).then_some(()));
    assert_eq!(
        one.host.snapshot().last_selected.as_deref(),
        Some(a.as_str())
    );
    one.run("session.recent", json!({}));
    one.wait("back to A", |m| {
        (m.selected.as_ref() == Some(&a)).then_some(())
    });
    // The legacy alias toggles the very same register back.
    one.run("session.last", json!({}));
    one.wait("back to B", |m| {
        (m.selected.as_ref() == Some(&b)).then_some(())
    });
    // Picker delivery invokes session.select via client.perform on this exact client.
    let client_id = one.host.snapshot().client_id.unwrap();
    let selected = one.call_from(
        &control,
        "client.perform",
        json!({
            "client_id": client_id, "action": "session.select", "args": {"session_id": c}
        }),
    );
    assert!(selected.is_ok(), "client.perform: {selected:?}");
    one.wait("picker selected existing C", |m| {
        (m.selected.as_ref() == Some(&c)).then_some(())
    });
    assert_eq!(
        one.host.snapshot().last_selected.as_deref(),
        Some(b.as_str())
    );
    one.run("session.recent", json!({}));
    one.wait("C to B", |m| {
        (m.selected.as_ref() == Some(&b)).then_some(())
    });
    one.run("session.recent", json!({}));
    one.wait("B to C", |m| {
        (m.selected.as_ref() == Some(&c)).then_some(())
    });
    // Same-session, failed target and a different client do not change this register.
    one.run("session.select", json!({"session_id": c}));
    let missing_target = one.cx.update(|cx| {
        window
            .update(cx, |ws, window, cx| {
                ws.run_action(
                    "session.select",
                    json!({"session_id": "session:missing"}),
                    "palette",
                    Default::default(),
                    window,
                    cx,
                )
            })
            .unwrap()
    });
    assert!(missing_target.is_err());
    assert_eq!(
        one.host.snapshot().last_selected.as_deref(),
        Some(b.as_str())
    );
    assert_eq!(two.host.snapshot().last_selected, None);
    two.run("session.select", json!({"session_id": b}));
    two.wait("other client B", |m| {
        (m.selected.as_ref() == Some(&b)).then_some(())
    });
    assert_eq!(
        one.host.snapshot().last_selected.as_deref(),
        Some(b.as_str())
    );
    let before_reconnect = one.host.snapshot().epoch;
    one.host.send(HostCommand::Reconnect);
    one.wait("same client reconnected", |m| {
        (m.epoch > before_reconnect && m.selected.as_ref() == Some(&c)).then_some(())
    });
    assert_eq!(
        one.host.snapshot().last_selected.as_deref(),
        Some(b.as_str())
    );
    // Destroying only the previous session leaves a stale register. l must
    // refuse it rather than searching farther back through a shared MRU list.
    common::request(&control, "session.destroy", json!({"session_id": b})).unwrap();
    one.wait("B removed", |m| {
        (!m.sessions.iter().any(|s| s.session_id == b)).then_some(())
    });
    let stale_last = one.cx.update(|cx| {
        window
            .update(cx, |ws, window, cx| {
                ws.run_action(
                    "session.recent",
                    json!({}),
                    "palette",
                    Default::default(),
                    window,
                    cx,
                )
            })
            .unwrap()
    });
    assert!(stale_last.is_err());
    assert_eq!(one.host.snapshot().selected.as_deref(), Some(c.as_str()));
    // Destroying this client's current session resets last; implicit fallback
    // must not point back to a destroyed session.
    let _ = common::request(&control, "session.destroy", json!({"session_id": c})).unwrap();
    one.wait("C removed", |m| {
        (!m.sessions.iter().any(|s| s.session_id == c)).then_some(())
    });
    assert_eq!(one.host.snapshot().last_selected, None);
    let missing_after_close = one.cx.update(|cx| {
        window
            .update(cx, |ws, window, cx| {
                ws.run_action(
                    "session.recent",
                    json!({}),
                    "palette",
                    Default::default(),
                    window,
                    cx,
                )
            })
            .unwrap()
    });
    assert!(missing_after_close.is_err());
    assert_ne!(one.host.snapshot().selected.as_deref(), Some(c.as_str()));
    // A newly created session is selected by the creating client only.
    one.host.send(HostCommand::CreateSession {
        label: Some("last-new".into()),
        reply: None,
    });
    let new = one.wait("new selected", |m| {
        m.selected
            .as_ref()
            .filter(|id| *id != &a && *id != &b && *id != &c)
            .cloned()
    });
    assert_ne!(two.host.snapshot().selected.as_deref(), Some(new.as_str()));
    sessions.track(new);
    assert!(one.host.snapshot().last_selected.is_some());
}

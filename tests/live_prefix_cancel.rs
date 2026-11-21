//! Opt-in regression: use a disposable server with tests/fixtures/init.lua.
mod common;

use common::{Harness, Sessions, focused_text};

#[test]
fn escape_cancels_prefix_without_reaching_the_terminal() {
    let Some(address) = common::server() else {
        return;
    };
    let control = common::connect(&address);
    let sessions = Sessions::create(&control, "rexia-prefix-cancel");
    let mut h = Harness::open(&address, &sessions.ids[0], 1200., 760.);
    h.wait("prompt and bindings", |m| {
        (m.keymap.is_some() && !focused_text(m).trim().is_empty()).then_some(())
    });

    // Read exactly two raw bytes. A cancelled prefix must not contribute a byte;
    // a subsequent ordinary Escape must still reach the application.
    h.type_text("sh -c 'saved=$(stty -g); stty -echo -icanon; printf \"\\nPREFIX-READY\\n\"; od -An -tx1 -N 2; stty \"$saved\"'");
    h.press("enter");
    h.wait("raw reader", |m| {
        focused_text(m)
            .lines()
            .any(|line| line.trim() == "PREFIX-READY")
            .then_some(())
    });
    h.press("ctrl-a");
    assert_eq!(h.workspace(|ws| ws.pending_keys()), 1);
    h.press("escape");
    assert_eq!(h.workspace(|ws| ws.pending_keys()), 0);
    h.press("x escape");
    h.wait("only x and ordinary Escape to reach the terminal", |m| {
        focused_text(m)
            .lines()
            .any(|line| line.trim() == "78 1b")
            .then_some(())
    });
}

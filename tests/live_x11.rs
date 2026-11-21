//! Opt-in native X11 regressions. See tests/fixtures/native_x11.py for dependencies.
use std::process::Command;

fn native(case: &str) {
    if std::env::var_os("REXIA_TEST_NATIVE_X11").is_none() {
        eprintln!("skipped: set REXIA_TEST_NATIVE_X11=1 to run with Xvfb and XTest");
        return;
    }
    let status = Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/native_x11.py"
        ))
        .arg(env!("CARGO_BIN_EXE_rexia"))
        .arg(case)
        .status()
        .expect("launch the native X11 regression");
    assert!(status.success(), "native X11 {case} regression failed");
}

#[test]
fn activation_click_focuses_an_already_focused_terminal_without_forwarding() {
    native("focus");
}

#[test]
fn cold_tab_draws_all_its_terminals_with_its_first_layout() {
    native("tabs");
}

#[test]
fn cold_tab_waits_for_a_healthy_slow_stream() {
    native("slow");
}

#[test]
fn cold_tab_waits_through_an_automatic_retry() {
    native("retry");
}

#[test]
fn escape_cancels_a_pending_tab_switch() {
    native("cancel");
}

#[test]
fn closing_a_tab_waits_for_its_cold_successor() {
    native("close");
}

#[test]
fn another_tab_can_be_selected_while_the_successor_waits() {
    native("close-select");
}

#[test]
fn jumping_sessions_waits_for_both_cold_terminal_frames() {
    native("cross-session");
}

#[test]
fn native_client_last_session_toggles_after_picker_jump() {
    native("last-session");
}

#[test]
fn floating_divider_stops_at_the_header_width() {
    native("floating-resize");
}

# Recent-session toggle

`session.recent` (Go to Recent Session) returns to the previous session viewed
by this Rexia client on the active host. Repeating it toggles between the two
sessions. `session.last` remains a compatibility alias using the same state and
dispatch path. Neither action has a default shortcut.

For a tmux-style binding in Rex's `init.lua`:

```lua
rex.bind("ctrl+a>l", "session.recent")
```

Restart into a Rexia build supporting `session.recent` before adopting that
binding. Older builds implementing only `session.last` must retain the old name
until upgraded. The alias supports old configurations on new clients, not new
configurations on old clients.

## Semantics

- Each client/host connection owns one previous-session slot. Other clients and
  switching hosts do not overwrite it. A new client starts without history.
- Successful session changes, including native pickers, `session.select` from
  a script/project picker, and creating a session, update the slot.
- Selecting the current session or failing to select a destination does not
  replace history. There is no traversal through older sessions.
- A missing or deleted previous session produces a notice rather than selecting
  a different historical session. Deleting the current session clears the slot;
  its automatic fallback is not a deliberate switch.
- Reconnecting the same client preserves history. Offline requests commit a
  change only after bootstrap validates the destination and any required attach
  succeeds; an unavailable request falls back without inventing history.

This uses no Rex server or protocol changes and does not call the shared
`focus_history.reverse` operation.

## Upstream compatibility

The official [Rex for Mac action reference](https://www.superlogical.com/rex/docs/reference/mac-actions#action-session-recent)
describes `session.recent` as: “Shows the session you were in before this one.
Use it again to return.” The name, title, absence of a default binding, and
A → B → A → B toggle agree with that documented contract. The separate
`session.back` action is documented to keep going back; it is not implemented
by this toggle.

The reference does not specify client/host isolation, deleted-session behavior,
persistence, reconnects, or offline requests. The rules above describe Rexia,
not verified macOS internals. No macOS source implementation or running macOS
client was available for this comparison; full edge-case parity remains
unverified. `session.last` is a Rexia compatibility extension, not an action
listed in the Mac reference.

## Regression coverage

Library tests cover registration of both names, custom bindings, the history
register, and offline reconciliation. `tests/live_last_session.rs` exercises
both dispatch names, script-driven selection, independent clients, reconnects,
deletion, and creation against a disposable Rex server. `tests/live_last_hosts.rs`
checks host-local isolation. Set `REXIA_TEST_SERVER` to that disposable server
when running those live tests.

The self-contained `last-session` case in `tests/fixtures/native_x11.py` checks
real native-picker selection and prefix-key toggling with Xvfb/XTest. It does
not require an external Rexconfig package or project-picker setup.

//! The actions rexia performs itself, named as Rex for Mac names them so init.lua bindings,
//! `rex do` and `client.perform` work the same, with Linux default keys.
use serde_json::{Value, json};

pub struct ActionDef {
    pub name: &'static str,
    pub title: &'static str,
    pub category: &'static str,
    pub keywords: &'static str,
    /// Default bindings: keys (one per stroke of a sequence) and fixed arguments.
    pub keys: &'static [(&'static [&'static str], Option<&'static str>)],
    pub mode: Option<&'static str>,
    /// A JSON Schema of the arguments, when the action takes any.
    pub args: Option<&'static str>,
    pub repeats: bool,
}

const DIRECTION: &str = r#"{"type":"object","properties":{"direction":{"type":"string","enum":["left","right","up","down"]}},"required":["direction"]}"#;
const SPLIT: &str = r#"{"type":"object","properties":{"direction":{"type":"string","enum":["left","right","up","down","auto"]}}}"#;
const RESIZE: &str = r#"{"type":"object","properties":{"direction":{"type":"string","enum":["left","right","up","down"]},"amount":{"type":"number"}},"required":["direction"]}"#;
const INDEX: &str = r#"{"type":"object","properties":{"index":{"type":"integer","minimum":1}},"required":["index"]}"#;
const SESSION: &str =
    r#"{"type":"object","properties":{"session_id":{"type":"string"}},"required":["session_id"]}"#;
const MODE: &str =
    r#"{"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}"#;
const KEY: &str = r#"{"type":"object","properties":{"key":{"type":"string"}},"required":["key"]}"#;
const HOST: &str = r#"{"type":"object","properties":{"host":{"type":"string"}}}"#;
const MUTE_TAB: &str =
    r#"{"type":"object","properties":{"window_id":{"type":"string"},"muted":{"type":"boolean"}}}"#;
const COPY_MOTION: &str = r#"{"type":"object","properties":{"motion":{"type":"string","enum":["left","right","up","down","word_forward","word_backward","word_end","big_word_forward","big_word_backward","big_word_end","line_start","first_non_blank","line_end","viewport_top","viewport_middle","viewport_bottom","half_page_up","half_page_down","page_up","page_down","history_top","history_bottom","paragraph_up","paragraph_down"]}},"required":["motion"]}"#;
const COPY_SELECTION: &str = r#"{"type":"object","properties":{"kind":{"type":"string","enum":["character","line","block"]}},"required":["kind"]}"#;
const COPY_COUNT: &str = r#"{"type":"object","properties":{"digit":{"type":"integer","minimum":0,"maximum":9}},"required":["digit"]}"#;

macro_rules! copy_binding {
    ($key:literal, $arg:literal, $value:literal) => {
        (&[$key], Some(concat!("{\"", $arg, "\":\"", $value, "\"}")))
    };
}

macro_rules! copy_digit {
    ($digit:literal) => {
        (
            &[stringify!($digit)],
            Some(concat!("{\"digit\":", stringify!($digit), "}")),
        )
    };
}

macro_rules! action {
    ($name:literal, $title:literal, $category:literal, $keywords:literal, [$($keys:expr),* $(,)?] $(, args = $args:expr)? $(, repeats = $repeats:expr)? $(, mode = $mode:literal)?) => {
        ActionDef {
            name: $name,
            title: $title,
            category: $category,
            keywords: $keywords,
            keys: &[$($keys),*],
            mode: [None $(, Some($mode))?].last().copied().flatten(),
            args: { #[allow(unused_mut, unused_assignments)] let mut a = None; $(a = Some($args);)? a },
            repeats: { #[allow(unused_mut, unused_assignments)] let mut r = false; $(r = $repeats;)? r },
        }
    };
}

pub const ACTIONS: &[ActionDef] = &[
    // Window and app
    action!(
        "client.close",
        "Close",
        "Window",
        "close pane tab window focused",
        [(&["ctrl+shift+w"], None)]
    ),
    action!(
        "client.window.fullscreen.toggle",
        "Toggle Full Screen",
        "Window",
        "full screen fullscreen",
        [(&["f11"], None)]
    ),
    action!(
        "client.version.copy",
        "Copy Version Info",
        "App",
        "version build copy clipboard",
        []
    ),
    action!(
        "client.debug.copy",
        "Copy Debug Info",
        "App",
        "debug report diagnostics copy clipboard",
        []
    ),
    action!(
        "client.notice.toggle_details",
        "Toggle Notice Details",
        "App",
        "expand collapse notice banner",
        []
    ),
    action!(
        "client.notice.dismiss",
        "Dismiss Notice",
        "App",
        "dismiss notice banner",
        [(&["ctrl+alt+shift+."], None)]
    ),
    action!(
        "client.config.copy_path",
        "Copy Config File Path",
        "View",
        "config init.lua path copy",
        []
    ),
    action!(
        "client.config.reload",
        "Reload Config",
        "View",
        "config init.lua reload keybindings",
        []
    ),
    action!(
        "client.palette.toggle",
        "Command Palette…",
        "App",
        "command palette search commands actions",
        [(&["ctrl+shift+p"], None)]
    ),
    action!(
        "client.settings",
        "Settings…",
        "App",
        "preferences settings options configure",
        [(&["ctrl+,"], None)]
    ),
    action!(
        "client.key.forward",
        "Send Key to Server",
        "App",
        "send key forward server script lua",
        []
    ),
    action!(
        "client.server.restart",
        "Restart Server…",
        "App",
        "restart server stop start local rex",
        []
    ),
    action!(
        "client.cursor_shader.toggle",
        "Enable Cursor Shaders",
        "View",
        "cursor shader trail effect animation smear glow warp sweep comet blaze tapered rainbow gradient frozen enable disable toggle",
        []
    ),
    action!(
        "client.cursor_shader.change",
        "Change Cursor Shader…",
        "View",
        "cursor shader trail effect choose pick warp sweep comet blaze tapered rainbow gradient frozen",
        []
    ),
    action!(
        "client.onboarding",
        "Open Onboarding Session…",
        "App",
        "onboarding welcome setup get started tour walkthrough migrate keybindings theme appearance tmux zellij ghostty agent",
        []
    ),
    action!(
        "client.quick_look",
        "Quick Look",
        "View",
        "quick look preview file image link path url web pointer hover",
        [(&["ctrl+shift+y"], None)]
    ),
    action!(
        "client.agents.setup",
        "Install Rex for Agents",
        "App",
        "rex skill install agent agents coding ai claude code codex opencode pi hook hooks setup",
        []
    ),
    action!(
        "client.shortcuts.edit",
        "Change Shortcut…",
        "App",
        "change shortcut key binding rebind keyboard",
        []
    ),
    action!(
        "client.visibility.toggle",
        "Toggle Visibility",
        "Window",
        "show hide visibility toggle front minimize",
        []
    ),
    action!(
        "client.diagnostics",
        "Rex Diagnostics",
        "App",
        "diagnostics logs debug troubleshoot report keymap",
        []
    ),
    action!(
        "client.find.open",
        "Find…",
        "Find",
        "find search text scrollback highlight",
        [(&["ctrl+shift+f"], None)]
    ),
    action!(
        "client.find.next",
        "Find Next",
        "Find",
        "find search next match",
        [(&["f3"], None)]
    ),
    action!(
        "client.find.previous",
        "Find Previous",
        "Find",
        "find search previous match",
        [(&["shift+f3"], None)]
    ),
    action!(
        "client.mode.enter",
        "Enter Mode",
        "Keys",
        "mode key table prefix",
        [],
        args = MODE
    ),
    action!(
        "client.mode.exit",
        "Exit Mode",
        "Keys",
        "mode exit leave",
        []
    ),
    action!(
        "client.mode.exit_all",
        "Exit All Modes",
        "Keys",
        "mode exit leave all",
        []
    ),
    // Sessions and hosts
    action!(
        "session.new",
        "New Session",
        "Session",
        "create session add",
        [(&["ctrl+alt+shift+n"], None)]
    ),
    action!(
        "session.switch",
        "Change Session…",
        "Session",
        "session switch jump go",
        [(&["ctrl+shift+k"], None)]
    ),
    action!(
        "session.rename",
        "Rename Session",
        "Session",
        "rename session name title",
        []
    ),
    action!(
        "session.close",
        "Close Session",
        "Session",
        "close kill destroy delete session",
        []
    ),
    action!(
        "session.select",
        "Go to Session",
        "Session",
        "session select show",
        [],
        args = SESSION
    ),
    action!(
        "session.next",
        "Next Session",
        "Session",
        "cycle session forward",
        [(&["ctrl+alt+shift+down"], None)]
    ),
    action!(
        "session.previous",
        "Previous Session",
        "Session",
        "cycle session back",
        [(&["ctrl+alt+shift+up"], None)]
    ),
    action!(
        "session.last",
        "Last Session",
        "Session",
        "return to last viewed session toggle tmux",
        []
    ),
    action!(
        "session.refresh",
        "Refresh Sessions",
        "Session",
        "reload sync sessions reconnect",
        [(&["ctrl+shift+r"], None)]
    ),
    action!(
        "session.collapse",
        "Collapse Session",
        "Session",
        "collapse fold sidebar tabs",
        []
    ),
    action!(
        "session.expand",
        "Expand Session",
        "Session",
        "expand unfold sidebar tabs",
        []
    ),
    action!(
        "session.collapse_others",
        "Collapse Other Sessions",
        "Session",
        "collapse other sessions sidebar",
        []
    ),
    action!(
        "client.host.switch",
        "Switch Host",
        "Host",
        "host remote machine server switch",
        [],
        args = HOST
    ),
    action!(
        "client.host.add",
        "Add Remote Host…",
        "Host",
        "host remote add server machine url https connect",
        []
    ),
    action!(
        "client.host.remove",
        "Remove Host…",
        "Host",
        "remove forget delete host remote server",
        []
    ),
    action!(
        "client.host.forward_port",
        "Forward Port…",
        "Host",
        "port forward forwarding tunnel localhost remote browser",
        []
    ),
    // Tabs
    action!(
        "client.tab.new",
        "New Tab",
        "Tab",
        "create tab window add",
        [(&["ctrl+shift+t"], None)]
    ),
    action!(
        "client.tab.close",
        "Close Tab",
        "Tab",
        "close tab window",
        [(&["ctrl+alt+w"], None)]
    ),
    action!(
        "client.tab.rename",
        "Rename Tab",
        "Tab",
        "rename tab window name title",
        []
    ),
    action!(
        "client.tab.mute",
        "Mute Tab",
        "Tab",
        "mute unmute silence quiet notifications sounds badge tab",
        [],
        args = MUTE_TAB
    ),
    action!(
        "client.tab.close_others",
        "Close Other Tabs",
        "Tab",
        "close other tabs keep",
        []
    ),
    action!(
        "client.tab.next",
        "Show Next Tab",
        "Tab",
        "cycle tab forward",
        [(&["ctrl+tab"], None), (&["ctrl+pagedown"], None)]
    ),
    action!(
        "client.tab.previous",
        "Show Previous Tab",
        "Tab",
        "cycle tab back",
        [(&["ctrl+shift+tab"], None), (&["ctrl+pageup"], None)]
    ),
    action!(
        "client.tab.goto",
        "Go to Tab",
        "Tab",
        "tab select number index",
        [
            (&["alt+1"], Some(r#"{"index":1}"#)),
            (&["alt+2"], Some(r#"{"index":2}"#)),
            (&["alt+3"], Some(r#"{"index":3}"#)),
            (&["alt+4"], Some(r#"{"index":4}"#)),
            (&["alt+5"], Some(r#"{"index":5}"#)),
            (&["alt+6"], Some(r#"{"index":6}"#)),
            (&["alt+7"], Some(r#"{"index":7}"#)),
            (&["alt+8"], Some(r#"{"index":8}"#)),
            (&["alt+9"], Some(r#"{"index":9}"#)),
        ],
        args = INDEX
    ),
    action!(
        "client.tab.move",
        "Move Tab",
        "Tab",
        "move tab reorder",
        [],
        args = INDEX
    ),
    action!(
        "client.tab.move.backward",
        "Move Tab Backward",
        "Tab",
        "move tab reorder left earlier",
        [(&["ctrl+shift+pageup"], None)]
    ),
    action!(
        "client.tab.move.forward",
        "Move Tab Forward",
        "Tab",
        "move tab reorder right later",
        [(&["ctrl+shift+pagedown"], None)]
    ),
    // Panes
    action!(
        "pane.split",
        "Split Pane",
        "Pane",
        "pane split divide new",
        [],
        args = SPLIT
    ),
    action!(
        "pane.split.left",
        "Split Pane Left",
        "Pane",
        "pane split left",
        []
    ),
    action!(
        "pane.split.right",
        "Split Pane Right",
        "Pane",
        "pane split right",
        [(&["ctrl+shift+o"], None)]
    ),
    action!(
        "pane.split.up",
        "Split Pane Up",
        "Pane",
        "pane split above up",
        []
    ),
    action!(
        "pane.split.down",
        "Split Pane Down",
        "Pane",
        "pane split below down",
        [(&["ctrl+shift+e"], None)]
    ),
    action!(
        "pane.split.auto",
        "Split Pane Best Fit",
        "Pane",
        "pane split automatic best fit",
        [(&["ctrl+alt+enter"], None)]
    ),
    action!(
        "pane.focus",
        "Focus Pane",
        "Pane",
        "pane focus navigate",
        [],
        args = DIRECTION
    ),
    action!(
        "pane.focus.left",
        "Focus Pane Left",
        "Pane",
        "pane focus left",
        [(&["ctrl+alt+left"], None)]
    ),
    action!(
        "pane.focus.right",
        "Focus Pane Right",
        "Pane",
        "pane focus right",
        [(&["ctrl+alt+right"], None)]
    ),
    action!(
        "pane.focus.up",
        "Focus Pane Up",
        "Pane",
        "pane focus up above",
        [(&["ctrl+alt+up"], None)]
    ),
    action!(
        "pane.focus.down",
        "Focus Pane Down",
        "Pane",
        "pane focus down below",
        [(&["ctrl+alt+down"], None)]
    ),
    action!(
        "pane.focus_next",
        "Focus Next Pane",
        "Pane",
        "pane focus next",
        [(&["ctrl+shift+]"], None)]
    ),
    action!(
        "pane.focus_previous",
        "Focus Previous Pane",
        "Pane",
        "pane focus previous",
        [(&["ctrl+shift+["], None)]
    ),
    action!(
        "pane.move",
        "Move Pane",
        "Pane",
        "pane move swap",
        [],
        args = DIRECTION
    ),
    action!(
        "pane.move.left",
        "Move Pane Left",
        "Pane",
        "pane move swap left",
        [(&["ctrl+alt+shift+left"], None)]
    ),
    action!(
        "pane.move.right",
        "Move Pane Right",
        "Pane",
        "pane move swap right",
        [(&["ctrl+alt+shift+right"], None)]
    ),
    action!(
        "pane.move.up",
        "Move Pane Up",
        "Pane",
        "pane move swap up",
        []
    ),
    action!(
        "pane.move.down",
        "Move Pane Down",
        "Pane",
        "pane move swap down",
        []
    ),
    action!(
        "pane.float",
        "Float Pane",
        "Pane",
        "pane float dock detach",
        [(&["ctrl+alt+shift+f"], None)]
    ),
    action!(
        "pane.new_floating",
        "New Floating Pane",
        "Pane",
        "pane new floating scratchpad",
        []
    ),
    action!(
        "pane.minimize_floating",
        "Minimize Floating Window",
        "Pane",
        "floating minimize restore",
        [(&["ctrl+alt+m"], None)]
    ),
    action!(
        "pane.minimize_all_floating",
        "Minimize Floating Windows",
        "Pane",
        "floating all minimize restore",
        [(&["ctrl+alt+shift+m"], None)]
    ),
    action!(
        "pane.zoom",
        "Zoom Pane",
        "Pane",
        "pane zoom maximize restore",
        [(&["ctrl+shift+enter"], None)]
    ),
    action!("pane.close", "Close Pane", "Pane", "pane close remove", []),
    action!(
        "pane.close_others",
        "Close Other Panes",
        "Pane",
        "close other panes keep focused",
        []
    ),
    action!(
        "pane.balance",
        "Balance Splits",
        "Pane",
        "balance equalize evenly",
        []
    ),
    action!(
        "pane.resize",
        "Resize Pane",
        "Pane",
        "pane resize grow shrink divider",
        [],
        args = RESIZE,
        repeats = true
    ),
    action!(
        "pane.move_to_new_tab",
        "Move Pane to New Tab",
        "Pane",
        "pane move new tab detach",
        []
    ),
    action!(
        "pane.move_to_tab",
        "Move Pane to Tab…",
        "Pane",
        "pane move tab send other existing join",
        []
    ),
    action!(
        "pane.go_to_directory",
        "Go to Directory…",
        "Pane",
        "go to directory folder path cd new tab",
        [(&["ctrl+alt+g"], None)]
    ),
    action!(
        "pane.floating_actions",
        "Floating Pane Actions…",
        "Pane",
        "floating pane controls menu minimize split zoom dock close",
        []
    ),
    action!(
        "pane.go_to",
        "Go to Anything…",
        "Pane",
        "go to anything pane tab session jump switch find",
        [(&["ctrl+shift+space"], None)]
    ),
    action!(
        "pane.view_activity",
        "View Activity…",
        "Pane",
        "view activity working blocked failed done agents status",
        []
    ),
    action!(
        "pane.insert_path",
        "Insert Path…",
        "Pane",
        "insert path file folder directory cursor type",
        [(&["ctrl+alt+i"], None)]
    ),
    action!(
        "pane.insert_file",
        "Insert File…",
        "Pane",
        "insert file document attach upload paste",
        []
    ),
    action!(
        "pane.insert_photo",
        "Insert Photo…",
        "Pane",
        "insert photo image picture screenshot attach upload paste",
        []
    ),
    action!(
        "pane.rename",
        "Rename Pane",
        "Pane",
        "rename pane block name title label",
        []
    ),
    action!(
        "pane.copy_id",
        "Copy Pane ID",
        "Pane",
        "pane block id copy",
        []
    ),
    action!(
        "pane.paste",
        "Paste into Pane",
        "Pane",
        "paste clipboard",
        [(&["ctrl+shift+v"], None), (&["shift+insert"], None)]
    ),
    action!(
        "pane.copy",
        "Copy from Pane",
        "Pane",
        "copy selection clipboard",
        [(&["ctrl+shift+c"], None)]
    ),
    action!(
        "pane.copy_mode.enter",
        "Enter Copy Mode",
        "Pane",
        "copy mode vim keyboard select scrollback",
        [(&["ctrl+alt+c"], None)]
    ),
    action!(
        "pane.copy_mode.exit",
        "Exit Copy Mode",
        "Pane",
        "copy mode leave escape",
        [(&["esc"], None), (&["q"], None)],
        mode = "copy"
    ),
    action!(
        "pane.copy_mode.move",
        "Move Copy Mode Cursor",
        "Pane",
        "copy mode cursor vim motion scrollback",
        [
            copy_binding!("h", "motion", "left"),
            copy_binding!("left", "motion", "left"),
            copy_binding!("l", "motion", "right"),
            copy_binding!("right", "motion", "right"),
            copy_binding!("k", "motion", "up"),
            copy_binding!("up", "motion", "up"),
            copy_binding!("j", "motion", "down"),
            copy_binding!("down", "motion", "down"),
            copy_binding!("w", "motion", "word_forward"),
            copy_binding!("b", "motion", "word_backward"),
            copy_binding!("e", "motion", "word_end"),
            copy_binding!("shift+w", "motion", "big_word_forward"),
            copy_binding!("shift+b", "motion", "big_word_backward"),
            copy_binding!("shift+e", "motion", "big_word_end"),
            copy_binding!("home", "motion", "line_start"),
            copy_binding!("shift+6", "motion", "first_non_blank"),
            copy_binding!("shift+4", "motion", "line_end"),
            copy_binding!("end", "motion", "line_end"),
            copy_binding!("shift+h", "motion", "viewport_top"),
            copy_binding!("shift+m", "motion", "viewport_middle"),
            copy_binding!("shift+l", "motion", "viewport_bottom"),
            copy_binding!("ctrl+u", "motion", "half_page_up"),
            copy_binding!("ctrl+d", "motion", "half_page_down"),
            copy_binding!("ctrl+b", "motion", "page_up"),
            copy_binding!("pageup", "motion", "page_up"),
            copy_binding!("ctrl+f", "motion", "page_down"),
            copy_binding!("pagedown", "motion", "page_down"),
            (&["g", "g"], Some(r#"{"motion":"history_top"}"#)),
            copy_binding!("shift+g", "motion", "history_bottom"),
            copy_binding!("shift+[", "motion", "paragraph_up"),
            copy_binding!("shift+]", "motion", "paragraph_down"),
        ],
        args = COPY_MOTION,
        repeats = true,
        mode = "copy"
    ),
    action!(
        "pane.copy_mode.select",
        "Select in Copy Mode",
        "Pane",
        "copy mode visual character line block selection",
        [
            copy_binding!("v", "kind", "character"),
            copy_binding!("shift+v", "kind", "line"),
            copy_binding!("ctrl+v", "kind", "block"),
        ],
        args = COPY_SELECTION,
        mode = "copy"
    ),
    action!(
        "pane.copy_mode.swap_ends",
        "Swap Copy Mode Selection Ends",
        "Pane",
        "copy mode visual selection anchor",
        [(&["o"], None)],
        mode = "copy"
    ),
    action!(
        "pane.copy_mode.yank",
        "Yank Copy Mode Selection",
        "Pane",
        "copy mode selection clipboard",
        [(&["y"], None), (&["enter"], None)],
        mode = "copy"
    ),
    action!(
        "pane.copy_mode.copy_to_line_end",
        "Yank to End of Line in Copy Mode",
        "Pane",
        "copy mode line clipboard",
        [(&["shift+d"], None)],
        mode = "copy"
    ),
    action!(
        "pane.copy_mode.count",
        "Type Copy Mode Count",
        "Pane",
        "copy mode count repeat prefix",
        [
            copy_digit!(0),
            copy_digit!(1),
            copy_digit!(2),
            copy_digit!(3),
            copy_digit!(4),
            copy_digit!(5),
            copy_digit!(6),
            copy_digit!(7),
            copy_digit!(8),
            copy_digit!(9),
        ],
        args = COPY_COUNT,
        mode = "copy"
    ),
    action!(
        "pane.select_all",
        "Select All in Pane",
        "Pane",
        "select all scrollback",
        [(&["ctrl+shift+a"], None)]
    ),
    action!(
        "pane.send_key",
        "Send Key",
        "Pane",
        "send key terminal press",
        [],
        args = KEY
    ),
    action!(
        "client.prompt.previous",
        "Jump to Previous Prompt",
        "Pane",
        "jump previous prompt",
        [(&["ctrl+shift+up"], None)]
    ),
    action!(
        "client.prompt.next",
        "Jump to Next Prompt",
        "Pane",
        "jump next prompt",
        [(&["ctrl+shift+down"], None)]
    ),
    action!(
        "client.scroll.page_up",
        "Scroll Page Up",
        "Pane",
        "scroll page up scrollback",
        [(&["shift+pageup"], None)],
        repeats = true
    ),
    action!(
        "client.scroll.page_down",
        "Scroll Page Down",
        "Pane",
        "scroll page down scrollback",
        [(&["shift+pagedown"], None)],
        repeats = true
    ),
    // View
    action!(
        "client.font.increase",
        "Increase Font Size",
        "View",
        "font size larger zoom",
        [
            (&["ctrl+="], None),
            (&["ctrl++"], None),
            (&["ctrl+shift+="], None)
        ],
        repeats = true
    ),
    action!(
        "client.font.decrease",
        "Decrease Font Size",
        "View",
        "font size smaller zoom",
        [(&["ctrl+-"], None)],
        repeats = true
    ),
    action!(
        "client.font.reset",
        "Reset Font Size",
        "View",
        "font size reset default",
        [(&["ctrl+0"], None)]
    ),
    action!(
        "client.sidebar.toggle",
        "Toggle Sidebar",
        "View",
        "sidebar collapse expand hide show",
        [(&["ctrl+shift+b"], None)]
    ),
    action!(
        "client.window.new",
        "New Window",
        "Window",
        "new window open another workspace",
        [(&["ctrl+shift+n"], None)]
    ),
    action!(
        "client.window.close",
        "Close Window",
        "Window",
        "close window workspace",
        [(&["ctrl+alt+shift+w"], None)]
    ),
    action!(
        "client.tab.move_to_new_window",
        "Move Tab to New Window",
        "Tab",
        "move tab detach split off new window",
        []
    ),
    action!(
        "client.toolbar.reveal",
        "Reveal Toolbar",
        "View",
        "toolbar tabs show reveal hidden hover",
        [(&["ctrl+alt+shift+t"], None)]
    ),
    action!(
        "client.pane_header.reveal",
        "Reveal Pane Header",
        "View",
        "pane header title show reveal hidden hover",
        [(&["ctrl+alt+shift+h"], None)]
    ),
    action!(
        "client.tab_style.toggle",
        "Toggle Toolbar Layout",
        "View",
        "toolbar layout tabs vertical horizontal sidebar above left right",
        [(&["ctrl+shift+s"], None)]
    ),
    action!(
        "client.sidebar.filter.focus",
        "Focus Tab Filter",
        "View",
        "filter search find type query focus keyboard sidebar tabs sessions hosts",
        []
    ),
    action!(
        "client.sidebar.activity.toggle",
        "Toggle Activity View",
        "View",
        "activity sidebar attention blocked done programs filter",
        [(&["ctrl+shift+."], None)]
    ),
];

static SETTING_ENTRIES: std::sync::LazyLock<Vec<(String, crate::settings_index::Entry)>> =
    std::sync::LazyLock::new(|| {
        crate::settings_index::entries()
            .into_iter()
            .map(|entry| {
                let suffix = if entry.path == "interface.pane_style" {
                    "interface.panes.density"
                } else {
                    &entry.path
                };
                (format!("client.setting.{suffix}"), entry)
            })
            .collect()
    });

static SETTING_ACTIONS: std::sync::LazyLock<Vec<ActionDef>> = std::sync::LazyLock::new(|| {
    SETTING_ENTRIES.iter().map(|(name, entry)| ActionDef {
        name, title: &entry.title, category: "Settings", keywords: &entry.path, keys: &[], mode: None,
        args: Some(r#"{"type":"object","properties":{"value":{},"increase":{"type":"boolean"},"decrease":{"type":"boolean"},"reset":{"type":"boolean"}}}"#),
        repeats: false,
    }).collect()
});

pub fn setting_action(path: &str) -> Option<&'static str> {
    SETTING_ENTRIES
        .iter()
        .find(|(_, entry)| entry.path == path)
        .map(|(name, _)| name.as_str())
}

pub fn setting_entry(name: &str) -> Option<&'static crate::settings_index::Entry> {
    name.starts_with("client.setting.")
        .then(|| {
            SETTING_ENTRIES
                .iter()
                .find(|(action, _)| action == name)
                .map(|(_, entry)| entry)
        })
        .flatten()
}

pub fn all() -> impl Iterator<Item = &'static ActionDef> {
    ACTIONS.iter().chain(SETTING_ACTIONS.iter())
}

pub fn find(name: &str) -> Option<&'static ActionDef> {
    let name = match name {
        "client.tab.mute.toggle" => "client.tab.mute",
        name => name,
    };
    all().find(|a| a.name == name)
}

/// The `client.keymap` request: rexia's actions and its default bindings as the bottom layer,
/// beneath init.lua and client scripts.
/// Shortcuts the user gave actions: the keys of one binding each, by action name.
pub type Shortcuts = std::collections::BTreeMap<String, Vec<String>>;

/// Whether Change Shortcut can rebind `action`: actions whose default keys carry
/// arguments, such as Go to Tab 1–9, keep them.
pub fn rebindable(action: &ActionDef) -> bool {
    action.mode.is_none() && action.keys.iter().all(|(_, args)| args.is_none())
}

/// Settings edits one in-mode default at a time, keeping that binding's arguments.
pub(crate) struct ModeShortcut {
    pub action: &'static ActionDef,
    pub keys: &'static [&'static str],
    pub args: Option<&'static str>,
    pub mode: &'static str,
}

impl ModeShortcut {
    pub fn id(&self) -> String {
        format!("{}/{}", self.action.name, self.keys.join(">"))
    }

    pub fn find(id: &str) -> Option<Self> {
        let (name, keys) = id.split_once('/')?;
        mode_shortcuts().find(|binding| {
            binding.action.name == name && binding.keys.iter().copied().eq(keys.split('>'))
        })
    }

    pub fn title(&self) -> String {
        format!(
            "{} ({})",
            self.action.title,
            crate::keymap::display(
                &self
                    .keys
                    .iter()
                    .map(|key| key.to_string())
                    .collect::<Vec<_>>()
            )
        )
    }
}

pub(crate) fn mode_shortcuts() -> impl Iterator<Item = ModeShortcut> {
    all()
        .filter_map(|action| action.mode.map(|mode| (action, mode)))
        .flat_map(|(action, mode)| {
            action.keys.iter().map(move |&(keys, args)| ModeShortcut {
                action,
                keys,
                args,
                mode,
            })
        })
}

pub fn keymap_request(shortcuts: &Shortcuts) -> Value {
    let actions: Vec<Value> = all()
        .map(|a| {
            let mut action = json!({"name": a.name, "kind": "client", "title": a.title,
                "category": a.category, "keywords": a.keywords});
            if let Some(schema) = a.args {
                action["args"] = serde_json::from_str(schema).expect("valid schema");
            }
            if a.repeats {
                action["repeats"] = json!(true);
            }
            action
        })
        .collect();
    // A key the user gave one action leaves every other action's defaults.
    let claimed: Vec<&Vec<String>> = shortcuts
        .iter()
        .filter(|(name, keys)| find(name).is_some() && !keys.is_empty())
        .map(|(_, keys)| keys)
        .collect();
    let bindings: Vec<Value> = all()
        .filter(|a| a.mode.is_none())
        .flat_map(|a| -> Vec<Value> {
            let keys = shortcuts.get(a.name).or_else(|| {
                (a.name == "client.tab.mute")
                    .then(|| shortcuts.get("client.tab.mute.toggle"))
                    .flatten()
            });
            if let Some(keys) = keys.filter(|_| rebindable(a)) {
                return if keys.is_empty() {
                    Vec::new()
                } else {
                    vec![json!({"keys": keys, "action": a.name})]
                };
            }
            a.keys
                .iter()
                .filter(|(keys, _)| !claimed.iter().any(|c| c.as_slice() == *keys))
                .map(|(keys, args)| {
                    let mut binding = json!({"keys": keys, "action": a.name});
                    if let Some(args) = args {
                        binding["args"] = serde_json::from_str(args).expect("valid args");
                    }
                    binding
                })
                .collect()
        })
        .chain(mode_shortcuts().filter_map(|binding| {
            let override_keys = shortcuts.get(&binding.id());
            if override_keys.is_some_and(Vec::is_empty)
                || (override_keys.is_none()
                    && shortcuts.iter().any(|(id, keys)| {
                        keys.as_slice() == binding.keys
                            && ModeShortcut::find(id)
                                .is_some_and(|other| other.mode == binding.mode)
                    }))
            {
                return None;
            }
            let keys = override_keys.map_or_else(|| json!(binding.keys), |keys| json!(keys));
            let mut result =
                json!({"keys": keys, "action": binding.action.name, "mode": binding.mode});
            if let Some(args) = binding.args {
                result["args"] = serde_json::from_str(args).expect("valid args");
            }
            Some(result)
        }))
        .collect();
    json!({"actions": actions, "defaults": [{"name": "rexia", "bindings": bindings,
        "modes": [{"name": "copy", "exclusive": true}]}]})
}

/// rexia's default bindings alone, for before the server's keymap arrives.
pub fn fallback_keymap(shortcuts: &Shortcuts) -> crate::keymap::Keymap {
    let request = keymap_request(shortcuts);
    crate::keymap::Keymap::parse(&json!({
        "bindings": request["defaults"][0]["bindings"],
        "modes": request["defaults"][0]["modes"],
        "actions": request["actions"],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn fallback_keymap_binds_the_palette() {
        let keymap = fallback_keymap(&Shortcuts::new());
        assert!(
            keymap
                .bindings
                .iter()
                .any(|b| b.action == "client.palette.commands" || b.keys == ["ctrl+shift+p"])
        );
    }

    #[test]
    fn names_are_unique_and_default_keys_do_not_collide() {
        let mut names = HashSet::new();
        let mut keys = HashSet::new();
        for action in ACTIONS {
            assert!(
                names.insert(action.name),
                "duplicate action {}",
                action.name
            );
            for (binding, args) in action.keys {
                let key = (binding.join(" "), *args);
                assert!(
                    keys.insert((action.mode, binding.join(" "))),
                    "{} is bound twice ({:?})",
                    key.0,
                    key.1
                );
            }
        }
        assert!(find("pane.split.right").is_some());
    }

    #[test]
    fn shortcuts_replace_defaults_and_take_their_keys() {
        let mut shortcuts = Shortcuts::new();
        // Split Right moves to ctrl+alt+k; Toggle Sidebar takes Split Right's old key.
        shortcuts.insert("pane.split.right".into(), vec!["ctrl+alt+k".into()]);
        let old = find("pane.split.right").unwrap().keys[0].0.to_vec();
        shortcuts.insert(
            "client.sidebar.toggle".into(),
            old.iter().map(|k| k.to_string()).collect(),
        );
        // Find loses its key.
        shortcuts.insert("client.find.open".into(), Vec::new());
        let request = keymap_request(&shortcuts);
        let bindings = request["defaults"][0]["bindings"].as_array().unwrap();
        let keys_of = |action: &str| -> Vec<Value> {
            bindings
                .iter()
                .filter(|b| b["action"] == action)
                .map(|b| b["keys"].clone())
                .collect()
        };
        assert_eq!(keys_of("pane.split.right"), vec![json!(["ctrl+alt+k"])]);
        assert_eq!(keys_of("client.sidebar.toggle"), vec![json!(old)]);
        assert!(keys_of("client.find.open").is_empty());
        // Go to Tab keeps its numbered keys: they carry arguments.
        assert!(!rebindable(find("client.tab.goto").unwrap()));
    }

    #[test]
    fn request_lists_actions_with_schemas_and_bindings() {
        let request = keymap_request(&Shortcuts::new());
        let actions = request["actions"].as_array().unwrap();
        assert_eq!(actions.len(), all().count());
        let goto = actions
            .iter()
            .find(|a| a["name"] == "client.tab.goto")
            .unwrap();
        assert_eq!(goto["args"]["required"][0], "index");
        let bindings = request["defaults"][0]["bindings"].as_array().unwrap();
        assert!(
            bindings
                .iter()
                .any(|b| b["keys"][0] == "alt+3" && b["args"]["index"] == 3)
        );
    }
}

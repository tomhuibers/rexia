//! A single-line text field for palettes, dialogs and the find bar.
use super::theme::Theme;
use gpui::{
    App, ClipboardItem, Context, EventEmitter, FocusHandle, Focusable, KeyDownEvent, SharedString,
    Window, div, prelude::*, px,
};

#[derive(Clone, Debug, PartialEq)]
pub enum InputEvent {
    Changed(String),
    /// Enter and its modifiers.
    Submit {
        shift: bool,
        alt: bool,
        primary: bool,
    },
    Cancel,
    /// Up (-1) or Down (1), for moving through a list under the field.
    Navigate(i32),
    Tab,
}

pub struct TextInput {
    focus: FocusHandle,
    text: String,
    /// Byte offsets: the cursor, and the other end of the selection when there is one.
    cursor: usize,
    anchor: Option<usize>,
    placeholder: SharedString,
    /// Shown as dots and never copied: passwords.
    secret: bool,
    /// Opt-in navigation aliases for native list pickers only.
    list_navigation: bool,
}

impl EventEmitter<InputEvent> for TextInput {}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn previous_boundary(text: &str, at: usize) -> usize {
    text[..at].char_indices().next_back().map_or(0, |(i, _)| i)
}

fn next_boundary(text: &str, at: usize) -> usize {
    text[at..].chars().next().map_or(at, |c| at + c.len_utf8())
}

/// The start of the word before `at`, skipping spaces and punctuation first.
fn word_start(text: &str, at: usize) -> usize {
    let before = &text[..at];
    let trimmed = before.trim_end_matches(|c: char| !c.is_alphanumeric());
    trimmed
        .char_indices()
        .rev()
        .find(|(_, c)| !c.is_alphanumeric())
        .map_or(0, |(i, c)| i + c.len_utf8())
}

fn word_end(text: &str, at: usize) -> usize {
    let after = &text[at..];
    let skipped = after.len()
        - after
            .trim_start_matches(|c: char| !c.is_alphanumeric())
            .len();
    let rest = &after[skipped..];
    at + skipped
        + rest
            .find(|c: char| !c.is_alphanumeric())
            .unwrap_or(rest.len())
}

fn list_navigation_step(enabled: bool, key: &str, modifiers: gpui::Modifiers) -> Option<i32> {
    if !enabled || !modifiers.control || modifiers.alt || modifiers.shift || modifiers.platform {
        return None;
    }
    match key {
        "p" => Some(-1),
        "n" => Some(1),
        _ => None,
    }
}

impl TextInput {
    pub fn new(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            text: String::new(),
            cursor: 0,
            anchor: None,
            placeholder: placeholder.into(),
            secret: false,
            list_navigation: false,
        }
    }

    /// Enable Ctrl+P/N alongside arrow keys for a native list picker.
    pub fn list_navigation(mut self) -> Self {
        self.list_navigation = true;
        self
    }

    /// A field for a password: dots on screen, and nothing copied from it.
    pub fn secret(mut self) -> Self {
        self.secret = true;
        self
    }

    /// The text from `start` to `end` as it's drawn.
    fn shown(&self, start: usize, end: usize) -> String {
        let text = &self.text[start..end];
        if self.secret {
            "•".repeat(text.chars().count())
        } else {
            text.to_owned()
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Replace the text, select all of it (so typing replaces it), and report the change.
    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.text = text.into();
        self.cursor = self.text.len();
        self.anchor = (!self.text.is_empty()).then_some(0);
        cx.emit(InputEvent::Changed(self.text.clone()));
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    fn selection(&self) -> Option<(usize, usize)> {
        let anchor = self.anchor?;
        (anchor != self.cursor).then(|| (anchor.min(self.cursor), anchor.max(self.cursor)))
    }

    fn delete_selection(&mut self) -> bool {
        let Some((start, end)) = self.selection() else {
            return false;
        };
        self.text.replace_range(start..end, "");
        self.cursor = start;
        self.anchor = None;
        true
    }

    fn insert(&mut self, text: &str) {
        self.delete_selection();
        // A collapsed selection (including Ctrl+A in an empty field) ends on input.
        self.anchor = None;
        let clean: String = text.chars().filter(|c| !c.is_control()).collect();
        self.text.insert_str(self.cursor, &clean);
        self.cursor += clean.len();
    }

    fn move_to(&mut self, to: usize, extend: bool) {
        if extend {
            self.anchor.get_or_insert(self.cursor);
        } else {
            self.anchor = None;
        }
        self.cursor = to.min(self.text.len());
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let m = keystroke.modifiers;
        let ctrl = m.control || m.platform;
        if let Some(step) = list_navigation_step(self.list_navigation, &keystroke.key, m) {
            cx.emit(InputEvent::Navigate(step));
            cx.stop_propagation();
            return;
        }
        let before = self.text.clone();
        let handled = match keystroke.key.as_str() {
            "enter" => {
                cx.emit(InputEvent::Submit {
                    shift: m.shift,
                    alt: m.alt,
                    primary: ctrl,
                });
                true
            }
            "escape" => {
                cx.emit(InputEvent::Cancel);
                true
            }
            "up" => {
                cx.emit(InputEvent::Navigate(-1));
                true
            }
            "down" => {
                cx.emit(InputEvent::Navigate(1));
                true
            }
            "tab" => {
                cx.emit(InputEvent::Tab);
                true
            }
            "backspace" => {
                if !self.delete_selection() && self.cursor > 0 {
                    let start = if ctrl || m.alt {
                        word_start(&self.text, self.cursor)
                    } else {
                        previous_boundary(&self.text, self.cursor)
                    };
                    self.text.replace_range(start..self.cursor, "");
                    self.cursor = start;
                }
                true
            }
            "delete" => {
                if !self.delete_selection() && self.cursor < self.text.len() {
                    let end = if ctrl || m.alt {
                        word_end(&self.text, self.cursor)
                    } else {
                        next_boundary(&self.text, self.cursor)
                    };
                    self.text.replace_range(self.cursor..end, "");
                }
                true
            }
            "left" => {
                let to = if ctrl || m.alt {
                    word_start(&self.text, self.cursor)
                } else if let (Some((start, _)), false) = (self.selection(), m.shift) {
                    start
                } else {
                    previous_boundary(&self.text, self.cursor)
                };
                self.move_to(to, m.shift);
                true
            }
            "right" => {
                let to = if ctrl || m.alt {
                    word_end(&self.text, self.cursor)
                } else if let (Some((_, end)), false) = (self.selection(), m.shift) {
                    end
                } else {
                    next_boundary(&self.text, self.cursor)
                };
                self.move_to(to, m.shift);
                true
            }
            "home" => {
                self.move_to(0, m.shift);
                true
            }
            "end" => {
                self.move_to(self.text.len(), m.shift);
                true
            }
            "a" if ctrl => {
                self.anchor = Some(0);
                self.cursor = self.text.len();
                true
            }
            "u" if m.control && !m.shift => {
                self.text.replace_range(..self.cursor, "");
                self.cursor = 0;
                self.anchor = None;
                true
            }
            "w" if m.control && !m.shift => {
                let start = word_start(&self.text, self.cursor);
                self.text.replace_range(start..self.cursor, "");
                self.cursor = start;
                true
            }
            "c" | "x" if ctrl && self.secret => true,
            "c" | "x" if ctrl => {
                if let Some((start, end)) = self.selection() {
                    cx.write_to_clipboard(ClipboardItem::new_string(
                        self.text[start..end].to_owned(),
                    ));
                    if keystroke.key == "x" {
                        self.delete_selection();
                    }
                }
                true
            }
            key if key == "paste" || (key == "v" && ctrl) || (key == "insert" && m.shift) => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    self.insert(&text);
                }
                true
            }
            _ => match keystroke.key_char.as_deref() {
                Some(text) if !ctrl && !text.is_empty() => {
                    self.insert(text);
                    true
                }
                _ => false,
            },
        };
        if handled {
            cx.stop_propagation();
            if self.text != before {
                cx.emit(InputEvent::Changed(self.text.clone()));
            }
            cx.notify();
        }
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let focused = self.focus.is_focused(window);
        let primary = theme.primary;
        // Anchor the caret between text runs without adding to their width.
        let focus = self.focus.clone();
        let cursor = |visible: bool| {
            let focus = focus.clone();
            div().relative().flex_none().w(px(0.)).h(px(18.)).child(
                gpui::canvas(
                    |_, _, _| {},
                    move |bounds, (), window, cx| {
                        super::workspace::Workspace::record_focus_caret(
                            window, cx, &focus, bounds, primary,
                        );
                        if visible {
                            window.paint_quad(gpui::fill(bounds, primary));
                        }
                    },
                )
                .absolute()
                .left_0()
                .top_0()
                .w(px(1.5))
                .h_full(),
            )
        };
        let mut line = div()
            .flex()
            .items_center()
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap();
        if self.text.is_empty() {
            line = line.when(focused, |d| d.child(cursor(true))).child(
                div()
                    .text_color(theme.text_muted)
                    .child(self.placeholder.clone()),
            );
        } else if let Some((start, end)) = self.selection() {
            line = line
                .child(self.shown(0, start))
                .when(focused && self.cursor == start, |d| d.child(cursor(false)))
                .child(
                    div()
                        .bg(theme.primary)
                        .text_color(theme.on_primary)
                        .child(self.shown(start, end)),
                );
            line = line
                .when(focused && self.cursor == end, |d| d.child(cursor(false)))
                .child(self.shown(end, self.text.len()));
        } else {
            line = line
                .child(self.shown(0, self.cursor))
                .when(focused, |d| d.child(cursor(true)))
                .child(self.shown(self.cursor, self.text.len()));
        }
        div()
            .id("text-input")
            .track_focus(&self.focus)
            .key_context("TextInput")
            .on_key_down(cx.listener(Self::on_key_down))
            .flex()
            .items_center()
            .h(px(36.))
            .px(px(10.))
            .rounded(theme.radius_small())
            .bg(theme.background)
            .when(theme.input_borders, |d| {
                d.border(theme.border_width).border_color(if focused {
                    theme.primary
                } else {
                    theme.border
                })
            })
            .text_size(px(14.))
            .text_color(theme.text)
            .cursor_text()
            .child(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_navigation_requires_opt_in_and_plain_control() {
        let ctrl = gpui::Modifiers {
            control: true,
            ..Default::default()
        };
        assert_eq!(list_navigation_step(true, "p", ctrl), Some(-1));
        assert_eq!(list_navigation_step(true, "n", ctrl), Some(1));
        assert_eq!(list_navigation_step(false, "n", ctrl), None);
        assert_eq!(list_navigation_step(false, "p", ctrl), None);
        assert_eq!(list_navigation_step(true, "x", ctrl), None);
        for modifiers in [
            gpui::Modifiers::default(),
            gpui::Modifiers {
                platform: true,
                ..Default::default()
            },
            gpui::Modifiers { alt: true, ..ctrl },
            gpui::Modifiers {
                shift: true,
                ..ctrl
            },
            gpui::Modifiers {
                platform: true,
                ..ctrl
            },
        ] {
            for key in ["n", "p"] {
                assert_eq!(list_navigation_step(true, key, modifiers), None);
            }
        }
    }

    #[test]
    fn word_motions_skip_punctuation() {
        let text = "git commit --amend";
        assert_eq!(word_start(text, text.len()), 13);
        assert_eq!(word_start(text, 13), 4);
        assert_eq!(word_end(text, 0), 3);
        assert_eq!(word_end(text, 3), 10);
        assert_eq!(previous_boundary("aé", 3), 1);
        assert_eq!(next_boundary("éa", 0), 2);
    }
}

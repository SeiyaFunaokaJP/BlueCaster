//! Log tab: the core's log lines with level, text and attempt filters,
//! selection and copy, following new lines, and the jump from a flow step.
//! Every kept line (up to 5000) is in a virtualized list: only the lines in
//! view are drawn.

use std::collections::HashSet;
use std::time::Duration;

use gpui_kit::component::Disableable as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Render, Role, ScrollStrategy, SharedString, StatefulInteractiveElement as _, Styled as _,
    UniformListScrollHandle, Window, div, px, uniform_list,
};
use ipc::{LogLevel, LogLine};
use ui_model::format::mask_identifiers;
use ui_model::i18n::t;
use ui_model::logview::{
    Attempt, LevelFilter, LogFilter, filter_log, format_log_line, keys_between, level_name, log_key,
};
use ui_model::t;

use super::shared::{mask, time_text};
use crate::app::{self, NoticeKind};
use crate::core::{self, CoreEvent, Want, WantKind};
use crate::host;
use crate::theme::{self, ActivePalette as _};
use crate::ui::{self, ButtonKind, Choice, IconName, icon};

/// How long a jump waits for its line (the debug lines come after a resubscription)
pub const JUMP_WAIT: Duration = Duration::from_millis(3000);

/// The log view's filters and selection (they outlive the log tab)
pub struct LogView {
    pub level: LevelFilter,
    pub query: String,
    pub attempt_only: bool,
    pub follow: bool,
    pub selected: HashSet<String>,
    pub anchor: Option<String>,
    /// The line jumped to from the flow (highlighted)
    pub focus_key: Option<String>,
    /// A jump from the flow waiting for its line (LogLine.seq)
    pub jump: Option<u64>,
}

impl Default for LogView {
    fn default() -> Self {
        LogView {
            level: LevelFilter::Level(LogLevel::Info),
            query: String::new(),
            attempt_only: false,
            follow: true,
            selected: HashSet::new(),
            anchor: None,
            focus_key: None,
            jump: None,
        }
    }
}

const LINE_H: f32 = 20.;

pub struct LogPanel {
    view: Entity<LogView>,
    search: Entity<InputState>,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    /// The log lines asked for: debug while "All" is chosen
    logs_want: Option<(bool, Want)>,
    /// The filtered lines: indices into the core's lines
    shown: Vec<usize>,
    /// The newest line drawn (to follow new ones)
    drawn_end: Option<(usize, String)>,
    /// A jump found its line: scroll to it once the filters show it
    scroll_to: Option<String>,
    /// A jump cleared the filters: the search box follows
    clear_search: bool,
}

impl LogPanel {
    pub fn new(view: Entity<LogView>, window: &mut Window, cx: &mut Context<Self>) -> LogPanel {
        core::want(WantKind::Flow, cx);
        let core = app::core(cx);
        cx.subscribe(&core, |this, _, ev: &CoreEvent, cx| {
            if matches!(ev, CoreEvent::Log | CoreEvent::Flow) {
                this.apply_jump(cx);
                cx.notify();
            }
        })
        .detach();
        cx.observe(&core, |_, _, cx| cx.notify()).detach();
        cx.observe(&view, |this, _, cx| {
            this.update_want(cx);
            this.apply_jump(cx);
            cx.notify();
        })
        .detach();
        cx.observe_global::<super::shared::DiagShared>(|_, cx| cx.notify()).detach();
        let query = view.read(cx).query.clone();
        let search = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder(t("dbgc.search_hint"));
            s.set_value(query, window, cx);
            s
        });
        cx.subscribe(&search, |this, search, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                let q = search.read(cx).value().to_string();
                this.view.update(cx, |v, cx| {
                    v.query = q;
                    cx.notify();
                });
            }
        })
        .detach();
        cx.on_release(|this: &mut LogPanel, cx| {
            if let Some((_, w)) = this.logs_want.take() {
                w.release(cx);
            }
        })
        .detach();
        let mut panel = LogPanel {
            view,
            search,
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            logs_want: None,
            shown: Vec::new(),
            drawn_end: None,
            scroll_to: None,
            clear_search: false,
        };
        panel.update_want(cx);
        panel.apply_jump(cx);
        panel
    }

    /// "All" shows debug lines: ask the core for them while that is chosen
    fn update_want(&mut self, cx: &mut Context<Self>) {
        let all = self.view.read(cx).level == LevelFilter::All;
        if self.logs_want.as_ref().is_some_and(|(a, _)| *a == all) {
            return;
        }
        let next = Want::new(WantKind::Logs(if all { LogLevel::Debug } else { LogLevel::Info }), cx);
        if let Some((_, old)) = self.logs_want.replace((all, next)) {
            old.release(cx);
        }
    }

    fn filter(&self, cx: &App) -> LogFilter {
        let v = self.view.read(cx);
        let attempt = if v.attempt_only { app::core(cx).read(cx).flow.as_ref().map(Attempt::from) } else { None };
        LogFilter { level: v.level, query: v.query.clone(), attempt }
    }

    /// A jump from the flow: find the line (a debug line comes with the
    /// "all" subscription), make the filters show it, select and scroll to it
    fn apply_jump(&mut self, cx: &mut Context<Self>) {
        let Some(seq) = self.view.read(cx).jump else { return };
        let core = app::core(cx);
        let line = core.read(cx).store.logs().iter().find(|l| l.seq == Some(seq)).cloned();
        let Some(line) = line else {
            if self.view.read(cx).level != LevelFilter::All {
                self.view.update(cx, |v, cx| {
                    v.level = LevelFilter::All;
                    cx.notify();
                });
            }
            return;
        };
        let hidden = filter_log(std::slice::from_ref(&line), &self.filter(cx)).is_empty();
        let k = log_key(&line);
        if hidden {
            self.clear_search = true;
        }
        self.scroll_to = Some(k.clone());
        self.view.update(cx, |v, cx| {
            if hidden {
                v.level = LevelFilter::All;
                v.query.clear();
                v.attempt_only = false;
            }
            v.follow = false;
            v.selected = HashSet::from([k.clone()]);
            v.anchor = Some(k.clone());
            v.focus_key = Some(k);
            v.jump = None;
            cx.notify();
        });
    }

    fn lines_text(lines: &[&LogLine], cx: &App) -> String {
        let m = mask(cx);
        lines.iter().map(|l| format_log_line(l, &time_text(l.time_ms, cx), m)).collect::<Vec<_>>().join("\n")
    }

    fn notify_copied(n: usize, cx: &mut App) {
        app::notify(t!("dbgc.copied", n), NoticeKind::Info, cx);
    }

    fn copy_shown(&mut self, cx: &mut Context<Self>) {
        let core = app::core(cx);
        let logs = core.read(cx).store.logs();
        let lines: Vec<&LogLine> = self.shown.iter().filter_map(|&i| logs.get(i)).collect();
        let text = Self::lines_text(&lines, cx);
        let n = lines.len();
        host::copy_text(text, cx);
        Self::notify_copied(n, cx);
    }

    fn copy_selected(&mut self, cx: &mut Context<Self>) {
        let core = app::core(cx);
        let sel = &self.view.read(cx).selected;
        let logs = core.read(cx).store.logs();
        let lines: Vec<&LogLine> = logs.iter().filter(|l| sel.contains(&log_key(l))).collect();
        if lines.is_empty() {
            return;
        }
        let text = Self::lines_text(&lines, cx);
        let n = lines.len();
        host::copy_text(text, cx);
        Self::notify_copied(n, cx);
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        app::core(cx).update(cx, |core, cx| core.clear_logs(cx));
        self.view.update(cx, |v, cx| {
            v.selected.clear();
            v.focus_key = None;
            cx.notify();
        });
    }

    fn on_line_click(&mut self, index: usize, e: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        let core = app::core(cx);
        let logs = core.read(cx).store.logs();
        let Some(line) = self.shown.get(index).and_then(|&i| logs.get(i)) else { return };
        let k = log_key(line);
        let m = e.modifiers();
        let filtered: Vec<LogLine> =
            if m.shift { self.shown.iter().filter_map(|&i| logs.get(i).cloned()).collect() } else { Vec::new() };
        // Selecting an older line stops following new ones (v1)
        let newest = self.shown.last().and_then(|&i| logs.get(i)).map(log_key);
        self.view.update(cx, |v, cx| {
            match (&v.anchor, m.shift, m.control || m.platform) {
                (Some(a), true, _) => v.selected = keys_between(&filtered, a, &k).into_iter().collect(),
                (_, _, true) => {
                    if !v.selected.remove(&k) {
                        v.selected.insert(k.clone());
                    }
                    v.anchor = Some(k.clone());
                }
                _ => {
                    v.selected = HashSet::from([k.clone()]);
                    v.anchor = Some(k.clone());
                }
            }
            v.focus_key = None;
            if newest.is_some_and(|n| n != k) {
                v.follow = false;
            }
            cx.notify();
        });
    }

    fn on_log_key(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let key = e.keystroke.key.to_lowercase();
        let m = &e.keystroke.modifiers;
        if key == "escape" && !self.view.read(cx).selected.is_empty() {
            self.view.update(cx, |v, cx| {
                v.selected.clear();
                cx.notify();
            });
            cx.stop_propagation();
            return;
        }
        if !(m.control || m.platform) {
            return;
        }
        if key == "a" {
            let core = app::core(cx);
            let logs = core.read(cx).store.logs();
            let all: HashSet<String> = self.shown.iter().filter_map(|&i| logs.get(i)).map(log_key).collect();
            self.view.update(cx, |v, cx| {
                v.selected = all;
                cx.notify();
            });
            cx.stop_propagation();
        } else if key == "c" && !self.view.read(cx).selected.is_empty() {
            self.copy_selected(cx);
            cx.stop_propagation();
        }
    }

    fn render_lines(&mut self, range: std::ops::Range<usize>, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let p = *cx.palette();
        let core = app::core(cx);
        let logs = core.read(cx).store.logs();
        let v = self.view.read(cx);
        let m = mask(cx);
        range
            .filter_map(|ix| {
                let l = logs.get(*self.shown.get(ix)?)?;
                let k = log_key(l);
                let selected = v.selected.contains(&k);
                let jumped = v.focus_key.as_deref() == Some(k.as_str());
                let (level_color, msg_color, line_color) = match l.level {
                    LogLevel::Error => (p.bad_text, p.bad_text, p.text),
                    LogLevel::Warn => (p.warn_text, p.warn_text, p.text),
                    LogLevel::Info => (p.text_2, p.text, p.text),
                    LogLevel::Debug | LogLevel::Trace => (p.text_3, p.text_3, p.text_3),
                };
                // One line each: the list needs lines of one height (a copy has the whole text)
                let msg = if m { mask_identifiers(&l.message) } else { l.message.clone() }.replace(['\r', '\n'], " ");
                // Assistive technology reads the whole message (the row cuts
                // it) and selects a row as a click does
                let name = format!("{} {} {}", level_name(l.level).to_uppercase(), l.target, msg);
                Some(
                    div()
                        .id(("log-line", ix))
                        .role(Role::ListItem)
                        .aria_label(name)
                        .aria_selected(selected)
                        // The list's width: the whole row is clicked and highlighted
                        .w_full()
                        .h(px(LINE_H))
                        .flex()
                        .items_center()
                        .gap(theme::S2)
                        .px(theme::S3)
                        .text_color(line_color)
                        .when(selected, |d| d.bg(gpui_kit::Hsla { a: 0.18, ..p.accent_fill }))
                        .when(!selected, |d| d.hover(|s| s.bg(p.surface_2)))
                        .when(jumped, |d| d.border_l(px(3.)).border_color(p.accent_fill))
                        .child(
                            div()
                                .w(px(120.))
                                .flex_none()
                                .truncate()
                                .text_color(p.text_3)
                                .child(time_text(l.time_ms, cx)),
                        )
                        .child(
                            div()
                                .w(px(56.))
                                .flex_none()
                                .text_color(level_color)
                                .child(level_name(l.level).to_uppercase()),
                        )
                        .child(div().w(px(160.)).flex_none().truncate().text_color(p.text_3).child(l.target.clone()))
                        .child(div().flex_1().min_w_0().truncate().text_color(msg_color).child(msg))
                        .on_click(cx.listener(move |this, e, window, cx| this.on_line_click(ix, e, window, cx)))
                        .into_any_element(),
                )
            })
            .collect()
    }
}

impl Render for LogPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        if std::mem::take(&mut self.clear_search) {
            self.search.update(cx, |s, cx| s.set_value("", window, cx));
        }
        let filter = self.filter(cx);
        let core = app::core(cx);
        let (total, log_dir) = {
            let c = core.read(cx);
            (c.store.logs().len(), c.snap().map(|s| s.log_dir.clone()).unwrap_or_default())
        };
        self.shown = core
            .read(cx)
            .store
            .logs()
            .iter()
            .enumerate()
            .filter(|(_, l)| !filter_log(std::slice::from_ref(*l), &filter).is_empty())
            .map(|(i, _)| i)
            .collect();
        let v = self.view.read(cx);
        let (level, attempt_only, follow, any_selected) = (v.level, v.attempt_only, v.follow, !v.selected.is_empty());

        // Scroll to a line jumped to, else follow new lines
        let keys = |i: usize| core.read(cx).store.logs().get(i).map(log_key);
        if let Some(k) = self.scroll_to.take() {
            match self.shown.iter().position(|&i| keys(i).as_deref() == Some(k.as_str())) {
                Some(ix) => self.scroll.scroll_to_item(ix, ScrollStrategy::Center),
                None => self.scroll_to = Some(k),
            }
        } else if follow {
            let end = self.shown.last().and_then(|&i| keys(i)).map(|k| (self.shown.len(), k));
            if end != self.drawn_end && !self.shown.is_empty() {
                self.scroll.scroll_to_item(self.shown.len() - 1, ScrollStrategy::Bottom);
            }
            self.drawn_end = end;
        } else {
            self.drawn_end = None;
        }

        let levels = vec![
            Choice::new(LevelFilter::All, t("dbgc.level_all")),
            Choice::new(LevelFilter::Level(LogLevel::Info), t("dbgc.level_info")),
            Choice::new(LevelFilter::Level(LogLevel::Warn), t("dbgc.level_warn")),
            Choice::new(LevelFilter::Level(LogLevel::Error), t("dbgc.level_error")),
        ];
        let view = self.view.clone();
        let set = move |cx: &mut App, f: Box<dyn FnOnce(&mut LogView)>| {
            view.update(cx, |v, cx| {
                f(v);
                cx.notify();
            })
        };
        let (set1, set2, set3) = (set.clone(), set.clone(), set);
        let tools =
            ui::inline()
                .gap(theme::S3)
                .child(div().min_w(px(160.)).child(ui::select_labelled(
                    "log-level",
                    t("dbgc.col_level"),
                    &level,
                    levels,
                    false,
                    move |l: &LevelFilter, _, cx| {
                        let l = *l;
                        set1(cx, Box::new(move |v| v.level = l));
                    },
                )))
                .child(div().flex_1().min_w(px(160.)).child(
                    Input::new(&self.search).prefix(icon(IconName::Search).text_color(p.text_3)).cleanable(true),
                ))
                .child(ui::check("log-attempt", t("dbgc.session_only"), attempt_only, false, move |c, _, cx| {
                    let c = *c;
                    set2(cx, Box::new(move |v| v.attempt_only = c));
                }))
                .child(ui::check("log-follow", t("dbgc.autoscroll"), follow, false, move |c, _, cx| {
                    let c = *c;
                    set3(cx, Box::new(move |v| v.follow = c));
                }))
                .child(ui::muted(t!("dbgc.count", self.shown.len(), total), cx));
        let actions = ui::inline()
            .gap(theme::S3)
            .child(
                ui::button("log-copy-selected", t("dbgc.copy_selected"), ButtonKind::Subtle)
                    .icon(icon(IconName::Copy))
                    .disabled(!any_selected)
                    .on_click(cx.listener(|this, _, _, cx| this.copy_selected(cx))),
            )
            .child(
                ui::button("log-copy", t("dbgc.copy_log"), ButtonKind::Subtle)
                    .icon(icon(IconName::Copy))
                    .on_click(cx.listener(|this, _, _, cx| this.copy_shown(cx))),
            )
            .child(
                ui::button("log-clear", t("ui.diag.clear"), ButtonKind::Subtle)
                    .icon(icon(IconName::Delete))
                    .on_click(cx.listener(|this, _, _, cx| this.clear(cx))),
            )
            .child(div().flex_1())
            // None while the core runs as a service: its log folder is protected
            .when(!log_dir.is_empty(), |d| {
                d.child(ui::hint(t("ui.diag.logs_unmasked"), cx)).child(
                    ui::button("open-logs", t("ui.diag.open_logs"), ButtonKind::Subtle)
                        .icon(icon(IconName::FolderOpen))
                        .on_click(move |_, _, cx| host::open_folder(&log_dir, cx)),
                )
            });

        // Grows with the window: the heading, the report card, the capture
        // line, the tabs and the tools above it take about this much
        let height = (window.viewport_size().height - px(660.)).max(px(256.));
        let count = self.shown.len();
        let list: gpui_kit::AnyElement = if count == 0 {
            div().p(theme::S3).text_color(p.text_2).child(SharedString::from(t("ui.diag.no_log"))).into_any_element()
        } else {
            uniform_list("log-lines", count, cx.processor(|this, range, _, cx| this.render_lines(range, cx)))
                .track_scroll(&self.scroll)
                .size_full()
                .into_any_element()
        };
        let log = div()
            .id("log")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_log_key))
            .h(height)
            .py(theme::S2)
            .bg(p.surface)
            .border_1()
            .border_color(p.border)
            .rounded(theme::RADIUS_CARD)
            .font_family(theme::FONT_MONO)
            .text_size(theme::FS_SMALL)
            .overflow_hidden()
            .child(list);

        div().flex().flex_col().gap(theme::S3).child(tools).child(actions).child(log)
    }
}

#[cfg(test)]
mod interaction_tests;

//! Chip tools (Developer screen): a chip script typed in or opened from a
//! file, checked as it is typed and run on a WinUSB adapter in place of the
//! Bluetooth stack (docs/chip-support.md). Lines that may change the chip
//! run only when allowed; the core refuses lines that could change it for
//! good.
//!
//! The Developer screen makes it with `cx.new(|cx| ChipTools::new(window, cx))`.

use std::time::Duration;

use gpui_kit::component::Disableable as _;
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, AsyncApp, Context, Div, Entity, IntoElement, ParentElement as _,
    PathPromptOptions, Render, Styled as _, Task, WeakEntity, Window, div, px,
};
use ipc::{AdapterInfo, DriverKind, Method, ScriptCheck, ScriptResult};
use ui_model::i18n::t;
use ui_model::links::LINKS;
use ui_model::view::adapter::script_runnable;

use super::adapter::adapter_name;
use crate::app;
use crate::core::Core;
use crate::host;
use crate::theme;
use crate::ui::{self, BannerKind, ButtonKind, Choice, IconName, icon};

/// Delay before a typed script is checked
const CHECK_MS: u64 = 300;

const EXAMPLE: &str = "# Read Local Version, then BD_ADDR\nhci 1001\nhci 1009\n";

/// Lines of a script or its answer (`.chip-lines`)
fn chip_lines(text: String) -> Div {
    div().font_family(theme::FONT_MONO).text_size(theme::FS_SMALL).child(text)
}

pub struct ChipTools {
    core: Entity<Core>,
    adapter: String,
    script: Entity<TextareaState>,
    check: Option<ScriptCheck>,
    allow: bool,
    running: bool,
    result: Option<ScriptResult>,
    /// The script and adapter the last check was asked for
    checked: Option<(String, String)>,
    check_task: Option<Task<()>>,
}

impl ChipTools {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> ChipTools {
        let core = app::core(cx);
        cx.observe(&core, |this: &mut ChipTools, _, cx| {
            this.schedule_check(cx);
            cx.notify();
        })
        .detach();
        let script = cx.new(|cx| TextareaState::new(window, cx).default_value(EXAMPLE));
        cx.subscribe(&script, |this: &mut ChipTools, _, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                // A new script asks again
                this.allow = false;
                this.schedule_check(cx);
                cx.notify();
            }
        })
        .detach();
        let mut this = ChipTools {
            core,
            adapter: String::new(),
            script,
            check: None,
            allow: false,
            running: false,
            result: None,
            checked: None,
            check_task: None,
        };
        this.schedule_check(cx);
        this
    }

    fn adapters(&self, cx: &App) -> Vec<AdapterInfo> {
        self.core
            .read(cx)
            .snap()
            .map(|s| s.adapter.adapters.iter().filter(|a| a.driver == DriverKind::Winusb).cloned().collect())
            .unwrap_or_default()
    }

    /// The adapter chosen, else the first on WinUSB
    fn current(&self, cx: &App) -> String {
        let adapters = self.adapters(cx);
        adapters
            .iter()
            .find(|a| a.instance_id == self.adapter)
            .or(adapters.first())
            .map(|a| a.instance_id.clone())
            .unwrap_or_default()
    }

    // Checked by the core as it is typed, for the adapter's vendor
    fn schedule_check(&mut self, cx: &mut Context<Self>) {
        let id = self.current(cx);
        let text = self.script.read(cx).value().to_string();
        let key = (text.clone(), id.clone());
        if id.is_empty() || self.checked.as_ref() == Some(&key) {
            return;
        }
        self.checked = Some(key);
        // A newer one drops the waiting one
        self.check_task = Some(cx.spawn(async move |this: WeakEntity<ChipTools>, cx: &mut AsyncApp| {
            cx.background_executor().timer(Duration::from_millis(CHECK_MS)).await;
            let Ok(task) = this.update(cx, |_, cx| {
                app::request::<ScriptCheck>(Method::AdapterCheckScript { instance_id: id, script: text }, cx)
            }) else {
                return;
            };
            let check = task.await.ok();
            let _ = this.update(cx, |this, cx| {
                this.check = check;
                cx.notify();
            });
        }));
    }

    fn run(&mut self, cx: &mut Context<Self>) {
        self.running = true;
        self.result = None;
        cx.notify();
        let method = Method::AdapterRunScript {
            instance_id: self.current(cx),
            script: self.script.read(cx).value().to_string(),
            allow_writes: self.allow,
        };
        let task = app::run(app::request::<ScriptResult>(method, cx), cx);
        cx.spawn(async move |this: WeakEntity<ChipTools>, cx: &mut AsyncApp| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.running = false;
                this.result = result;
                cx.notify();
            });
        })
        .detach();
    }

    /// A script from a file
    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(t("ui.dev.chip_open").into()),
        });
        cx.spawn_in(window, async move |this: WeakEntity<ChipTools>, cx| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let read = cx.background_executor().spawn(async move { std::fs::read_to_string(path) }).await;
            let _ = this.update_in(cx, |this, window, cx| match read {
                Ok(text) => this.script.update(cx, |s, cx| s.set_value(text, window, cx)),
                Err(_) => app::notify(t("status.error"), app::NoticeKind::Error, cx),
            });
        })
        .detach();
    }

    fn check_banners(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(c) = &self.check else { return vec![] };
        let banner = |kind: BannerKind, title: &str, lines: &[String], extra: Option<AnyElement>, cx: &App| {
            let body = div().flex().flex_col().gap(theme::S2).child(chip_lines(lines.join("\n"))).children(extra);
            ui::banner(kind, Some(t(title).into()), Some(body.into_any_element()), None, None, cx).into_any_element()
        };
        let mut out = Vec::new();
        if !c.errors.is_empty() {
            out.push(banner(BannerKind::Error, "ui.dev.chip_errors", &c.errors, None, cx));
        }
        if !c.denied.is_empty() {
            out.push(banner(BannerKind::Error, "ui.dev.chip_denied", &c.denied, None, cx));
        }
        if c.errors.is_empty() && c.denied.is_empty() && !c.writes.is_empty() {
            let this = cx.weak_entity();
            let allow =
                ui::check("chip-allow-writes", t("ui.dev.chip_allow_writes"), self.allow, false, move |v, _, cx| {
                    let v = *v;
                    let _ = this.update(cx, |this, cx| {
                        this.allow = v;
                        cx.notify();
                    });
                });
            out.push(banner(BannerKind::Warn, "ui.dev.chip_writes", &c.writes, Some(allow.into_any_element()), cx));
        }
        out
    }
}

impl Render for ChipTools {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let adapters = self.adapters(cx);
        if adapters.is_empty() {
            return ui::banner(BannerKind::Info, Some(t("ui.dev.chip_no_adapter").into()), None, None, None, cx)
                .into_any_element();
        }
        let current = self.current(cx);
        let this = cx.weak_entity();
        let choices = adapters.iter().map(|a| Choice::new(a.instance_id.clone(), adapter_name(a))).collect();
        let select = ui::select("chip-adapter", &current, choices, false, move |id: &String, _, cx| {
            let id = id.clone();
            let _ = this.update(cx, |this, cx| {
                this.adapter = id;
                this.schedule_check(cx);
                cx.notify();
            });
        });
        let format = ui::button("chip-format", t("ui.dev.chip_format"), ButtonKind::Subtle)
            .icon(icon(IconName::Open))
            .on_click(|_, _, cx| host::open_url(LINKS.chip_scripts, cx));
        let script = div()
            .my(theme::S2)
            .font_family(theme::FONT_MONO)
            .child(Textarea::new(&self.script).h(px(160.)).aria_label(t("ui.dev.chip_script")));
        let buttons = ui::row()
            .justify_start()
            .gap(theme::S2)
            .child(
                ui::button("chip-open", t("ui.dev.chip_open"), ButtonKind::Default)
                    .icon(icon(IconName::FolderOpen))
                    .on_click(cx.listener(|this, _, window, cx| this.open(window, cx))),
            )
            .child(
                ui::button("chip-run", t("ui.dev.chip_run"), ButtonKind::Strong)
                    .icon(icon(IconName::Play))
                    .disabled(!script_runnable(self.check.as_ref(), self.allow) || self.running)
                    .on_click(cx.listener(|this, _, _, cx| this.run(cx))),
            );
        let script_section = ui::section(t("ui.dev.chip_script"), Some(format.into_any_element()), cx)
            .child(ui::field(t("ui.nav.adapter"), None, None, select, cx))
            .child(script)
            .child(buttons)
            .children(self.check_banners(cx));

        let result = self.result.clone().map(|r| {
            let path = r.path.clone();
            let show = ui::button("chip-show-report", t("ui.dev.chip_show_report"), ButtonKind::Subtle)
                .icon(icon(IconName::FolderOpen))
                .on_click(move |_, _, cx| host::reveal_path(&path, cx));
            ui::section(r.summary.clone(), Some(show.into_any_element()), cx)
                .child(chip_lines(r.lines.join("\n")))
                .when(!r.other_events.is_empty(), |d| {
                    d.child(chip_lines(
                        r.other_events.iter().map(|e| format!("event {e}")).collect::<Vec<_>>().join("\n"),
                    ))
                })
        });

        div().flex().flex_col().gap(theme::S4).child(script_section).children(result).into_any_element()
    }
}

//! About: the program, its version, licence and where to find the author;
//! then, in a box of its own (issue #86), the third-party software: the
//! libraries' licences and the dialog of VB-Audio's cables, which names them
//! as VB-Audio's donationware.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{
    AppContext as _, AsyncApp, Context, Entity, FontWeight, Image, ImageFormat, IntoElement, ParentElement as _,
    Render, Styled as _, WeakEntity, Window, div, img, px,
};
use ipc::{AudioDevice, Method};
use ui_model::i18n::t;
use ui_model::links::LINKS;
use ui_model::t;
use ui_model::view::about::{Licenses, licenses};

use crate::app;
use crate::core::Core;
use crate::host;
use crate::theme;
use crate::ui::cable_notes::{CablesDialog, Reload};
use crate::ui::{self, ButtonKind, IconName, icon};

const APP_ICON: &[u8] = include_bytes!("../../resources/app-icon.png");

pub struct About {
    core: Entity<Core>,
    icon: Arc<Image>,
    /// The cables' dialog, with Windows' outputs read when it opens
    cables: Entity<CablesDialog>,
    cables_open: bool,
    devices_key: String,
}

impl About {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> About {
        let core = app::core(cx);
        let devices_key = ui::audio_devices_key(core.read(cx).snap());
        cx.observe(&core, |this, core, cx| {
            let key = ui::audio_devices_key(core.read(cx).snap());
            if key != this.devices_key {
                this.devices_key = key;
                if this.cables_open {
                    this.load_devices(cx);
                }
            }
            cx.notify();
        })
        .detach();
        let weak = cx.weak_entity();
        let reload: Reload = Rc::new(move |cx| {
            let _ = weak.update(cx, |this: &mut About, cx| this.load_devices(cx));
        });
        let cables = cx.new(|cx| CablesDialog::new(reload, cx));
        About {
            core,
            icon: Arc::new(Image::from_bytes(ImageFormat::Png, APP_ICON.to_vec())),
            cables,
            cables_open: false,
            devices_key,
        }
    }

    fn load_devices(&mut self, cx: &mut Context<Self>) {
        let task = app::request::<Vec<AudioDevice>>(Method::AudioDevices, cx);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let Ok(list) = task.await else { return };
            let _ = this.update(cx, |this, cx| this.cables.update(cx, |d, cx| d.set_devices(Some(&list), cx)));
        })
        .detach();
    }

    fn open_cables(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cables.update(cx, |d, cx| d.set_devices(None, cx));
        self.load_devices(cx);
        self.cables_open = true;
        let weak = cx.weak_entity();
        CablesDialog::open(
            &self.cables,
            move |_, cx| {
                let _ = weak.update(cx, |this, _| this.cables_open = false);
            },
            window,
            cx,
        );
    }
}

/// The licence texts shipped next to the programs; a development build
/// has none, so the release's copy online
fn open_licenses(cx: &mut gpui_kit::App) {
    match licenses(host::licenses_dir(), host::VERSION) {
        Licenses::Folder(dir) => host::open_folder(&dir, cx),
        Licenses::Online(url) => host::open_url(&url, cx),
    }
}

/// A subtle button that opens a page
fn page_button(id: &'static str, label: String, url: &'static str) -> impl IntoElement {
    ui::button(id, label, ButtonKind::Subtle)
        .icon(icon(IconName::Open))
        .on_click(move |_, _, cx| host::open_url(url, cx))
}

impl Render for About {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let core_version = self.core.read(cx).snap().map(|s| s.update.current_version.clone()).unwrap_or_default();
        ui::page("about", t("ui.nav.about"))
            .child(
                ui::section(t("help.about_title"), None, cx)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(theme::S4)
                            .child(img(self.icon.clone()).size(px(48.)).flex_none())
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_w_0()
                                    .gap(theme::S1)
                                    .child(div().font_weight(FontWeight::SEMIBOLD).child(t("help.full_name")))
                                    .child(ui::muted(
                                        format!(
                                            "{} {} · {}",
                                            t("help.version"),
                                            host::VERSION,
                                            t!("ui.settings.core_version", &core_version)
                                        ),
                                        cx,
                                    ))
                                    .child(ui::muted(t("help.author"), cx)),
                            ),
                    )
                    .child(ui::hint(t("help.license"), cx))
                    .child(ui::hint(t("ui.settings.icons_license"), cx))
                    .child(
                        ui::inline()
                            .child(page_button("github", t("help.github"), LINKS.repo))
                            .child(page_button("report-issue", t("menu.help.report_issue"), LINKS.issues))
                            .child(page_button("sponsor", t("help.sponsor"), LINKS.sponsor)),
                    ),
            )
            .child(
                ui::section(t("help.third_party_title"), None, cx)
                    .child(ui::hint(t("help.binary_license"), cx))
                    .child(ui::hint(t("help.third_party"), cx))
                    .child(
                        ui::inline()
                            .child(
                                ui::button("licenses", t("help.third_party_link"), ButtonKind::Subtle)
                                    .icon(icon(IconName::Open))
                                    .on_click(|_, _, cx| open_licenses(cx)),
                            )
                            .child(
                                ui::button("cables", t("cables.title"), ButtonKind::Subtle)
                                    .on_click(cx.listener(|this, _, window, cx| this.open_cables(window, cx))),
                            ),
                    ),
            )
    }
}

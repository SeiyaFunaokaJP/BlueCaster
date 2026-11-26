//! Volume Mixer (in developer mode, issue #223): the output's mute, and each
//! app's volume, which also works under that mute.

use gpui_kit::{AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, Window, div};
use ui_model::i18n::t;

use crate::app;
use crate::core::Core;
use crate::theme::{self, ActivePalette as _};
use crate::ui::{self, mixer::MixerList};

pub struct Mixer {
    core: Entity<Core>,
    list: Entity<MixerList>,
}

impl Mixer {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Mixer {
        let core = app::core(cx);
        cx.observe(&core, |_, _, cx| cx.notify()).detach();
        let list = cx.new(|cx| MixerList::new(window, cx));
        Mixer { core, list }
    }
}

impl Render for Mixer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let mute = self.core.read(cx).snap().is_some_and(|s| s.settings.mute_output);
        let note = |key: &str| {
            div()
                .flex()
                .gap(theme::S2)
                .child(div().flex_none().child("•"))
                .child(div().flex_1().min_w_0().child(t(key)))
        };
        ui::page("mixer", t("ui.nav.mixer"))
            .child(
                ui::section(t("ui.mixer.output"), None, cx)
                    .child(ui::toggle(
                        "mute-output",
                        t("capture.mute_output"),
                        Some(t("capture.mute_output_hint").into()),
                        mute,
                        false,
                        |v, _, cx| {
                            let v = *v;
                            app::save_settings(move |s| s.mute_output = v, cx);
                        },
                        cx,
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .pl(theme::S2)
                            .text_size(theme::FS_SMALL)
                            .text_color(p.text_2)
                            .child(note("ui.mixer.note_windows"))
                            .child(note("ui.mixer.note_exit"))
                            .child(note("ui.mixer.note_virtual")),
                    ),
            )
            .child(ui::section(t("ui.mixer.apps"), None, cx).child(self.list.clone()))
    }
}

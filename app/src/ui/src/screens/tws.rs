//! Developer > TWS: the matrix mixer of an LE Audio stream to a device's
//! two outputs (FL FR; tws.*). Off until turned on, then a stream started
//! from then on takes it instead of the fixed downmix; while one runs with
//! it, a change is heard at once. The device's own channel settings (Audio
//! Devices) apply after it.

use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, Subscription, Window, div,
};
use ipc::StreamState;

use crate::app;
use crate::core::Core;
use crate::screens::auracast_mixer::{AuracastMixer, MixTarget};
use crate::theme;

pub struct Tws {
    core: Entity<Core>,
    mixer: Entity<AuracastMixer>,
    _subs: Vec<Subscription>,
}

impl Tws {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Tws {
        let core = app::core(cx);
        let mixer = cx.new(|cx| AuracastMixer::new_for(MixTarget::Tws, window, cx));
        let subs = vec![
            cx.observe_in(&core, window, |this, _, window, cx| this.sync(window, cx)),
            cx.observe(&mixer, |_, _, cx| cx.notify()),
        ];
        let mut this = Tws { core, mixer, _subs: subs };
        this.sync(window, cx);
        this
    }

    /// The mixer is read again when a stream starts or stops (whether a
    /// change is heard at once)
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let live = self.core.read(cx).snap().is_some_and(|s| s.stream.state == StreamState::Streaming);
        self.mixer.update(cx, |m, cx| m.set_props(Some((false, live)), window, cx));
    }
}

impl Render for Tws {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let section = self.mixer.update(cx, |m, cx| m.section(Vec::new(), cx));
        div().flex().flex_col().gap(theme::S4).children(section)
    }
}

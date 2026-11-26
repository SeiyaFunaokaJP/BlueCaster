//! Codecs (issue #86, it replaced Profiles): each saved audio device with
//! its codec cards, the codecs it offered when it last connected in green
//! and the others in red (hidden unless asked for), each with its settings
//! behind "...", and a dot beside the device's name that is green while it
//! streams. The cards are dragged into the device's own order (its
//! profile's codec_order, issue #86 round 8; with Alt+arrow keys too while
//! Auto-select is off), highest sound quality first unless changed; Auto
//! takes the first card the device offers. "Auto-select the codec with the
//! highest quality" is one setting for every device (settings.auto_codec,
//! on by default; each device had its own before): on, the cards show what
//! Auto picks, and a click on one, or on its "...", offers to turn
//! Auto-select off; off, choosing a card is choosing that device's codec (a
//! device with none chosen yet, its profile's codec "auto", goes on with
//! Auto's pick). A card the device does not offer is chosen like any other
//! (issue #86: without Auto-select the choice is the user's): the core asks
//! for it anyway (the profile's codec_forced) and the stream stops with an
//! error if the device refuses it. A codec chosen while the device offered
//! it and no longer offered (earbuds switched codecs) falls back to the
//! first card it offers in that order (ui_model::codecs effective_codec, as
//! the core), shown chosen. A codec it may offer only once chosen
//! (Device.codecs_on_request) stays a card of its own, chosen also while
//! another one streams instead (issue #30). The codec chosen stays in the
//! device's profile while Auto-select is on. The device is named without
//! its address (shown on Audio Devices).
//! Choosing changes the device's profile (made when first needed; the core
//! keeps one per device and starts a stream in use again with it). What
//! belongs to the device rather than the codec (capture source, volume sync,
//! packet size) is on Audio Devices. An LE Audio entry (experimental) has LC3
//! only: its cards LC3 (the default) and LC3 LL (the profile's
//! le_low_latency, issue #86). aptX Adaptive's low latency mode is a card of
//! its own, aptX Adaptive LL (ui_model::codecs APTXAD_LL: aptX Adaptive with
//! quality "ll" in the profile; Auto never takes it). The LE Audio and call
//! codec cards' "..." shows the fields of an A2DP codec's, read only.
//! The microphone's codec (issue #86): while a classic device's call link
//! is open its call codec cards replace the music ones, and while an LE Audio
//! device's microphone runs an LC3 card for it follows LC3's. Under
//! Auto-select they show what the link took (a call codec card clicked
//! offers to turn Auto-select off); off, a call codec card is chosen (the
//! profile's voice_codec: the best the link may use, the next lower one the
//! headphones list otherwise), and the LE Audio microphone's "..." chooses
//! its rate among those the headphones allow (the profile's mic_rate, 0 =
//! Auto). The choice stays in the profile while Auto-select is on. The
//! microphone stays mono. The call codec cards say which the headphones
//! list (CVSD always) while the unsupported ones show. While the call link
//! opens again with another codec (A2DP in between) or the LE Audio stream
//! is set up again for another rate, the microphone's cards stay, waiting.
//! A codec's "..." applies each change at once (issue #86): the codec in use
//! is saved, which starts the stream again but for the bitpools, which the
//! encoder takes as the slider moves; another codec's settings are only kept
//! for when it is chosen.
//! A codec chosen while a stream runs starts it again: the cards (and Home's
//! Codec card) then wait until it streams again before the next choice
//! (issue #86: quick switches left no sound or crackling; a fixed 3 s was
//! slow on a quick device and too short on a slow one). With the unsupported
//! codecs hidden, the cards shown need no "Supported" tag (issue #86).
//!
//! The rules are ui_model::view::codecs; the cards and the codec dialog are
//! ui/codec_cards.rs.

use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::component::Disableable as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, AsyncApp, Context, Div, DragMoveEvent, Entity, FocusHandle, FontWeight, Global,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Task, WeakEntity, Window, canvas, div, px,
};
use ipc::{AudioDevice, MediaEq, Method, Profile, StreamState};
use ui_model::codecs::{
    CARD_ORDER, CodecChoice, DevMode, auto_keeps_codec_settings, caps_for_settings, card_of, card_order, codec_spec,
    sbc_caps_for_settings,
};
use ui_model::i18n::t;
use ui_model::profile::{
    AudioEntry, EntryByKey, audio_entries, capture_device, entry_base, followed_name, known_codec_caps,
    normalize_profile,
};
use ui_model::sbc::known_sbc_caps;
use ui_model::t;
use ui_model::view::codecs::{
    FixedCard, LE_CARDS, LE_MIC_CARD, MicView, REOPEN_WAIT_MS, SAVE_DELAY_MS, SETTLE_MAX_MS, SETTLE_MIN_MS, call_cards,
    chosen_profile, device_cards, fixed_values, forced_for, mic_rate_options, mic_view, moved_by, only_bitpools,
    order_keys, placed, shown_in_default_order, shown_profile, values_of,
};
use ui_model::view::stream::{entry_state, state_label};

// What Home and the flyout take from here
#[allow(unused_imports)]
pub use ui_model::view::codecs::{codec_shown, picked_codec, restarting, streaming_codec};

use crate::app::{self, RouteName};
use crate::core::Core;
use crate::theme::{self, ActivePalette as _};
use crate::ui::codec_cards::{
    Act, CardActions, CardSpec, CardState, DraggedCodec, SettingsInput, card_min_w, codec_card, open_codec_settings,
    value_box,
};
use crate::ui::device_picker::tag;
use crate::ui::dialog::{self, Question};
use crate::ui::stream::{dot, state_colors};
use crate::ui::{self, BannerKind, ButtonKind, Choice, IconName, icon};

// ---- saving a device's profile ----------------------------------------------------

/// Saves the device's profile as `edit` changes it (the profile is made
/// when the device has none yet); its name follows the codec while it is
/// "<device> | <codec>". The profile's name once saved, None if that failed.
pub fn save_entry(entry: &AudioEntry, edit: impl FnOnce(&mut Profile), cx: &App) -> Task<Option<String>> {
    let Some(snap) = app::core(cx).read(cx).snap() else { return Task::ready(None) };
    let cur = entry.profile.clone();
    let mut next =
        cur.clone().unwrap_or_else(|| entry_base(&entry.address, &entry.name, entry.le_audio, &snap.profiles));
    edit(&mut next);
    let cur_name = cur.as_ref().map(|c| c.name.clone());
    let taken: Vec<&str> =
        snap.profiles.iter().map(|p| p.name.as_str()).filter(|n| Some(*n) != cur_name.as_deref()).collect();
    let codec = if entry.le_audio { String::new() } else { t(codec_spec(card_of(&next.codec, &next.quality)).label) };
    next.name = followed_name(&next, &codec, &taken);
    if cur.as_ref() == Some(&next) {
        return Task::ready(Some(next.name));
    }
    let saved = normalize_profile(&next);
    let name = saved.name.clone();
    let task = app::run(
        app::request::<serde_json::Value>(
            Method::ProfileSave { profile: Box::new(saved), original_name: cur_name },
            cx,
        ),
        cx,
    );
    cx.spawn(async move |_: &mut AsyncApp| task.await.map(|_| name))
}

/// Saves a device's headset mode, microphone output, default input, the
/// microphone's processing (Audio Devices) or mute (Home's and the flyout's
/// chip). The core opens or closes the microphone link (Auto: as apps record
/// from the output); the processing applies in place. True once saved.
pub fn save_headset_mic(entry: &AudioEntry, patch: impl FnOnce(&mut Profile), cx: &App) -> Task<bool> {
    let task = save_entry(entry, patch, cx);
    cx.spawn(async move |_: &mut AsyncApp| task.await.is_some())
}

/// Saves the microphone's codec chosen for a device: the call codec of a
/// classic one, the microphone's rate (Hz, 0 = Auto) of an LE Audio one.
/// The core takes it in place or sets the stream up again. True once saved.
pub fn save_mic_codec(entry: &AudioEntry, patch: impl FnOnce(&mut Profile), cx: &App) -> Task<bool> {
    let task = save_entry(entry, patch, cx);
    cx.spawn(async move |_: &mut AsyncApp| task.await.is_some())
}

/// Saves a device's playback EQ (Audio Devices). A stream of it takes the
/// change in place. True once saved.
pub fn save_media_eq(entry: &AudioEntry, eq: MediaEq, cx: &App) -> Task<bool> {
    let task = save_entry(entry, move |p| p.media_eq = eq, cx);
    cx.spawn(async move |_: &mut AsyncApp| task.await.is_some())
}

/// Saves the device's profile with `codec` chosen (`values`: from its "..."
/// dialog; `order`: the device's codec order, when one is not saved yet;
/// `forced`: chosen though the device does not offer it, asked for anyway).
/// The profile's name once saved, None if that failed.
pub fn choose_codec(
    entry: &AudioEntry,
    codec: &str,
    values: Option<&CodecChoice>,
    order: Option<&[&'static str]>,
    forced: bool,
    cx: &App,
) -> Task<Option<String>> {
    let values = values.map(values_of);
    let le_audio = entry.le_audio;
    save_entry(entry, |p| *p = chosen_profile(p, le_audio, codec, values.as_ref(), order, forced), cx)
}

/// Saves the device's codec order (its cards' order)
pub fn save_codec_order(entry: &AudioEntry, order: &[&'static str], cx: &App) -> Task<Option<String>> {
    let keys = order_keys(order);
    save_entry(entry, move |p| p.codec_order = keys, cx)
}

// ---- waiting for a stream started again ---------------------------------------

/// For every device and across pages: one stream starts again at a time
pub struct Settle {
    pub settling: bool,
    timer: Option<Task<()>>,
}

struct SettleGlobal(Entity<Settle>);

impl Global for SettleGlobal {}

/// The codec cards' wait, to observe
pub fn settle_state(cx: &mut App) -> Entity<Settle> {
    if let Some(g) = cx.try_global::<SettleGlobal>() {
        return g.0.clone();
    }
    let state = cx.new(|_| Settle { settling: false, timer: None });
    cx.set_global(SettleGlobal(state.clone()));
    // Home's and the flyout's codec lists (functions, not views) wait too
    cx.observe(&state, |_, cx| cx.refresh_windows()).detach();
    state
}

/// The codec cards wait for the stream started again with a new codec
pub fn settling(cx: &App) -> bool {
    cx.try_global::<SettleGlobal>().is_some_and(|g| g.0.read(cx).settling)
}

/// The codec cards wait for the stream started again with a new codec: at
/// least SETTLE_MIN_MS and until the stream is no longer starting again,
/// never longer than SETTLE_MAX_MS
pub fn settle(cx: &mut App) {
    let state = settle_state(cx);
    state.update(cx, |s, cx| {
        let since = Instant::now();
        s.settling = true;
        s.timer = Some(cx.spawn(async move |this: WeakEntity<Settle>, cx: &mut AsyncApp| {
            loop {
                cx.background_executor().timer(Duration::from_millis(200)).await;
                let waited = since.elapsed();
                let again = cx.update(|cx| app::try_core(cx).is_some_and(|c| restarting(c.read(cx).snap())));
                if (waited >= Duration::from_millis(SETTLE_MIN_MS) && !again)
                    || waited >= Duration::from_millis(SETTLE_MAX_MS)
                {
                    let _ = this.update(cx, |s, cx| {
                        s.settling = false;
                        cx.notify();
                    });
                    return;
                }
            }
        }));
        cx.notify();
    });
}

/// Resolves once the codec cards no longer wait
async fn until_settled(cx: &mut AsyncApp) {
    while cx.update(|cx| settling(cx)) {
        cx.background_executor().timer(Duration::from_millis(100)).await;
    }
}

// ---- Auto-select in the way -----------------------------------------------------

/// A card, or its "...", clicked under Auto-select (issue #86 round 8):
/// offers to turn it off, then `then` (the card chosen, or its "..." opened)
fn ask_auto_off(then: Act, window: &mut Window, cx: &mut App) {
    dialog::ask(
        Question::new(t("ui.codecs.auto_dialog_title")).confirm(t("ui.codecs.auto_dialog_disable")),
        |_, _| div().child(t("ui.codecs.auto_dialog_text")).into_any_element(),
        move |window, cx| {
            let saved = app::apply_settings(|s| s.auto_codec = false, cx);
            let then = then.clone();
            window
                .spawn(cx, async move |cx| {
                    if saved.await.is_some() {
                        let _ = cx.update(|window, cx| then(window, cx));
                    }
                })
                .detach();
        },
        window,
        cx,
    );
}

// ---- the screen -------------------------------------------------------------------

pub struct Codecs {
    core: Entity<Core>,
    /// By key: a device keeps its section (and an open dialog) across updates
    sections: Vec<(String, Entity<DeviceCodecs>)>,
    /// The PC's output devices with their formats (the headset microphone's
    /// output, Auto matching them in developer mode), again when the window
    /// comes back (changed in Windows meanwhile)
    audio_devices: Rc<Vec<AudioDevice>>,
    audio_key: String,
}

impl Codecs {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Codecs {
        let core = app::core(cx);
        let audio_key = ui::audio_devices_key(core.read(cx).snap());
        cx.observe(&core, |this: &mut Self, core, cx| {
            let key = ui::audio_devices_key(core.read(cx).snap());
            if key != this.audio_key {
                this.audio_key = key;
                this.load_devices(cx);
            }
            this.sync(cx);
            cx.notify();
        })
        .detach();
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.load_devices(cx);
            }
        })
        .detach();
        let mut this = Codecs { core, sections: Vec::new(), audio_devices: Rc::new(Vec::new()), audio_key };
        this.load_devices(cx);
        this.sync(cx);
        this
    }

    fn load_devices(&mut self, cx: &mut Context<Self>) {
        let task = app::request::<Vec<AudioDevice>>(Method::AudioDevices, cx);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let Ok(list) = task.await else { return };
            let _ = this.update(cx, |this, cx| {
                this.audio_devices = Rc::new(list);
                for (_, s) in &this.sections {
                    let devices = this.audio_devices.clone();
                    s.update(cx, |s, cx| s.set_audio_devices(devices, cx));
                }
            });
        })
        .detach();
    }

    /// One section per saved device, kept by its key
    fn sync(&mut self, cx: &mut Context<Self>) {
        let Some(snap) = self.core.read(cx).snap() else { return };
        let keys: Vec<String> = audio_entries(snap).into_iter().map(|e| e.key).collect();
        if keys.iter().eq(self.sections.iter().map(|(k, _)| k)) {
            return;
        }
        let mut old = std::mem::take(&mut self.sections);
        for key in keys {
            let section = match old.iter().position(|(k, _)| *k == key) {
                Some(i) => old.swap_remove(i).1,
                None => {
                    let devices = self.audio_devices.clone();
                    let k = key.clone();
                    cx.new(|cx| DeviceCodecs::new(k, devices, cx))
                }
            };
            self.sections.push((key, section));
        }
    }

    fn set_hide(&mut self, hide: bool, cx: &mut Context<Self>) {
        crate::ui_state::set_hide_unsupported(hide);
        for (_, s) in &self.sections {
            s.update(cx, |_, cx| cx.notify());
        }
        cx.notify();
    }

    /// No saved device: a way to add one
    fn empty(cx: &App) -> Div {
        let p = cx.palette();
        ui::card(cx)
            .items_center()
            .py(theme::S8)
            .child(div().text_color(p.text_3).child(icon(IconName::Headphones).size(px(48.))))
            .child(div().text_color(p.text_2).child(t("ui.home.no_device")))
            .child(
                ui::button("add-device", t("ui.home.add_device"), ButtonKind::Strong)
                    .icon(icon(IconName::Add))
                    .on_click(|_, _, cx| app::navigate(RouteName::Devices, cx)),
            )
    }
}

impl Render for Codecs {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(auto) = self.core.read(cx).snap().map(|s| s.settings.auto_codec) else {
            return div().into_any_element();
        };
        let hide = hide();
        let options = ui::card(cx)
            .child(ui::first_row(ui::toggle(
                "auto-select",
                t("ui.codecs.auto_select"),
                None,
                auto,
                false,
                |v, _, cx| {
                    let v = *v;
                    app::save_settings(move |s| s.auto_codec = v, cx);
                },
                cx,
            )))
            .child(ui::toggle(
                "codecs-hide",
                t("ui.profile.codecs_hide"),
                None,
                hide,
                false,
                cx.listener(|this, v: &bool, _, cx| this.set_hide(*v, cx)),
                cx,
            ));
        let page = ui::page("codecs", t("ui.nav.codecs")).child(options);
        if self.sections.is_empty() {
            page.child(Self::empty(cx)).into_any_element()
        } else {
            page.children(self.sections.iter().map(|(_, s)| s.clone())).into_any_element()
        }
    }
}

// ---- one device -------------------------------------------------------------------

type Job = Box<dyn FnOnce(&mut App) -> Task<Option<String>>>;
/// A fixed card's choice or click, by its id
type CardAct = Rc<dyn Fn(&'static str, &mut Window, &mut App)>;

/// The microphone's link opening again with a codec chosen: the call codec
/// shown chosen meanwhile, the LE microphone's rate
#[derive(Debug, Clone, PartialEq)]
struct Reopening {
    to: String,
    rate: u32,
}

/// One device: its activity, name and tags, then its codec cards
pub struct DeviceCodecs {
    core: Entity<Core>,
    key: String,
    entry: EntryByKey,
    /// The entry as last seen
    current: Option<AudioEntry>,
    audio_devices: Rc<Vec<AudioDevice>>,
    busy: bool,
    // The device's saves one after the other: a card chosen right after one
    // was moved starts from the profile with the new order
    queue: VecDeque<(Job, tokio::sync::oneshot::Sender<Option<String>>)>,
    saving: bool,
    // ---- a "..." dialog's changes (issue #86: applied at once) ----
    pending: Option<CodecChoice>,
    save_timer: Option<Task<()>>,
    // ---- the microphone's codec ----
    reopening: Option<Reopening>,
    reopen_timer: Option<Task<()>>,
    went_down: bool,
    /// The rates the headphones allow, kept while the stream is set up again
    last_mic_rates: Vec<u32>,
    last_auto: Option<bool>,
    // ---- the device's order (issue #86 round 8) ----
    /// The device's order while it is being dragged (None: the saved one)
    drag_order: Option<Vec<&'static str>>,
    last_order: Vec<String>,
    /// Not `busy`: the card moved with the keyboard keeps the focus
    ordering: bool,
    dragging: Option<&'static str>,
    focus: HashMap<&'static str, FocusHandle>,
    /// The grid's width, for its columns (`.codec-grid`: auto-fill)
    grid_width: Rc<Cell<f32>>,
    /// A card's smallest width in the language shown (card_min_w), as of the
    /// last draw
    card_min_w: f32,
}

impl DeviceCodecs {
    fn new(key: String, audio_devices: Rc<Vec<AudioDevice>>, cx: &mut Context<Self>) -> DeviceCodecs {
        let core = app::core(cx);
        cx.observe(&core, |this: &mut Self, _, cx| {
            this.on_core(cx);
            cx.notify();
        })
        .detach();
        let settle = settle_state(cx);
        cx.observe(&settle, |_, _, cx| cx.notify()).detach();
        let mut this = DeviceCodecs {
            core,
            entry: EntryByKey::new(key.clone()),
            key,
            current: None,
            audio_devices,
            busy: false,
            queue: VecDeque::new(),
            saving: false,
            pending: None,
            save_timer: None,
            reopening: None,
            reopen_timer: None,
            went_down: false,
            last_mic_rates: Vec::new(),
            last_auto: None,
            drag_order: None,
            last_order: Vec::new(),
            ordering: false,
            dragging: None,
            focus: HashMap::new(),
            grid_width: Rc::new(Cell::new(0.)),
            card_min_w: 152.,
        };
        this.on_core(cx);
        this
    }

    fn set_audio_devices(&mut self, devices: Rc<Vec<AudioDevice>>, cx: &mut Context<Self>) {
        self.audio_devices = devices;
        cx.notify();
    }

    /// What follows the core: the entry, a saved order replacing the one
    /// dragged, the microphone's link opening again, Auto-select switched
    fn on_core(&mut self, cx: &mut Context<Self>) {
        let core = self.core.clone();
        let Some(snap) = core.read(cx).snap() else { return };
        let Some(e) = self.entry.get(&audio_entries(snap)).cloned() else { return };
        let shown = shown_profile(snap, &e);
        // A saved order replaces the one dragged
        if shown.codec_order != self.last_order {
            self.last_order = shown.codec_order.clone();
            self.drag_order = None;
        }
        let mic = mic_view(snap, &e);
        if self.reopening.is_some() {
            // A classic stream stays while its call link opens again: gone, it stopped
            if !e.le_audio && !mic.ours {
                self.end_reopen();
            } else if !mic.up(e.le_audio) {
                self.went_down = true;
            } else if self.went_down {
                self.end_reopen();
            }
        }
        if !mic.mic_rates.is_empty() {
            self.last_mic_rates = mic.mic_rates.clone();
        }
        // Auto-select switched while the microphone runs with a codec chosen:
        // it opens again (Auto's or the one chosen, as the core)
        let auto = snap.settings.auto_codec;
        if self.last_auto.is_some_and(|a| a != auto) {
            let chosen = if e.le_audio { shown.mic_rate != 0 } else { !shown.voice_codec.is_empty() };
            if mic.up(e.le_audio) && chosen {
                self.await_reopen(mic.call_codec.clone().unwrap_or_default(), mic.le_mic_rate.unwrap_or(0), cx);
            }
        }
        self.last_auto = Some(auto);
        self.current = Some(e);
    }

    fn locked(&self, cx: &App) -> bool {
        self.busy || settling(cx) || self.reopening.is_some()
    }

    /// Runs `job` once the device's earlier saves are done
    fn queued(&mut self, job: Job, cx: &mut Context<Self>) -> Task<Option<String>> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.queue.push_back((job, tx));
        self.pump(cx);
        cx.background_executor().spawn(async move { rx.await.ok().flatten() })
    }

    fn pump(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let Some((job, tx)) = self.queue.pop_front() else { return };
        self.saving = true;
        let task = job(cx);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let _ = tx.send(task.await);
            let _ = this.update(cx, |this, cx| {
                this.saving = false;
                this.pump(cx);
            });
        })
        .detach();
    }

    /// The entry as it is when a queued save runs
    fn fresh(key: &str, fallback: &AudioEntry, cx: &App) -> AudioEntry {
        app::core(cx)
            .read(cx)
            .snap()
            .and_then(|s| audio_entries(s).into_iter().find(|e| e.key == key))
            .unwrap_or_else(|| fallback.clone())
    }

    /// Chooses a card (`values`: from its "..." dialog; `live`: only the
    /// bitpools changed, which the encoder takes without starting again)
    fn choose(&mut self, id: &'static str, values: Option<CodecChoice>, live: bool, cx: &mut Context<Self>) {
        let core = self.core.clone();
        let Some(snap) = core.read(cx).snap() else { return };
        let Some(e) = self.current.clone() else { return };
        let auto = snap.settings.auto_codec;
        let shown = shown_profile(snap, &e);
        let order = self.order(snap, &e);
        let picked = picked_codec(snap, &e, auto, &order);
        let forced = forced_for(snap, &e, &shown, picked.as_deref(), id);
        // The device's stream starts again with the codec: give it time first
        let state = entry_state(snap, &e);
        let running = state != StreamState::Idle && state != StreamState::Error;
        // An order moved but not back from the core yet goes with it
        let drag = if auto { None } else { self.drag_order.clone() };
        let key = self.key.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            // A "..." dialog's change while the cards wait: applied after (the
            // cards themselves are disabled meanwhile)
            until_settled(cx).await;
            let Ok(task) = this.update(cx, |this, cx| {
                this.busy = true;
                cx.notify();
                this.queued(
                    Box::new(move |cx: &mut App| {
                        let e = Self::fresh(&key, &e, cx);
                        choose_codec(&e, id, values.as_ref(), drag.as_deref(), forced, cx)
                    }),
                    cx,
                )
            }) else {
                return;
            };
            let saved = task.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                cx.notify();
                if saved.is_some() && running && !live {
                    settle(cx);
                }
            });
        })
        .detach();
    }

    /// Its cards' order, which Auto follows under Auto-select too
    fn order(&self, snap: &ipc::Snapshot, e: &AudioEntry) -> Vec<&'static str> {
        self.drag_order.clone().unwrap_or_else(|| card_order(&shown_profile(snap, e).codec_order))
    }

    // ---- a "..." dialog's changes ----

    fn apply(&mut self, values: CodecChoice, cx: &mut Context<Self>) {
        let core = self.core.clone();
        let Some(snap) = core.read(cx).snap() else { return };
        let Some(e) = self.current.clone() else { return };
        let p = shown_profile(snap, &e);
        let id = codec_spec(&values.codec).id;
        let mine = id == card_of(&p.codec, &p.quality);
        let order = self.order(snap, &e);
        let picked = picked_codec(snap, &e, snap.settings.auto_codec, &order);
        if mine || picked.as_deref() == Some(id) {
            let live = mine && only_bitpools(&p, &values);
            self.choose(id, Some(values), live, cx);
        } else {
            let key = self.key.clone();
            let kept = values_of(&values);
            self.queued(
                Box::new(move |cx: &mut App| {
                    let e = Self::fresh(&key, &e, cx);
                    save_entry(
                        &e,
                        move |p| {
                            p.codec_settings.insert(id.to_owned(), kept);
                        },
                        cx,
                    )
                }),
                cx,
            )
            .detach();
        }
    }

    fn on_settings_change(&mut self, values: CodecChoice, cx: &mut Context<Self>) {
        self.pending = Some(values);
        self.save_timer = Some(cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor().timer(Duration::from_millis(SAVE_DELAY_MS)).await;
            let _ = this.update(cx, |this, cx| this.save_now(cx));
        }));
    }

    fn save_now(&mut self, cx: &mut Context<Self>) {
        self.save_timer = None;
        if let Some(values) = self.pending.take() {
            self.apply(values, cx);
        }
    }

    /// What a codec's dialog goes by
    fn settings_input(&self, cx: &App) -> Option<SettingsInput> {
        let snap = self.core.read(cx).snap()?;
        let e = self.current.as_ref()?;
        let p = shown_profile(snap, e);
        let s = &snap.settings;
        // The format Windows plays its source in (one app: the default
        // output, as the core reads it)
        let d = if p.capture_mode == "app" {
            self.audio_devices.iter().find(|x| x.is_default)
        } else {
            capture_device(&p.capture_mode, &p.audio_device_id, &self.audio_devices)
        };
        let source_format = d.and_then(|d| Some((d.mix_rate?, u32::from(d.mix_bits?))));
        let dev = DevMode::from(s);
        Some(SettingsInput {
            caps: caps_for_settings(known_codec_caps(&snap.devices, &e.address), dev),
            sbc_caps: sbc_caps_for_settings(known_sbc_caps(&snap.devices, &e.address), dev),
            developer: s.developer_mode,
            match_source: s.developer_mode && s.dev_overrides.match_source,
            source_format,
            device_name: e.name.clone(),
            address: e.address.clone(),
            warn_unlisted: !s.unlisted_options_warned,
            profile: p,
        })
    }

    fn open_settings(weak: WeakEntity<Self>, id: &'static str, window: &mut Window, cx: &mut App) {
        let w = weak.clone();
        let source = Rc::new(move |cx: &App| w.upgrade().and_then(|this| this.read(cx).settings_input(cx)));
        let w = weak.clone();
        let on_change = Rc::new(move |values: CodecChoice, cx: &mut App| {
            let _ = w.update(cx, |this, cx| this.on_settings_change(values, cx));
        });
        open_codec_settings(
            id,
            source,
            on_change,
            move |cx| {
                let _ = weak.update(cx, |this, cx| this.save_now(cx));
            },
            window,
            cx,
        );
    }

    // ---- the microphone's codec ----

    fn end_reopen(&mut self) {
        self.reopen_timer = None;
        self.reopening = None;
    }

    /// The microphone's codec chosen (or Auto-select switched) closes the
    /// call link and opens it again with A2DP in between, or sets the LE
    /// Audio stream up again: its cards stay until it is back (issue #86: the
    /// music cards flashed up meanwhile, the LE microphone's "..." closed)
    fn await_reopen(&mut self, to: String, rate: u32, cx: &mut Context<Self>) {
        self.went_down = false;
        self.reopening = Some(Reopening { to, rate });
        self.reopen_timer = Some(cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor().timer(Duration::from_millis(REOPEN_WAIT_MS)).await;
            // Not closed soon: nothing to wait for (it kept what it had)
            let Ok(down) = this.update(cx, |this, _| this.went_down) else { return };
            if down {
                cx.background_executor().timer(Duration::from_millis(SETTLE_MAX_MS)).await;
            }
            let _ = this.update(cx, |this, cx| {
                this.reopening = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// The microphone's codec chosen (issue #86): the call link (or the LE
    /// stream) opens again with it
    fn save_mic(&mut self, voice_codec: Option<String>, mic_rate: Option<u32>, cx: &mut Context<Self>) {
        let core = self.core.clone();
        let Some(snap) = core.read(cx).snap() else { return };
        let Some(e) = self.current.clone() else { return };
        let mic = mic_view(snap, &e);
        self.busy = true;
        if mic.up(e.le_audio) {
            self.await_reopen(voice_codec.clone().unwrap_or_default(), mic.le_mic_rate.unwrap_or(0), cx);
        }
        let key = self.key.clone();
        let task = self.queued(
            Box::new(move |cx: &mut App| {
                let e = Self::fresh(&key, &e, cx);
                let task = save_mic_codec(
                    &e,
                    move |p| {
                        if let Some(c) = voice_codec {
                            p.voice_codec = c;
                        }
                        if let Some(r) = mic_rate {
                            p.mic_rate = r;
                        }
                    },
                    cx,
                );
                cx.spawn(async move |_: &mut AsyncApp| task.await.then(String::new))
            }),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let ok = task.await.is_some();
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                if !ok {
                    this.end_reopen();
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn save_low_latency(&mut self, on: bool, cx: &mut Context<Self>) {
        let core = self.core.clone();
        let Some(snap) = core.read(cx).snap() else { return };
        let Some(e) = self.current.clone() else { return };
        if e.profile.is_some() && shown_profile(snap, &e).le_low_latency == on {
            return;
        }
        self.busy = true;
        let key = self.key.clone();
        let task = self.queued(
            Box::new(move |cx: &mut App| {
                let e = Self::fresh(&key, &e, cx);
                save_entry(&e, move |p| p.le_low_latency = on, cx)
            }),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            task.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    // ---- the device's order (issue #86 round 8) ----

    fn save_order(&mut self, next: Vec<&'static str>, cx: &mut Context<Self>) {
        let core = self.core.clone();
        let Some(snap) = core.read(cx).snap() else { return };
        let Some(e) = self.current.clone() else { return };
        let ids: Vec<&'static str> =
            device_cards(snap, &e, hide(), snap.settings.auto_codec, self.drag_order.as_deref())
                .cards
                .iter()
                .map(|c| c.id)
                .collect();
        // The cards shown back in their default order: the default order, the
        // hidden ones too (issue #86: no Reset for an order that changes nothing)
        let next = if shown_in_default_order(&next, &ids) { CARD_ORDER.to_vec() } else { next };
        if next == card_order(&shown_profile(snap, &e).codec_order) {
            self.drag_order = None;
            cx.notify();
            return;
        }
        self.drag_order = Some(next.clone());
        self.ordering = true;
        let key = self.key.clone();
        let task = self.queued(
            Box::new(move |cx: &mut App| {
                let e = Self::fresh(&key, &e, cx);
                save_codec_order(&e, &next, cx)
            }),
            cx,
        );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let ok = task.await.is_some();
            let _ = this.update(cx, |this, cx| {
                this.ordering = false;
                if !ok {
                    this.drag_order = None;
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Alt+arrow keys: next to the card before or after it among those shown
    fn move_by(&mut self, id: &'static str, by: i32, cx: &mut Context<Self>) {
        let core = self.core.clone();
        let Some(snap) = core.read(cx).snap() else { return };
        let Some(e) = self.current.clone() else { return };
        let view = device_cards(snap, &e, hide(), snap.settings.auto_codec, self.drag_order.as_deref());
        let ids: Vec<&'static str> = view.cards.iter().map(|c| c.id).collect();
        if let Some((next, _)) = moved_by(&view.order, &ids, id, by) {
            self.save_order(next, cx);
        }
    }

    /// The card dragged is over `over`: it takes that place (hidden cards
    /// keep theirs)
    fn drag_over(&mut self, id: &'static str, over: &'static str, cx: &mut Context<Self>) {
        if self.dragging.is_none() {
            self.dragging = Some(id);
        }
        if over == id {
            return;
        }
        let core = self.core.clone();
        let Some(snap) = core.read(cx).snap() else { return };
        let Some(e) = self.current.clone() else { return };
        let order = self.order(snap, &e);
        self.drag_order = Some(placed(&order, id, over));
        cx.notify();
    }

    /// The card let go: the order is saved where it is
    fn end_drag(&mut self, cx: &mut Context<Self>) {
        self.dragging = None;
        let Some(order) = self.drag_order.clone() else { return };
        self.save_order(order, cx);
    }

    // ---- drawing ----

    fn head(&self, e: &AudioEntry, state: StreamState, cx: &App) -> Div {
        let p = *cx.palette();
        // Which device is in use (issue #86 round 8), as the sidebar's dot;
        // never red here, it looked like recording (issue #86)
        let state = if state == StreamState::Error { StreamState::Idle } else { state };
        let name = if e.name.is_empty() { t("device.name_unknown") } else { e.name.clone() };
        div()
            .flex()
            .items_center()
            .gap(theme::S3)
            // A line between the device and its codecs
            .pb(theme::S3)
            .border_b_1()
            .border_color(p.border)
            .child(
                div()
                    .id(SharedString::from(format!("dot-{}", self.key)))
                    .role(Role::Image)
                    .aria_label(state_label(state))
                    .tooltip(crate::ui::stream::tip(state_label(state)))
                    .child(dot(state_colors(state, &p).0).size(px(10.))),
            )
            .child(div().flex_none().text_color(p.text_2).child(icon(IconName::Headphones).size(px(24.))))
            .child(
                div().flex().flex_col().flex_1().min_w_0().child(
                    div()
                        .text_size(theme::FS_LARGE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .line_clamp(2)
                        .text_ellipsis()
                        .child(name),
                ),
            )
            // No "Paired": that is the Audio Devices page's (issue #86); LE
            // Audio: that tag alone
            .child(
                div()
                    .flex()
                    .gap(theme::S1)
                    .when(e.device.audio && !e.le_audio, |d| d.child(tag(t("device.tag_audio"), cx)))
                    .when(e.le_audio, |d| d.child(tag(t("device.tag_le_audio"), cx))),
            )
    }

    /// The grid of cards, its columns as wide as `.codec-grid` makes them
    fn grid(&self, cells: Vec<AnyElement>, cx: &mut Context<Self>) -> Div {
        let width = self.grid_width.clone();
        let id = cx.entity_id();
        let w = self.grid_width.get();
        let gap = 8.;
        let cols = if w > 0. { (((w + gap) / (self.card_min_w + gap)).floor() as u16).max(1) } else { 3 };
        div()
            .relative()
            .w_full()
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let w = f32::from(bounds.size.width);
                        if (w - width.get()).abs() > 0.5 {
                            width.set(w);
                            cx.notify(id);
                            window.refresh();
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(div().grid().grid_cols(cols).gap(theme::S2).children(cells))
    }

    fn cell_width(&self) -> Pixels {
        let w = self.grid_width.get();
        let gap = 8.;
        let cols = if w > 0. { ((w + gap) / (self.card_min_w + gap)).floor().max(1.) } else { 3. };
        px(if w > 0. { (w - gap * (cols - 1.)) / cols } else { self.card_min_w })
    }

    /// The music codec cards with their order hint and Reset
    fn music(&mut self, snap: &ipc::Snapshot, e: &AudioEntry, window: &Window, cx: &mut Context<Self>) -> Div {
        let auto = snap.settings.auto_codec;
        let hide = hide();
        let view = device_cards(snap, e, hide, auto, self.drag_order.as_deref());
        let locked = self.locked(cx);
        let weak = cx.entity().downgrade();
        let keeps_settings = auto_keeps_codec_settings(DevMode::from(&snap.settings));
        let width = self.cell_width();
        let mut cells = Vec::new();
        for c in &view.cards {
            let id = c.id;
            let spec = CardSpec { id, name: t(c.label).into(), bars: c.bars };
            let focus = self.focus.entry(id).or_insert_with(|| cx.focus_handle()).clone();
            let state = CardState {
                selected: c.selected,
                disabled: locked,
                read_only: auto,
                support: c.support,
                orderable: true,
                dragging: self.dragging == Some(id),
                focused: !locked && !auto && focus.is_focused(window),
            };
            let (w1, w2, w3, w4) = (weak.clone(), weak.clone(), weak.clone(), weak.clone());
            let actions = CardActions {
                on_pick: Rc::new(move |_, cx| {
                    let _ = w1.update(cx, |this, cx| this.choose(id, None, false, cx));
                }),
                on_read_only: Rc::new(move |settings, window, cx| {
                    // Developer mode: the codec's settings apply under Auto-select
                    if settings && keeps_settings {
                        Self::open_settings(w2.clone(), id, window, cx);
                        return;
                    }
                    let w = w2.clone();
                    ask_auto_off(
                        Rc::new(move |window, cx| {
                            if settings {
                                Self::open_settings(w.clone(), id, window, cx);
                            } else {
                                let _ = w.update(cx, |this, cx| this.choose(id, None, false, cx));
                            }
                        }),
                        window,
                        cx,
                    );
                }),
                on_settings: Rc::new(move |window, cx| Self::open_settings(w3.clone(), id, window, cx)),
                on_move: Some(Rc::new(move |by, _, cx| {
                    let _ = w4.update(cx, |this, cx| this.move_by(id, by, cx));
                })),
            };
            // The keys need a card that takes the focus (not under Auto-select)
            let description = (!auto).then(|| SharedString::from(t("ui.codecs.order_keys")));
            let cell_id: SharedString = format!("codec-{}-{id}", self.key).into();
            let mut cell = codec_card(cell_id, &spec, state, actions, Some(&focus), description, cx);
            let over = weak.clone();
            let device = self.key.clone();
            cell = cell.on_drag_move(move |ev: &DragMoveEvent<DraggedCodec>, _, cx| {
                if !ev.bounds.contains(&ev.event.position) {
                    return;
                }
                let dragged = ev.drag(cx);
                if dragged.device != device {
                    return;
                }
                let from = dragged.id;
                let _ = over.update(cx, |this, cx| this.drag_over(from, id, cx));
            });
            // The card dragged is lifted and follows the pointer; the others
            // take their new places as it passes (a click without moving
            // still chooses)
            if !locked {
                let state = CardState { focused: false, ..state };
                let dragged = DraggedCodec { device: self.key.clone(), id, spec: spec.clone(), state, width };
                let start = weak.clone();
                cell = cell.on_drag(dragged, move |d, _, _, cx| {
                    let id = d.id;
                    let _ = start.update(cx, |this, cx| {
                        this.dragging = Some(id);
                        cx.notify();
                    });
                    let d = d.clone();
                    cx.new(|_| d)
                });
            }
            cells.push(cell.into_any_element());
        }
        let reset = view.reset;
        let busy = self.busy || self.ordering;
        div()
            .flex()
            .flex_col()
            .gap(theme::S3)
            .child(self.grid(cells, cx).on_drop(cx.listener(|this, d: &DraggedCodec, _, cx| {
                if d.device == this.key {
                    this.end_drag(cx);
                }
            })))
            // Visible hint for dragging (the cards' description has the keys
            // too); Reset beside it while the order is changed
            .when(view.cards.len() > 1, |d| {
                d.child(ui::inline().child(ui::hint(t("ui.codecs.order_hint"), cx)).when(reset, |d| {
                    d.child(
                        ui::button(
                            SharedString::from(format!("order-reset-{}", self.key)),
                            t("ui.codecs.order_reset"),
                            ButtonKind::Default,
                        )
                        .icon(icon(IconName::ArrowReset))
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.save_order(CARD_ORDER.to_vec(), cx))),
                    )
                }))
            })
            .when(view.unknown, |d| d.child(ui::hint(t("ui.profile.codecs_unknown"), cx)))
    }

    /// Cards that only show their codec (`FixedCards`): the call codecs and
    /// LE Audio's, their "..." read only
    #[allow(clippy::too_many_arguments)]
    fn fixed_cards(
        &self,
        group: &str,
        cards: &[(FixedCard, Option<ui_model::codecs::CodecSupport>)],
        picked: &str,
        read_only: bool,
        pick: Option<CardAct>,
        on_locked: Option<CardAct>,
        cx: &mut Context<Self>,
    ) -> Div {
        let locked = self.locked(cx);
        let weak = cx.entity().downgrade();
        let cells = cards
            .iter()
            .map(|(c, support)| {
                let card = *c;
                let spec = CardSpec { id: c.id, name: t(c.label).into(), bars: Some(c.bars) };
                let state = CardState {
                    selected: picked == c.id,
                    disabled: locked,
                    read_only: read_only || pick.is_none(),
                    support: *support,
                    ..CardState::default()
                };
                let (pick, on_locked) = (pick.clone(), on_locked.clone());
                let (w1, w2) = (weak.clone(), weak.clone());
                let actions = CardActions {
                    on_pick: Rc::new(move |window, cx| {
                        if let Some(f) = &pick {
                            f(card.id, window, cx);
                        }
                    }),
                    on_read_only: Rc::new(move |settings, window, cx| {
                        if settings {
                            Self::open_fixed(w1.clone(), card, window, cx);
                        } else if let Some(f) = &on_locked {
                            f(card.id, window, cx);
                        }
                    }),
                    on_settings: Rc::new(move |window, cx| Self::open_fixed(w2.clone(), card, window, cx)),
                    on_move: None,
                };
                let focus = None;
                codec_card(
                    format!("fixed-{}-{group}-{}", self.key, c.id).into(),
                    &spec,
                    state,
                    actions,
                    focus,
                    None,
                    cx,
                )
                .into_any_element()
            })
            .collect();
        self.grid(cells, cx)
    }

    /// The card a fixed group shows chosen
    fn fixed_picked(&self, snap: &ipc::Snapshot, e: &AudioEntry, card: &FixedCard) -> String {
        let shown = shown_profile(snap, e);
        if card.id == LE_MIC_CARD.id {
            return card.id.to_owned();
        }
        if e.le_audio {
            return if shown.le_low_latency { "lc3_ll" } else { "lc3" }.to_owned();
        }
        let mic = mic_view(snap, e);
        mic.call_codec.or_else(|| self.reopening.as_ref().map(|r| r.to.clone())).unwrap_or_default()
    }

    /// A read-only card's "...": as an A2DP codec's settings, read only, with
    /// the same descriptions (issue #86); the LE Audio microphone's rate is
    /// chosen there with Auto-select off
    fn open_fixed(weak: WeakEntity<Self>, card: FixedCard, window: &mut Window, cx: &mut App) {
        let title = t!("ui.profile.codec_settings", t(card.label));
        dialog::ask(
            Question::new(title).cancel(t("modal.close")),
            move |_, cx| {
                let Some(this) = weak.upgrade() else { return div().into_any_element() };
                let this = this.read(cx);
                let Some(snap) = this.core.read(cx).snap() else { return div().into_any_element() };
                let Some(e) = this.current.as_ref() else { return div().into_any_element() };
                let mic = mic_view(snap, e);
                let rate_shown = mic.le_mic_rate.or_else(|| this.reopening.as_ref().map(|r| r.rate).filter(|r| *r > 0));
                let card = if card.id == LE_MIC_CARD.id { FixedCard { rate: rate_shown, ..card } } else { card };
                let picked = this.fixed_picked(snap, e, &card);
                // LE Audio's rate and bit rate are the stream's, while it plays
                let st = &snap.stream;
                let info = st.info.as_ref().filter(|_| {
                    st.state == StreamState::Streaming && st.profile == e.profile.as_ref().map(|p| p.name.clone())
                });
                let (rate, kbps) = fixed_values(&card, &picked, info);
                let shown = shown_profile(snap, e);
                let rates = if mic.mic_rates.is_empty() && this.reopening.is_some() {
                    this.last_mic_rates.clone()
                } else {
                    mic.mic_rates.clone()
                };
                let choice = (card.id == LE_MIC_CARD.id && !snap.settings.auto_codec)
                    .then(|| mic_rate_options(shown.mic_rate, &rates))
                    .flatten();
                let locked = this.locked(cx);
                let mono = card.id == LE_MIC_CARD.id || !e.le_audio;
                let w = weak.clone();
                let rate_control: AnyElement = match choice {
                    Some(options) => ui::select_labelled(
                        "mic-rate",
                        t("connection.sample_rate"),
                        &shown.mic_rate,
                        options.into_iter().map(|o| Choice::new(o.value, o.label)).collect(),
                        locked,
                        move |r: &u32, _, cx| {
                            let r = *r;
                            let _ = w.update(cx, |this, cx| this.save_mic(None, Some(r), cx));
                        },
                    )
                    .into_any_element(),
                    None => value_box(rate, cx).into_any_element(),
                };
                div()
                    .flex()
                    .flex_col()
                    .gap(theme::S3)
                    .when(!card.no_bit_rate, |d| {
                        d.child(ui::field(
                            t("connection.quality"),
                            Some(t("tooltip.quality").into()),
                            None,
                            value_box(kbps, cx),
                            cx,
                        ))
                    })
                    .child(ui::field(
                        t("connection.sample_rate"),
                        Some(t("tooltip.sample_rate").into()),
                        None,
                        rate_control,
                        cx,
                    ))
                    // LC3 and the call codecs carry 16-bit samples
                    .child(ui::field(
                        t("connection.bit_depth"),
                        Some(t("tooltip.bit_depth").into()),
                        None,
                        value_box(t("bitdepth.16"), cx),
                        cx,
                    ))
                    // The call link and the microphone are one channel
                    .when(mono, |d| {
                        d.child(ui::field(
                            t("ui.diag.cd_channel_mode"),
                            Some(t("tooltip.channel_mode").into()),
                            None,
                            value_box(t("ui.diag.cd_mode.mono"), cx),
                            cx,
                        ))
                    })
                    .into_any_element()
            },
            |_, _| {},
            window,
            cx,
        );
    }

    /// The LE Audio cards, or the call codecs while the call link is open
    fn microphone(&mut self, snap: &ipc::Snapshot, e: &AudioEntry, cx: &mut Context<Self>) -> Div {
        let p = *cx.palette();
        let auto = snap.settings.auto_codec;
        let mic: MicView = mic_view(snap, e);
        let shown = shown_profile(snap, e);
        let weak = cx.entity().downgrade();
        let w = weak.clone();
        let pick: CardAct = if e.le_audio {
            Rc::new(move |id, _, cx| {
                let _ = w.update(cx, |this, cx| this.save_low_latency(id == "lc3_ll", cx));
            })
        } else {
            Rc::new(move |id, _, cx| {
                let _ = w.update(cx, |this, cx| this.save_mic(Some(id.to_owned()), None, cx));
            })
        };
        let w = weak.clone();
        let on_locked: CardAct = Rc::new(move |id, window, cx| {
            let w = w.clone();
            ask_auto_off(
                Rc::new(move |_, cx| {
                    let _ = w.update(cx, |this, cx| this.save_mic(Some(id.to_owned()), None, cx));
                }),
                window,
                cx,
            );
        });
        let cards: Vec<(FixedCard, Option<ui_model::codecs::CodecSupport>)> = if e.le_audio {
            LE_CARDS.iter().map(|c| (*c, None)).collect()
        } else {
            call_cards(&mic.listed_calls, mic.call_codec.as_deref(), hide())
        };
        // LE Audio: LC3 unless LC3 LL is chosen (issue #86)
        let picked = if e.le_audio {
            if shown.le_low_latency { "lc3_ll".to_owned() } else { "lc3".to_owned() }
        } else {
            mic.call_codec.clone().or_else(|| self.reopening.as_ref().map(|r| r.to.clone())).unwrap_or_default()
        };
        // LE Audio: LC3 or its low latency configuration, from the device's
        // next connection, under Auto-select too; the call codec only with
        // Auto-select off (issue #86)
        let read_only = !e.le_audio && auto;
        let group = self.fixed_cards("main", &cards, &picked, read_only, Some(pick), Some(on_locked), cx);
        let le_support = snap.adapter.le_audio;
        // The LE Audio microphone while it runs: its rate in its "..."
        let mic_rate = mic
            .le_mic_rate
            .or_else(|| if e.le_audio { self.reopening.as_ref().map(|r| r.rate).filter(|r| *r > 0) } else { None });
        let mic_cards = mic_rate.map(|rate| {
            let card = FixedCard { rate: Some(rate), ..LE_MIC_CARD };
            self.fixed_cards("mic", &[(card, None)], LE_MIC_CARD.id, true, None, None, cx)
        });
        div()
            .flex()
            .flex_col()
            .gap(theme::S3)
            .child(group)
            .when(e.le_audio && le_support != Some(true), |d| {
                let text = if le_support == Some(false) {
                    t("ui.profile.le_audio_unsupported")
                } else {
                    t("ui.profile.le_audio_unchecked")
                };
                d.child(ui::banner(BannerKind::Warn, None, Some(div().child(text).into_any_element()), None, None, cx))
            })
            .when_some(mic_cards, |d, cards| {
                d.child(
                    div()
                        .text_size(theme::FS_BODY)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.text_2)
                        .child(t("ui.diag.mic_title")),
                )
                .child(cards)
            })
    }
}

/// "Show only supported codecs", on unless turned off (kept across starts
/// in ui_state)
fn hide() -> bool {
    crate::ui_state::hide_unsupported()
}

impl Render for DeviceCodecs {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The card let go (anywhere): its order is saved
        if self.dragging.is_some() && !cx.has_active_drag() {
            self.end_drag(cx);
        }
        self.card_min_w = card_min_w(window);
        let Some(snap) = self.core.read(cx).snap().cloned() else { return div() };
        let snap = &snap;
        let Some(e) = self.current.clone() else { return div() };
        let state = entry_state(snap, &e);
        let mic = mic_view(snap, &e);
        // The call link (HFP) open: its codecs instead of the music ones,
        // which do not play meanwhile (issue #86)
        let call_open = !e.le_audio && (mic.call_codec.is_some() || self.reopening.is_some());
        let body =
            if !e.le_audio && !call_open { self.music(snap, &e, window, cx) } else { self.microphone(snap, &e, cx) };
        ui::card(cx).min_w_0().child(self.head(&e, state, cx)).child(body)
    }
}

#[cfg(test)]
mod interaction_tests;

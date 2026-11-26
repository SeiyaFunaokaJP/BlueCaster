//! Home: a card per saved audio device, side by side (issue #86), each with
//! its connection ring, state and codec and its own Connect / Disconnect.
//! One device streams at a time: Connect on another one switches to it (the
//! core stops the running stream first). The volume is on the card of the
//! device in use, as each device keeps its own, and disabled on the others
//! (issue #86); its codec is chosen on its card (e.g. one for games and one
//! for music), and its link quality shows there while it streams. The
//! capture source is chosen on Audio Devices.
//!
//! The decisions are in `ui_model::view::home`, tested there. The pieces
//! other screens show too (entry state, microphone row, headset chip, hung
//! adapter, v1 running) are in `ui/stream.rs`; [`codec_picker`] is here
//! (the flyout shows it too).

use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::Mutex;

use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, AsyncApp, Context, Entity, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Task, WeakEntity, Window,
    div, px, relative,
};
use ipc::{ErrorInfo, Method, Snapshot, StreamState};
use ui_model::codecs::codec_spec;
use ui_model::i18n::t;
use ui_model::profile::{AudioEntry, EntryByKey, audio_entries};
use ui_model::view::home::{
    self as model, CardPress, CodecPick, FirmwareBar, HiResBar, HomeError, card_press, codec_pick, device_card,
    firmware_bar, home_error, offers_add, other_radio_text, request_error_key,
};

use crate::app::{self, AdapterSection, Route, RouteName};
use crate::core::Core;
use crate::screens::codecs::{choose_codec, settle, settling};
use crate::theme::{self, ActivePalette as _};
use crate::ui::stream::{
    CodecBadge, LinkSparkline, OnError, QualityIndicator, VolumeControl, adapter_hung, battery_level, codec_badge,
    connect_button, dot, error_panel, firmware_download, firmware_folder_button, mic_row, ring_sized, state_colors,
    tip, v1_running,
};
use crate::ui::{self, BannerKind, ButtonKind, IconName, icon};

/// The firmware screen (Adapter, its firmware section)
fn to_firmware(cx: &mut App) {
    app::navigate(Route { name: RouteName::Adapter, arg: Some(AdapterSection::Firmware) }, cx);
}

/// The bar over Home while an adapter lacks its firmware files, or the
/// running stack found its Realtek adapter on the ROM code (issue #81)
fn firmware_missing(snap: &Snapshot, cx: &App) -> Option<AnyElement> {
    let p = *cx.palette();
    let fix = |kind: ButtonKind| {
        ui::button("firmware-fix", t("ui.home.firmware_fix"), kind)
            .icon(icon(IconName::UsbStick))
            .on_click(|_, _, cx| to_firmware(cx))
    };
    let bar = match firmware_bar(snap)? {
        FirmwareBar::Rom => ui::banner(
            BannerKind::Warn,
            Some(t("ui.adapter.rom_firmware").into()),
            Some(div().child(t("firmware.rom_status")).into_any_element()),
            Some(fix(ButtonKind::Strong).into_any_element()),
            None,
            cx,
        ),
        FirmwareBar::Missing { adapters, title_key, downloadable } => {
            let lines = div()
                .flex()
                .flex_col()
                .gap(theme::S1)
                .children(adapters.iter().map(|a| {
                    let what = match &a.firmware_invalid {
                        Some(file) => div().child(ui_model::t!("firmware.invalid", file)),
                        None => div()
                            .flex()
                            .flex_wrap()
                            .gap_x(theme::S1)
                            .child(t("firmware.required_files"))
                            .child(ui::mono(a.firmware_files.join(", ")).min_w_0().max_w_full()),
                    }
                    .min_w_0()
                    .max_w_full();
                    // Long names and file lists wrap inside the card
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_x(theme::S1)
                        .min_w_0()
                        .child(
                            div()
                                .min_w_0()
                                .max_w_full()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(a.display_name.clone()),
                        )
                        .child(div().text_color(p.text_2).child("·"))
                        .child(what)
                }))
                // Missing files and the ROM code at once: say it is not usable (issue #81)
                .when(snap.adapter.rom_firmware, |d| d.child(div().child(t("firmware.rom_short"))));
            // The Adapter screen's actions right here (issue #81): the
            // download, the folder for the files fetched by hand, the details
            let actions = ui::inline()
                .when_some(downloadable, |d, a| {
                    d.child(firmware_download("firmware-download", a.firmware_files.clone(), ButtonKind::Strong))
                })
                .children(firmware_folder_button("firmware-folder", ButtonKind::Default, cx))
                .child(fix(if downloadable.is_some() { ButtonKind::Default } else { ButtonKind::Strong }));
            ui::banner(
                BannerKind::Warn,
                Some(t(title_key).into()),
                Some(lines.into_any_element()),
                Some(actions.into_any_element()),
                None,
                cx,
            )
        }
    };
    Some(bar.into_any_element())
}

/// A banner's close button's action
type OnDismiss = Box<dyn Fn(&gpui_kit::ClickEvent, &mut Window, &mut App)>;

/// Another Bluetooth adapter is active (issue #86): said until dismissed,
/// which holds for good (across starts: crate::ui_state)
fn other_radio(snap: &Snapshot, cx: &App) -> Option<AnyElement> {
    if crate::ui_state::other_radio_dismissed() {
        return None;
    }
    let text = other_radio_text(snap)?;
    let dismiss: OnDismiss = Box::new(|_, _, cx| {
        crate::ui_state::set_other_radio_dismissed();
        cx.refresh_windows();
    });
    Some(
        ui::banner(
            BannerKind::Warn,
            None,
            Some(div().child(text).into_any_element()),
            None,
            Some(("other-radio-dismiss".into(), dismiss)),
            cx,
        )
        .into_any_element(),
    )
}

pub struct Home {
    core: Entity<Core>,
    /// A failed request (Connect) until the state moves on
    req_error: Option<ErrorInfo>,
    /// What clears it: the stream's state, the adapters' drivers
    req_error_key: (StreamState, String),
    hi_res: HiResBar,
    /// A card per device, by key: a device keeps its card (its volume, its
    /// link quality) across updates
    cards: Vec<Entity<DeviceCard>>,
}

impl Home {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Home {
        let core = app::core(cx);
        let req_error_key = core.read(cx).snap().map(request_error_key).unwrap_or_default();
        cx.observe_in(&core, window, |this: &mut Self, core, window, cx| {
            if let Some(snap) = core.read(cx).snap().cloned() {
                let key = request_error_key(&snap);
                if key != this.req_error_key {
                    this.req_error_key = key;
                    this.req_error = None;
                }
                this.hi_res.update(&snap);
                this.sync_cards(&snap, window, cx);
            }
            cx.notify();
        })
        .detach();
        let mut this =
            Home { core: core.clone(), req_error: None, req_error_key, hi_res: HiResBar::default(), cards: Vec::new() };
        if let Some(snap) = core.read(cx).snap().cloned() {
            this.sync_cards(&snap, window, cx);
        }
        this
    }

    /// One card per device, in the devices' order; a device that left loses its card
    fn sync_cards(&mut self, snap: &Snapshot, window: &mut Window, cx: &mut Context<Self>) {
        let keys: Vec<String> = audio_entries(snap).into_iter().map(|e| e.key).collect();
        let current: Vec<String> = self.cards.iter().map(|c| c.read(cx).key.clone()).collect();
        if keys == current {
            return;
        }
        let weak = cx.entity().downgrade();
        let on_error: OnError = Rc::new(move |e: ErrorInfo, cx: &mut App| {
            let _ = weak.update(cx, |this, cx| {
                this.req_error = Some(e);
                cx.notify();
            });
        });
        let mut old = std::mem::take(&mut self.cards);
        self.cards = keys
            .into_iter()
            .map(|key| match old.iter().position(|c| c.read(cx).key == key) {
                Some(i) => old.remove(i),
                None => {
                    let on_error = on_error.clone();
                    cx.new(|cx| DeviceCard::new(key, on_error, window, cx))
                }
            })
            .collect();
    }
}

impl Render for Home {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(snap) = self.core.read(cx).snap().cloned() else { return div().into_any_element() };
        let p = *cx.palette();
        let error = home_error(&snap, self.req_error.as_ref());
        let hi_res = self.hi_res.shown(&snap).then(|| {
            let weak = cx.entity().downgrade();
            let dismiss: OnDismiss = Box::new(move |_, _, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.hi_res.dismiss();
                    cx.notify();
                });
            });
            ui::banner(
                BannerKind::Warn,
                None,
                Some(div().child(t("ui.home.hi_res_set")).into_any_element()),
                None,
                Some(("hi-res-dismiss".into(), dismiss)),
                cx,
            )
        });
        let entries = audio_entries(&snap);

        let devices = if entries.is_empty() {
            ui::card(cx)
                .items_center()
                .py(theme::S8)
                .px(theme::S4)
                .text_color(p.text_2)
                .child(icon(IconName::Headphones).size(px(40.)))
                .child(div().child(t("ui.home.no_device")))
                .child(
                    ui::button("home-add-first", t("ui.home.add_device"), ButtonKind::Strong)
                        .icon(icon(IconName::Add))
                        .on_click(|_, _, cx| app::navigate(RouteName::Devices, cx)),
                )
                .into_any_element()
        } else {
            // A card per device side by side, at least 20rem wide, as many
            // to a row as fit (`.home-devices`' grid)
            let cell = || div().flex().flex_col().flex_grow(1.).flex_shrink(1.).flex_basis(px(320.)).min_w_0();
            let mut items: Vec<AnyElement> =
                self.cards.iter().map(|c| cell().child(c.clone()).into_any_element()).collect();
            // A lone card does not stretch over the page (issue #86)
            if offers_add(&entries) {
                items.push(
                    cell()
                        .child(
                            ui::button("home-add", t("ui.home.add_device"), ButtonKind::Subtle)
                                .flex_1()
                                .w_full()
                                .min_h(px(64.))
                                .p(theme::S4)
                                // An empty place, not a card: dashed and unfilled
                                .icon(icon(IconName::Add))
                                .border_1()
                                .border_dashed()
                                .border_color(p.border_strong)
                                .rounded(theme::RADIUS_CARD)
                                .text_color(p.text_2)
                                .on_click(|_, _, cx| app::navigate(RouteName::Devices, cx)),
                        )
                        .into_any_element(),
                );
            }
            // Empty cells keep the last row's cards as wide as the others
            let fillers = items.len().saturating_sub(1).min(2);
            div()
                .flex()
                .flex_wrap()
                .gap(theme::S4)
                .max_w(px(1024.))
                .children(items)
                .children((0..fillers).map(|_| cell().h(px(0.))))
                .into_any_element()
        };

        ui::page("home", t("ui.nav.home"))
            .children(v1_running(cx))
            .children(adapter_hung(cx))
            .children(
                error.map(|HomeError { code, message }| error_panel("home-error", code.as_deref(), &message, None, cx)),
            )
            .children(firmware_missing(&snap, cx))
            .children(other_radio(&snap, cx))
            .children(hi_res)
            .child(devices)
            .into_any_element()
    }
}

// ---------------------------------------------------------------------------
// A device's card
// ---------------------------------------------------------------------------

/// One device: ring, name, battery and state, codec, volume, link quality
/// while streaming, and its own button, all in one card (issue #86)
pub struct DeviceCard {
    key: String,
    entry: EntryByKey,
    on_error: OnError,
    busy: bool,
    /// Its own volume while it is the one in use, disabled on the others
    volume: Entity<VolumeControl>,
    /// The link quality while it streams
    link: Option<(Entity<QualityIndicator>, Entity<LinkSparkline>)>,
}

impl DeviceCard {
    fn new(key: String, on_error: OnError, window: &mut Window, cx: &mut Context<Self>) -> DeviceCard {
        let core = app::core(cx);
        let volume = cx.new(|cx| VolumeControl::new(true, window, cx));
        cx.observe(&core, |this: &mut Self, _, cx| {
            let live = this.card(cx).is_some_and(|(_, c)| c.live);
            this.volume.update(cx, |v, cx| v.set_off(!live, cx));
            cx.notify();
        })
        .detach();
        let mut this =
            DeviceCard { entry: EntryByKey::new(key.clone()), key, on_error, busy: false, volume, link: None };
        let live = this.card(cx).is_some_and(|(_, c)| c.live);
        this.volume.update(cx, |v, cx| v.set_off(!live, cx));
        this
    }

    /// The card's device and what it shows (None before the first state)
    fn card(&mut self, cx: &App) -> Option<(AudioEntry, model::DeviceCard)> {
        let snap = app::core(cx).read(cx).snap()?;
        let entry = self.entry.get(&audio_entries(snap))?.clone();
        let card = device_card(snap, &entry);
        Some((entry, card))
    }

    fn press(&mut self, cx: &mut Context<Self>) {
        let Some(snap) = app::core(cx).read(cx).snap().cloned() else { return };
        let Some((entry, _)) = self.card(cx) else { return };
        self.busy = true;
        cx.notify();
        let task = connect_entry(card_press(&snap, &entry), &entry, cx);
        let on_error = self.on_error.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                cx.notify();
            });
            if let Err(e) = result {
                cx.update(|cx| on_error(e, cx));
            }
        })
        .detach();
    }
}

/// Connect for a device: its profile (Auto made for it if it has none);
/// another device streaming gives way (stream.start stops it first)
fn connect_entry(press: CardPress, entry: &AudioEntry, cx: &App) -> Task<Result<(), ErrorInfo>> {
    let start = |profile: String, cx: &App| {
        let task = app::request::<serde_json::Value>(Method::StreamStart { profile }, cx);
        cx.spawn(async move |_: &mut AsyncApp| task.await.map(|_| ()))
    };
    match press {
        CardPress::Stop => {
            let task = app::request::<serde_json::Value>(Method::StreamStop, cx);
            cx.spawn(async move |_: &mut AsyncApp| task.await.map(|_| ()))
        }
        CardPress::Start(profile) => start(profile, cx),
        CardPress::StartAuto => {
            let saved = choose_codec(entry, "auto", None, None, false, cx);
            cx.spawn(async move |cx: &mut AsyncApp| match saved.await {
                Some(name) => cx.update(|cx| start(name, cx)).await,
                // Not saved: already told
                None => Ok(()),
            })
        }
    }
}

impl Render for DeviceCard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some((entry, card)) = self.card(cx) else { return div().into_any_element() };
        let Some(snap) = app::core(cx).read(cx).snap().cloned() else { return div().into_any_element() };
        let p = *cx.palette();
        let stream = &snap.stream;
        let key = self.key.clone();

        // The link quality while it streams: its views ask for the link's
        // samples only while on screen
        if card.state == StreamState::Streaming {
            if self.link.is_none() {
                let quality = cx.new(|cx| QualityIndicator::new(window, cx));
                let spark = cx.new(|cx| LinkSparkline::new(window, cx));
                self.link = Some((quality, spark));
            }
        } else {
            self.link = None;
        }

        let (dot_color, text_color) = state_colors(card.state, &p);
        let ring = ring_sized(
            SharedString::from(format!("ring-{key}")),
            card.state,
            if card.live { stream.volume } else { None },
            88.,
            false,
            icon(IconName::Headphones).size(px(35.)),
            cx,
        );
        // The device, battery and state, the codec; the sidebar shows the same
        let name = div()
            .id(SharedString::from(format!("name-{key}")))
            .font_family(theme::FONT_DISPLAY)
            .text_size(px(22.))
            .font_weight(FontWeight::SEMIBOLD)
            .line_height(relative(1.2))
            .line_clamp(2)
            .text_ellipsis()
            .min_w_0()
            .child(card.name.clone())
            // The whole name on hover when two lines cut it
            .when(card.name.chars().count() > 36, |d| d.tooltip(tip(card.name.clone())));
        let state_line = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(theme::S2)
            .min_w_0()
            .when(card.battery, |d| {
                d.child(battery_level(SharedString::from(format!("battery-{key}")), stream.battery, false, cx))
                    .child(div().text_color(p.text_2).child("|"))
            })
            .child(
                div()
                    .flex()
                    // The dot beside the first line
                    .items_start()
                    .gap(theme::S2)
                    .min_w_0()
                    // No wider than the card: a long state goes onto more lines
                    .max_w_full()
                    .text_color(text_color)
                    // No red dot for an error: it read as recording (issue #86)
                    .when(card.dot, |d| d.child(div().flex_none().pt(px(8.)).child(dot(dot_color))))
                    // An error's text is in the panel above, with its fix
                    .child(div().min_w_0().child(card.status.clone())),
            );
        let codec = card
            .live
            .then(|| {
                codec_badge(
                    CodecBadge { picker: Some(codec_picker(&entry, cx)), stretch: true, ..CodecBadge::default() },
                    cx,
                )
            })
            .flatten();
        let mic = card.live.then(|| mic_row(&entry, cx)).flatten();
        // The application it captures is not playing yet (the source is
        // chosen on Audio Devices)
        let waiting = card.waiting.as_ref().map(|w| {
            div()
                .id(SharedString::from(format!("capture-{key}")))
                .flex()
                .items_center()
                .gap(theme::S1)
                .min_w_0()
                .text_color(p.warn_text)
                .tooltip(tip(format!("{}: {w}", t("capture.mode"))))
                .child(div().flex_none().child(icon(IconName::Warning).small()))
                .child(div().min_w_0().truncate().child(w.clone()))
        });
        let info = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(theme::S2)
            .child(name)
            .child(state_line)
            .children(codec)
            // Close under the codec's details, as their two lines are (issue #86)
            .children(mic.map(|m| div().mt(theme::S1 - theme::S2).child(m)))
            .children(waiting);

        let link = self.link.as_ref().map(|(quality, spark)| {
            div()
                .flex()
                .items_center()
                .gap(theme::S3)
                .min_w_0()
                .px(theme::S1)
                .child(quality.clone())
                .child(div().flex_1().min_w_0().child(spark.clone()))
        });

        let button = connect_button(SharedString::from(format!("connect-{key}")), card.state, card.running, true)
            .h(px(44.))
            .rounded(theme::RADIUS)
            .text_size(theme::FS_LARGE)
            .loading(self.busy)
            .disabled(self.busy)
            .on_click(cx.listener(|this, _, _, cx| this.press(cx)));

        ui::card(cx)
            .h_full()
            .p(theme::S5)
            .min_w_0()
            .child(div().flex().items_center().gap(theme::S4).min_w_0().child(ring).child(info))
            .child(self.volume.clone())
            .children(link)
            .child(div().mt_auto().child(button))
            .into_any_element()
    }
}

// ---------------------------------------------------------------------------
// The codec chosen on the card (Home, the flyout)
// ---------------------------------------------------------------------------

// Devices whose codec is being saved: their list waits
static CODEC_SAVING: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

fn codec_saving(key: &str, on: Option<bool>) -> bool {
    let mut set = CODEC_SAVING.lock().unwrap_or_else(|e| e.into_inner());
    match on {
        Some(true) => set.insert(key.to_owned()),
        Some(false) => set.remove(key),
        None => set.contains(key),
    }
}

/// The codec of the device in use, chosen on its card (Home, the flyout;
/// issue #86): one of the codecs it supports, as on Codecs, by name only.
/// It shows the codec streaming (the warning beside it says why another
/// one than chosen), else Auto's pick or the one chosen; choosing a codec turns
/// Auto-select off, which is turned back on on Codecs. A choice applies at
/// once: the device in use connects again with it (settings.set,
/// profile.save in the core). An LE Audio device has LC3 only.
pub fn codec_picker(entry: &AudioEntry, cx: &App) -> AnyElement {
    let p = *cx.palette();
    if entry.le_audio {
        // LC3 is its only codec: a value, not a list that cannot open (issue #86)
        return div()
            .w_full()
            .h(px(32.))
            .flex()
            .items_center()
            .px(theme::S3)
            .border_1()
            .border_color(p.border)
            .rounded(theme::RADIUS)
            .font_weight(FontWeight::SEMIBOLD)
            .child(t("ui.codec.lc3"))
            .into_any_element();
    }
    let Some(snap) = app::core(cx).read(cx).snap() else { return div().into_any_element() };
    let picker = model::codec_picker(snap, entry);
    let mut choices: Vec<ui::Choice<Option<String>>> =
        picker.choices.iter().map(|id| ui::Choice::new(Some(id.clone()), t(codec_spec(id).label))).collect();
    if picker.value.is_none() {
        choices.insert(0, ui::Choice::new(None, t("ui.codec.none")).disabled(true));
    }
    let busy = codec_saving(&entry.key, None) || settling(cx);
    let entry = entry.clone();
    div()
        .flex()
        .flex_col()
        .w_full()
        .min_w_0()
        .child(ui::select_labelled(
            SharedString::from(format!("codec-{}", entry.key)),
            t("quality.codec"),
            &picker.value,
            choices,
            busy,
            move |id: &Option<String>, _, cx| {
                if let Some(id) = id {
                    choose(&entry, id, cx);
                }
            },
        ))
        .into_any_element()
}

/// A codec chosen on the card
fn choose(entry: &AudioEntry, id: &str, cx: &mut App) {
    let Some(snap) = app::core(cx).read(cx).snap().cloned() else { return };
    // The one shown, or chosen already while another one streams instead:
    // the list shows what the device has again by itself
    let CodecPick::Save { auto_off } = codec_pick(&snap, entry, id) else { return };
    // The stream starts again with the codec: the next choice waits for it,
    // as on Codecs (issue #86)
    let running = model::codec_restarts(&snap, entry);
    codec_saving(&entry.key, Some(true));
    cx.refresh_windows();
    let (entry, id) = (entry.clone(), id.to_owned());
    cx.spawn(async move |cx: &mut AsyncApp| {
        // Auto-select is one setting for every device (as on Codecs): turned
        // off before the codec is chosen, so the device connects again with it
        let off = auto_off.then(|| cx.update(|cx| app::apply_settings(|s| s.auto_codec = false, cx)));
        let ok = match off {
            Some(task) => task.await.is_some(),
            None => true,
        };
        let ok = ok && cx.update(|cx| choose_codec(&entry, &id, None, None, false, cx)).await.is_some();
        codec_saving(&entry.key, Some(false));
        cx.update(|cx| {
            if ok && running {
                settle(cx);
            }
            cx.refresh_windows();
        });
    })
    .detach();
}

//! Developer (shown while developer mode is on): the chip tools (ChipTools),
//! the peer receiver test, an Auracast broadcast (Auracast), the earbuds'
//! matrix mixer (Tws),
//! overrides of the codec and the media path for finding out why a sink stays
//! silent (issue #87; read when the next stream is set up), each saved
//! device's max media packet size (issue #86), when vendors' own protocols
//! are used (issue #237: kept by vendor id, the vendors as the core lists
//! them, so a new vendor needs only its option labels here), and how the core
//! logs: retention, the daily size cap, verbose logging that stays on, the
//! stream health line every interval (issue #84) and marker lines.

use std::collections::HashMap;

use gpui_kit::component::Disableable as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, AsyncApp, Context, Div, Entity, Focusable as _, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Subscription, WeakEntity, Window, div, px,
};
use ipc::{KeysComparison, LogLevel, Method, Profile, Settings as CoreSettings, Snapshot};
use ui_model::codecs::{PACKET_DEFAULT, PACKET_MAX, PACKET_MIN};
use ui_model::i18n::t;
use ui_model::profile::{audio_entries, normalize_profile, profiles_of};
use ui_model::t;
use ui_model::view::developer::{
    MIC_PRESETS, VENDOR_MODES, all_default, commit_bitrate, commit_day_cap, commit_iso_queue, commit_keep_days,
    commit_pad, commit_queue_wait, default_log, hex_text, key_count_args, le_number, no_debug, no_overrides,
    packet_commit, packet_valid, reset_all, value_names, vendor_mode_key, vendor_of, vendor_with_mode,
    vendor_with_option, vendors_active, zero_empty,
};

use crate::app;
use crate::core::Core;
use crate::host;
use crate::screens::auracast::Auracast;
use crate::screens::chip_tools::ChipTools;
use crate::screens::hid_host::HidSection;
use crate::screens::receiver_test::ReceiverTest;
use crate::screens::tws::Tws;
use crate::theme;
use crate::ui::{self, BannerKind, ButtonKind, Choice, IconName, icon};

// The app's own settings apart from the codec and media path (issue #86)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    App,
    Receiver,
    Auracast,
    Tws,
    Overrides,
    Vendors,
    Logging,
    Chip,
    Debug,
}

/// A text field bound to a setting: shows the saved value, saves what is
/// typed when it is left or Enter is pressed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Num {
    QueueWait,
    AacBitrate,
    MtkPad,
    MtkCommand,
    MtkRegister,
    MtkQueue,
    LeLatencyCToP,
    LeLatencyPToC,
    LeRtnCToP,
    LeRtnPToC,
    LeBufferMargin,
    KeepDays,
    DayCap,
}

impl Num {
    const ALL: [Num; 13] = [
        Num::QueueWait,
        Num::AacBitrate,
        Num::MtkPad,
        Num::MtkCommand,
        Num::MtkRegister,
        Num::MtkQueue,
        Num::LeLatencyCToP,
        Num::LeLatencyPToC,
        Num::LeRtnCToP,
        Num::LeRtnPToC,
        Num::LeBufferMargin,
        Num::KeepDays,
        Num::DayCap,
    ];

    /// The text the field shows for the saved settings
    fn shown(self, s: &CoreSettings) -> String {
        let (o, d, l) = (&s.dev_overrides, &s.dev_debug, &s.dev_log);
        match self {
            Num::QueueWait => zero_empty(o.queue_max_wait_ms.into()),
            Num::AacBitrate => zero_empty(o.aac_bitrate_kbps.into()),
            Num::MtkPad => zero_empty(d.mtk_iso_pad.into()),
            Num::MtkCommand => d.mtk_command.clone(),
            Num::MtkRegister => d.mtk_register.clone(),
            Num::MtkQueue => zero_empty(d.mtk_iso_queue.into()),
            Num::LeLatencyCToP => zero_empty(d.le_latency_c_to_p_ms.into()),
            Num::LeLatencyPToC => zero_empty(d.le_latency_p_to_c_ms.into()),
            Num::LeRtnCToP => d.le_rtn_c_to_p.map_or_else(String::new, |n| n.to_string()),
            Num::LeRtnPToC => d.le_rtn_p_to_c.map_or_else(String::new, |n| n.to_string()),
            Num::LeBufferMargin => d.le_buffer_margin.map_or_else(String::new, |n| n.to_string()),
            Num::KeepDays => l.keep_days.to_string(),
            Num::DayCap => l.day_cap_mb.to_string(),
        }
    }

    /// What is typed, into the settings
    fn commit(self, text: &str, s: &mut CoreSettings) {
        let (o, d, l) = (&mut s.dev_overrides, &mut s.dev_debug, &mut s.dev_log);
        let le = |max| le_number(text, max);
        match self {
            // The bit rate commits when the field is left, so typing does not restart anything
            Num::AacBitrate => o.aac_bitrate_kbps = commit_bitrate(text),
            // The send queue limit too; empty = the default
            Num::QueueWait => o.queue_max_wait_ms = commit_queue_wait(text),
            Num::MtkPad => d.mtk_iso_pad = commit_pad(text),
            Num::MtkCommand => d.mtk_command = hex_text(text),
            Num::MtkRegister => d.mtk_register = hex_text(text),
            Num::MtkQueue => d.mtk_iso_queue = commit_iso_queue(text),
            Num::LeLatencyCToP => d.le_latency_c_to_p_ms = le(4000).unwrap_or(0) as u16,
            Num::LeLatencyPToC => d.le_latency_p_to_c_ms = le(4000).unwrap_or(0) as u16,
            Num::LeRtnCToP => d.le_rtn_c_to_p = le(15).map(|n| n as u8),
            Num::LeRtnPToC => d.le_rtn_p_to_c = le(15).map(|n| n as u8),
            // Empty: the core's own margin (3 frames)
            Num::LeBufferMargin => d.le_buffer_margin = le(7).map(|n| n as u8),
            Num::KeepDays => l.keep_days = commit_keep_days(text, l.keep_days),
            Num::DayCap => l.day_cap_mb = commit_day_cap(text, l.day_cap_mb),
        }
    }

    fn placeholder(self) -> Option<String> {
        match self {
            Num::QueueWait => Some(t("ui.dev.queue_wait_auto")),
            Num::AacBitrate => Some(t("ui.dev.aac_bitrate_auto")),
            Num::MtkPad => Some("342".into()),
            Num::MtkQueue => Some("12".into()),
            Num::LeLatencyCToP | Num::LeLatencyPToC | Num::LeRtnCToP | Num::LeRtnPToC => Some(t("codec.auto")),
            Num::LeBufferMargin => Some("3".into()),
            Num::MtkCommand | Num::MtkRegister | Num::KeepDays | Num::DayCap => None,
        }
    }

    /// Text in hex, the others whole numbers
    fn text(self) -> bool {
        matches!(self, Num::MtkCommand | Num::MtkRegister)
    }
}

/// One device's max media packet size field
struct PacketField {
    input: Entity<InputState>,
    /// What is typed; None: the saved value
    text: Option<String>,
    value: u16,
    _sub: Subscription,
}

pub struct Developer {
    core: Entity<Core>,
    tab: Tab,
    fields: Vec<(Num, Entity<InputState>)>,
    packets: HashMap<String, PacketField>,
    note: Entity<InputState>,
    marked: Option<u64>,
    hid: Entity<HidSection>,
    receiver: Option<Entity<ReceiverTest>>,
    auracast: Option<Entity<Auracast>>,
    tws: Option<Entity<Tws>>,
    chip: Option<Entity<ChipTools>>,
    keys_chosen: Option<String>,
    keys_busy: bool,
    keys_result: Option<KeysComparison>,
    _subs: Vec<Subscription>,
}

impl Developer {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Developer {
        let core = app::core(cx);
        let mut subs = vec![cx.observe_in(&core, window, |this, _, window, cx| {
            this.sync_fields(window, cx);
            this.sync_packets(window, cx);
            cx.notify();
        })];
        let snap = core.read(cx).snap().cloned();
        let fields = Num::ALL
            .into_iter()
            .map(|n| {
                let shown = snap.as_ref().map(|s| n.shown(&s.settings)).unwrap_or_default();
                let input = cx.new(|cx| {
                    let mut i = InputState::new(window, cx).default_value(shown);
                    if let Some(p) = n.placeholder() {
                        i = i.placeholder(p);
                    }
                    if !n.text() {
                        i = i.validate(|s, _| s.chars().all(|c| c.is_ascii_digit()));
                    }
                    i
                });
                subs.push(cx.subscribe_in(&input, window, move |this, _, ev: &InputEvent, window, cx| {
                    if matches!(ev, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                        this.commit(n, window, cx);
                    }
                }));
                (n, input)
            })
            .collect();
        let note = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t("ui.dev.mark_placeholder"))
                .validate(|s, _| s.chars().count() <= 200)
        });
        subs.push(cx.subscribe_in(&note, window, |this, _, ev: &InputEvent, window, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.mark(window, cx);
            }
        }));
        let hid = cx.new(|cx| HidSection::new(window, cx));
        let mut this = Developer {
            core,
            tab: Tab::App,
            fields,
            packets: HashMap::new(),
            note,
            marked: None,
            hid,
            receiver: None,
            auracast: None,
            tws: None,
            chip: None,
            keys_chosen: None,
            keys_busy: false,
            keys_result: None,
            _subs: subs,
        };
        this.sync_packets(window, cx);
        this
    }

    fn settings(&self, cx: &App) -> Option<CoreSettings> {
        self.core.read(cx).snap().map(|s| s.settings.clone())
    }

    fn input(&self, n: Num) -> &Entity<InputState> {
        &self.fields.iter().find(|(f, _)| *f == n).expect("every field has an input").1
    }

    /// The fields not being edited show what is saved
    fn sync_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = self.settings(cx) else { return };
        for (n, input) in &self.fields {
            let shown = n.shown(&s);
            let focused = input.read(cx).focus_handle(cx).is_focused(window);
            if !focused && input.read(cx).value().as_ref() != shown {
                input.update(cx, |i, cx| i.set_value(shown, window, cx));
            }
        }
    }

    fn commit(&mut self, n: Num, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut s) = self.settings(cx) else { return };
        let input = self.input(n).clone();
        let text = input.read(cx).value().to_string();
        if text == n.shown(&s) {
            return;
        }
        n.commit(&text, &mut s);
        // The field shows what is saved (clamped) at once
        let shown = n.shown(&s);
        input.update(cx, |i, cx| i.set_value(shown, window, cx));
        app::save_settings(move |settings| n.commit(&text, settings), cx);
    }

    fn select_tab(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        self.tab = tab;
        // A tab's part lives while it is shown
        self.receiver = (tab == Tab::Receiver).then(|| cx.new(|cx| ReceiverTest::new(window, cx)));
        self.auracast = (tab == Tab::Auracast).then(|| cx.new(|cx| Auracast::new(window, cx)));
        self.tws = (tab == Tab::Tws).then(|| cx.new(|cx| Tws::new(window, cx)));
        self.chip = (tab == Tab::Chip).then(|| cx.new(|cx| ChipTools::new(window, cx)));
        cx.notify();
    }

    // ---- App settings ----

    fn app_tab(&self, s: &CoreSettings, cx: &mut Context<Self>) -> Div {
        let reset = ui::button("dev-reset-all", t("ui.dev.reset_all"), ButtonKind::Default)
            .icon(icon(IconName::ArrowReset))
            .disabled(all_default(s))
            .on_click(|_, _, cx| app::save_settings(reset_all, cx));
        tab_panel()
            .child(ui::section(t("ui.nav.mixer"), None, cx).child(ui::toggle(
                "dev-volume-mixer",
                t("ui.dev.volume_mixer"),
                Some(t("ui.dev.volume_mixer_hint").into()),
                s.volume_mixer,
                false,
                |v, _, cx| {
                    let v = *v;
                    app::save_settings(move |s| s.volume_mixer = v, cx);
                },
                cx,
            )))
            .child(
                ui::section(t("ui.nav.devices"), None, cx)
                    .child(ui::toggle(
                        "dev-all-audio-devices",
                        t("ui.dev.all_audio_devices"),
                        None,
                        s.all_audio_devices,
                        false,
                        |v, _, cx| {
                            let v = *v;
                            app::save_settings(move |s| s.all_audio_devices = v, cx);
                        },
                        cx,
                    ))
                    .child(ui::toggle(
                        "dev-hide-cable-warning",
                        t("ui.dev.hide_cable_warning"),
                        None,
                        s.vb_cable_note_hidden || s.hifi_cable_note_hidden,
                        false,
                        |v, _, cx| {
                            let v = *v;
                            app::save_settings(
                                move |s| {
                                    s.vb_cable_note_hidden = v;
                                    s.hifi_cable_note_hidden = v;
                                },
                                cx,
                            );
                        },
                        cx,
                    )),
            )
            .child(ui::section(t("update.check"), None, cx).child(ui::toggle(
                "dev-include-alphas",
                t("ui.dev.include_alphas"),
                None,
                s.include_alphas,
                false,
                |v, _, cx| {
                    let v = *v;
                    app::save_settings(move |s| s.include_alphas = v, cx);
                },
                cx,
            )))
            .child(self.hid.clone())
            .child(div().flex().child(reset))
    }

    // ---- Codec settings ----

    fn overrides_tab(&self, snap: &Snapshot, window: &Window, cx: &mut Context<Self>) -> Div {
        let o = snap.settings.dev_overrides;
        let active = o != no_overrides();
        let toggle = |id: &'static str,
                      key: &str,
                      hint: Option<&str>,
                      value: bool,
                      set: fn(&mut CoreSettings, bool),
                      cx: &App| {
            ui::toggle(
                id,
                t(key),
                hint.map(|h| t(h).into()),
                value,
                false,
                move |v, _, cx| {
                    let v = *v;
                    app::save_settings(move |s| set(s, v), cx);
                },
                cx,
            )
        };
        tab_panel()
            .child(intro_banner(active, "ui.dev.intro", cx))
            .child(
                ui::section(t("ui.nav.codecs"), None, cx)
                    .child(toggle(
                        "dev-auto-codec-settings",
                        "ui.dev.auto_codec_settings",
                        Some("ui.dev.auto_codec_settings_hint"),
                        o.auto_codec_settings,
                        |s, v| s.dev_overrides.auto_codec_settings = v,
                        cx,
                    ))
                    .child(toggle(
                        "dev-match-source",
                        "ui.dev.match_source",
                        Some("ui.dev.match_source_hint"),
                        o.match_source,
                        |s, v| s.dev_overrides.match_source = v,
                        cx,
                    ))
                    .child(toggle(
                        "dev-ignore-sink-max",
                        "ui.dev.ignore_sink_max",
                        None,
                        o.ignore_sink_max,
                        |s, v| s.dev_overrides.ignore_sink_max = v,
                        cx,
                    )),
            )
            .child(
                ui::section(t("ui.dev.section_media"), None, cx)
                    .child(toggle(
                        "dev-rtp-marker",
                        "ui.dev.rtp_marker",
                        None,
                        o.rtp_marker,
                        |s, v| s.dev_overrides.rtp_marker = v,
                        cx,
                    ))
                    .child(toggle(
                        "dev-packet-dump",
                        "ui.dev.packet_dump",
                        Some("ui.dev.packet_dump_hint"),
                        o.packet_dump,
                        |s, v| s.dev_overrides.packet_dump = v,
                        cx,
                    ))
                    .child(self.num_field(
                        Num::QueueWait,
                        "ui.dev.queue_wait",
                        Some("ui.dev.queue_wait_hint"),
                        false,
                        cx,
                    ))
                    .child(toggle(
                        "dev-encode-at-send",
                        "ui.dev.encode_at_send",
                        Some("ui.dev.encode_at_send_hint"),
                        o.encode_at_send,
                        |s, v| s.dev_overrides.encode_at_send = v,
                        cx,
                    ))
                    .child(toggle(
                        "dev-keep-bitrate",
                        "ui.dev.keep_bitrate_on_slow_link",
                        None,
                        o.keep_bitrate_on_slow_link,
                        |s, v| s.dev_overrides.keep_bitrate_on_slow_link = v,
                        cx,
                    )),
            )
            .when_some(self.packet_sizes(snap, window, cx), |d, p| d.child(p))
            .child(ui::section(t("ui.diag.mic_title"), None, cx).child(toggle(
                "dev-auto-mic-tick-poll",
                "ui.dev.auto_mic_tick_poll",
                None,
                o.auto_mic_tick_poll,
                |s, v| s.dev_overrides.auto_mic_tick_poll = v,
                cx,
            )))
            .child(
                ui::section("AAC", None, cx)
                    .child(toggle(
                        "dev-aac-latm-v0",
                        "ui.dev.aac_latm_v0",
                        Some("ui.dev.aac_latm_v0_hint"),
                        o.aac_latm_v0,
                        |s, v| s.dev_overrides.aac_latm_v0 = v,
                        cx,
                    ))
                    .child(toggle(
                        "dev-aac-config-bitrate0",
                        "ui.dev.aac_config_bitrate0",
                        Some("ui.dev.aac_config_bitrate0_hint"),
                        o.aac_config_bitrate0,
                        |s, v| s.dev_overrides.aac_config_bitrate0 = v,
                        cx,
                    ))
                    .child(self.num_field(
                        Num::AacBitrate,
                        "ui.dev.aac_bitrate",
                        Some("ui.dev.aac_bitrate_hint"),
                        false,
                        cx,
                    ))
                    .child(toggle(
                        "dev-aac-no-peak",
                        "ui.dev.aac_no_peak",
                        None,
                        o.aac_no_peak,
                        |s, v| s.dev_overrides.aac_no_peak = v,
                        cx,
                    ))
                    .child(toggle(
                        "dev-aac-no-afterburner",
                        "ui.dev.aac_no_afterburner",
                        None,
                        o.aac_no_afterburner,
                        |s, v| s.dev_overrides.aac_no_afterburner = v,
                        cx,
                    )),
            )
    }

    /// A field bound to a setting, its label and hint on the left
    fn num_field(&self, n: Num, label: &str, hint: Option<&str>, disabled: bool, cx: &App) -> Div {
        let mut input = Input::new(self.input(n)).aria_label(t(label)).disabled(disabled);
        if n.text() {
            input = input.font_family(theme::FONT_MONO);
        }
        let width = if n.text() { px(240.) } else { px(140.) };
        ui::field(t(label), hint.map(|h| t(h).into()), None, div().w(width).child(input), cx)
    }

    /// The saved devices' packet fields, keyed by the entry, so a field keeps
    /// what is typed across updates
    fn sync_packets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(snap) = self.core.read(cx).snap().cloned() else { return };
        let entries: Vec<_> = audio_entries(&snap).into_iter().filter(|e| !e.le_audio && e.profile.is_some()).collect();
        self.packets.retain(|k, _| entries.iter().any(|e| e.key == *k));
        for e in entries {
            let value = e.profile.as_ref().map_or(PACKET_DEFAULT, |p| p.max_media_payload);
            if let Some(f) = self.packets.get_mut(&e.key) {
                // The saved value changed: what was typed goes, once the field
                // is left (a field being edited is never overwritten)
                if f.value != value && !f.input.read(cx).focus_handle(cx).is_focused(window) {
                    f.value = value;
                    f.text = None;
                    f.input.update(cx, |i, cx| i.set_value(value.to_string(), window, cx));
                }
                continue;
            }
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(value.to_string())
                    .validate(|s, _| s.chars().all(|c| c.is_ascii_digit()))
            });
            let key = e.key.clone();
            let address = e.address.clone();
            let sub = cx.subscribe_in(&input, window, move |this, input, ev: &InputEvent, _, cx| match ev {
                InputEvent::Change => {
                    let v = input.read(cx).value().to_string();
                    if let Some(f) = this.packets.get_mut(&key) {
                        f.text = Some(v);
                    }
                    cx.notify();
                }
                InputEvent::Blur | InputEvent::PressEnter { .. } => {
                    // Saved when a valid value is left
                    let Some(f) = this.packets.get(&key) else { return };
                    if let Some(v) = packet_commit(f.text.as_deref(), f.value) {
                        this.save_packet(address.clone(), v, cx);
                    }
                }
                _ => {}
            });
            self.packets.insert(e.key.clone(), PacketField { input, text: None, value, _sub: sub });
        }
    }

    /// Kept in the device's profiles, as the core reads it from the one it
    /// streams; LE Audio sends no A2DP packets
    fn save_packet(&mut self, address: String, v: u16, cx: &mut Context<Self>) {
        let profiles: Vec<Profile> = self
            .core
            .read(cx)
            .snap()
            .map(|s| profiles_of(&s.profiles, &address).into_iter().cloned().collect())
            .unwrap_or_default();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            for p in profiles {
                if p.le_audio || p.max_media_payload == v {
                    continue;
                }
                let profile = normalize_profile(&Profile { max_media_payload: v, ..p.clone() });
                let Ok(task) = this.update(cx, |_, cx| {
                    app::run(
                        app::request::<serde_json::Value>(
                            Method::ProfileSave { profile: Box::new(profile), original_name: Some(p.name.clone()) },
                            cx,
                        ),
                        cx,
                    )
                }) else {
                    return;
                };
                if task.await.is_none() {
                    break;
                }
            }
        })
        .detach();
    }

    /// The max media packet size of each saved device (issue #86: not one
    /// for end users, so here and not in the device's settings)
    fn packet_sizes(&self, snap: &Snapshot, _window: &Window, cx: &mut Context<Self>) -> Option<Div> {
        let entries: Vec<_> = audio_entries(snap).into_iter().filter(|e| !e.le_audio && e.profile.is_some()).collect();
        if entries.is_empty() {
            return None;
        }
        let mut section = ui::section(t("advanced.max_packet"), None, cx)
            .child(ui::hint(t!("advanced.max_packet_help", PACKET_MIN, PACKET_MAX, PACKET_DEFAULT), cx));
        for e in entries {
            let Some(f) = self.packets.get(&e.key) else { continue };
            let invalid = f.text.as_deref().is_some_and(|s| !packet_valid(s));
            let error = invalid.then(|| SharedString::from(t!("ui.profile.err_packet", PACKET_MIN, PACKET_MAX)));
            let address = e.address.clone();
            let recommended = ui::button(
                SharedString::from(format!("dev-packet-default-{}", e.key)),
                t!("advanced.max_packet_recommended", PACKET_DEFAULT),
                ButtonKind::Subtle,
            )
            .disabled(f.value == PACKET_DEFAULT && f.text.is_none())
            .on_click(cx.listener(move |this, _, _, cx| this.save_packet(address.clone(), PACKET_DEFAULT, cx)));
            let label = if e.name.is_empty() { e.address.clone() } else { e.name.clone() };
            section = section.child(ui::field(
                label.clone(),
                None,
                error,
                ui::inline().child(div().w(px(100.)).child(Input::new(&f.input).aria_label(label))).child(recommended),
                cx,
            ));
        }
        Some(section)
    }

    // ---- Vendors ----

    fn vendors_tab(&self, snap: &Snapshot, cx: &mut Context<Self>) -> Div {
        let value = snap.settings.dev_vendors.clone();
        let active = vendors_active(&value);
        let mut panel = tab_panel().child(intro_banner(active, "ui.dev.vendors_intro", cx));
        for v in &snap.vendors {
            let current = vendor_of(&value, &v.id);
            let modes = VENDOR_MODES.iter().map(|m| Choice::new(*m, t(vendor_mode_key(*m)))).collect();
            let (id, all) = (v.id.clone(), value.clone());
            let mut section = ui::section(v.name.clone(), None, cx).child(ui::field(
                t("ui.dev.vendor_mode"),
                None,
                None,
                ui::select(
                    SharedString::from(format!("dev-vendor-{}", v.id)),
                    &current.mode,
                    modes,
                    false,
                    move |m, _, cx| {
                        let next = vendor_with_mode(&all, &id, *m);
                        app::save_settings(move |s| s.dev_vendors = next, cx);
                    },
                ),
                cx,
            ));
            for o in &v.options {
                let (id, option, all) = (v.id.clone(), o.clone(), value.clone());
                section = section.child(ui::toggle(
                    SharedString::from(format!("dev-vendor-{}-{o}", v.id)),
                    t(&format!("ui.dev.vendor_option.{}.{o}", v.id)),
                    None,
                    current.options.contains(o),
                    false,
                    move |on, _, cx| {
                        let next = vendor_with_option(&all, &id, &option, *on);
                        app::save_settings(move |s| s.dev_vendors = next, cx);
                    },
                    cx,
                ));
            }
            panel = panel.child(section);
        }
        panel.child(
            div().flex().child(
                ui::button("dev-vendors-reset", t("ui.dev.reset"), ButtonKind::Default)
                    .icon(icon(IconName::ArrowReset))
                    .disabled(!active)
                    .on_click(|_, _, cx| app::save_settings(|s| s.dev_vendors = Default::default(), cx)),
            ),
        )
    }

    // ---- Logging ----

    fn mark(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let note = self.note.read(cx).value().to_string();
        let task = app::run(app::request::<u64>(Method::DiagMark { note }, cx), cx);
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx| {
            let Some(n) = task.await else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                this.marked = Some(n);
                this.note.update(cx, |i, cx| i.set_value("", window, cx));
                cx.notify();
            });
        })
        .detach();
    }

    fn logging_tab(&self, snap: &Snapshot, cx: &mut Context<Self>) -> Div {
        let l = snap.settings.dev_log;
        let changed = l != default_log();
        let log_dir = snap.log_dir.clone();
        let actions = ui::inline()
            // None while the core runs as a service: its log folder is protected
            .when(!log_dir.is_empty(), |d| {
                d.child(
                    ui::button("dev-open-logs", t("ui.diag.open_logs"), ButtonKind::Subtle)
                        .icon(icon(IconName::FolderOpen))
                        .tooltip(t("ui.diag.logs_unmasked"))
                        .on_click(move |_, _, cx| host::open_folder(&log_dir, cx)),
                )
            })
            .child(
                ui::button("dev-log-reset", t("ui.dev.log_reset"), ButtonKind::Default)
                    .icon(icon(IconName::ArrowReset))
                    .disabled(!changed)
                    .on_click(|_, _, cx| app::save_settings(|s| s.dev_log = default_log(), cx)),
            );
        let levels = vec![
            Choice::new(None, t("ui.diag.verbose_off")),
            Choice::new(Some(LogLevel::Debug), t("ui.dev.log_always_debug")),
            Choice::new(Some(LogLevel::Trace), t("ui.dev.log_always_trace")),
        ];
        tab_panel()
            .child(ui::banner(BannerKind::Info, Some(t("ui.dev.log_intro").into()), None, None, None, cx))
            .child(
                ui::section(t("ui.dev.log_files"), Some(actions.into_any_element()), cx)
                    .child(self.num_field(
                        Num::KeepDays,
                        "ui.dev.log_keep_days",
                        Some("ui.dev.log_keep_days_hint"),
                        false,
                        cx,
                    ))
                    .child(self.num_field(
                        Num::DayCap,
                        "ui.dev.log_day_cap",
                        Some("ui.dev.log_day_cap_hint"),
                        false,
                        cx,
                    )),
            )
            .child(
                ui::section(t("ui.dev.log_detail"), None, cx)
                    .child(ui::field(
                        t("ui.dev.log_always"),
                        Some(t("ui.dev.log_always_hint").into()),
                        None,
                        ui::select("dev-log-always", &l.always_verbose, levels, false, |v, _, cx| {
                            let v = *v;
                            app::save_settings(move |s| s.dev_log.always_verbose = v, cx);
                        }),
                        cx,
                    ))
                    .when(l.always_verbose == Some(LogLevel::Trace), |d| {
                        d.child(ui::banner(
                            BannerKind::Warn,
                            Some(t("ui.dev.log_trace_warn").into()),
                            None,
                            None,
                            None,
                            cx,
                        ))
                    })
                    .child(ui::toggle(
                        "dev-log-health",
                        t("ui.dev.log_health"),
                        Some(t("ui.dev.log_health_hint").into()),
                        l.health_every_interval,
                        false,
                        |v, _, cx| {
                            let v = *v;
                            app::save_settings(move |s| s.dev_log.health_every_interval = v, cx);
                        },
                        cx,
                    )),
            )
            .child(
                ui::section(t("ui.dev.mark"), None, cx)
                    .child(ui::field_stacked(
                        t("ui.dev.mark_note"),
                        Some(t("ui.dev.mark_hint").into()),
                        None,
                        Input::new(&self.note).aria_label(t("ui.dev.mark_note")),
                        cx,
                    ))
                    .child(
                        ui::inline()
                            .child(
                                ui::button("dev-mark-add", t("ui.dev.mark_add"), ButtonKind::Default)
                                    .icon(icon(IconName::Add))
                                    .on_click(cx.listener(|this, _, window, cx| this.mark(window, cx))),
                            )
                            .when_some(self.marked, |d, n| d.child(ui::muted(t!("ui.dev.mark_done", n), cx))),
                    ),
            )
    }

    // ---- Debug ----

    fn debug_tab(&self, snap: &Snapshot, cx: &mut Context<Self>) -> Div {
        let d = snap.settings.dev_debug.clone();
        let active = d != no_debug();
        let toggle =
            |id: &'static str, key: &str, value: bool, disabled: bool, set: fn(&mut CoreSettings, bool), cx: &App| {
                ui::toggle(
                    id,
                    t(key),
                    None,
                    value,
                    disabled,
                    move |v, _, cx| {
                        let v = *v;
                        app::save_settings(move |s| set(s, v), cx);
                    },
                    cx,
                )
            };
        let presets = std::iter::once(Choice::new(String::new(), t("codec.auto")))
            .chain(MIC_PRESETS.iter().map(|p| Choice::new((*p).to_owned(), *p)))
            .collect();
        tab_panel()
            .child(intro_banner(active, "ui.dev.debug_intro", cx))
            .child(
                ui::section(t("ui.dev.mtk_section"), None, cx)
                    .child(toggle(
                        "dev-mtk-iso-bulk",
                        "ui.dev.mtk_iso_bulk",
                        d.mtk_iso_bulk,
                        false,
                        |s, v| s.dev_debug.mtk_iso_bulk = v,
                        cx,
                    ))
                    .child(self.num_field(Num::MtkPad, "ui.dev.mtk_iso_pad", None, d.mtk_iso_bulk, cx))
                    .child(self.num_field(
                        Num::MtkCommand,
                        "ui.dev.mtk_command",
                        Some("ui.dev.mtk_command_hint"),
                        false,
                        cx,
                    ))
                    .child(toggle(
                        "dev-mtk-command-before-reset",
                        "ui.dev.mtk_command_before_reset",
                        d.mtk_command_before_reset,
                        d.mtk_command.is_empty(),
                        |s, v| s.dev_debug.mtk_command_before_reset = v,
                        cx,
                    ))
                    .child(self.num_field(
                        Num::MtkRegister,
                        "ui.dev.mtk_register",
                        Some("ui.dev.mtk_register_hint"),
                        false,
                        cx,
                    ))
                    .child(self.num_field(Num::MtkQueue, "ui.dev.mtk_iso_queue", None, false, cx)),
            )
            // LE Audio stream experiments: the CIG's latency, RTN and packing,
            // the microphone preset, where the retry without the microphone
            // runs. Empty fields keep what BlueCaster plans.
            .child(
                ui::section(t("ui.dev.le_section"), None, cx)
                    .child(ui::hint(t("ui.dev.le_section_hint"), cx))
                    .child(self.num_field(Num::LeLatencyCToP, "ui.dev.le_latency_c_to_p", None, false, cx))
                    .child(self.num_field(Num::LeLatencyPToC, "ui.dev.le_latency_p_to_c", None, false, cx))
                    .child(self.num_field(Num::LeRtnCToP, "ui.dev.le_rtn_c_to_p", None, false, cx))
                    .child(self.num_field(Num::LeRtnPToC, "ui.dev.le_rtn_p_to_c", None, false, cx))
                    .child(toggle(
                        "dev-le-interleaved",
                        "ui.dev.le_interleaved",
                        d.le_interleaved,
                        false,
                        |s, v| s.dev_debug.le_interleaved = v,
                        cx,
                    ))
                    .child(self.num_field(
                        Num::LeBufferMargin,
                        "ui.dev.le_buffer_margin",
                        Some("ui.dev.le_buffer_margin_hint"),
                        false,
                        cx,
                    ))
                    // On unless turned off: saved as le_realign_off
                    .child(toggle(
                        "dev-le-realign",
                        "ui.dev.le_realign",
                        !d.le_realign_off,
                        false,
                        |s, v| s.dev_debug.le_realign_off = !v,
                        cx,
                    ))
                    .child(ui::field(
                        t("ui.dev.le_mic_preset"),
                        None,
                        None,
                        ui::select("dev-le-mic-preset", &d.le_mic_preset, presets, false, |v: &String, _, cx| {
                            let v = v.clone();
                            app::save_settings(move |s| s.dev_debug.le_mic_preset = v, cx);
                        }),
                        cx,
                    ))
                    .child(toggle(
                        "dev-le-mic-retry",
                        "ui.dev.le_mic_retry_same_link",
                        d.le_mic_retry_same_link,
                        false,
                        |s, v| s.dev_debug.le_mic_retry_same_link = v,
                        cx,
                    )),
            )
            .child(
                ui::section(t("ui.dev.keys_section"), None, cx)
                    .child(toggle(
                        "dev-le-keys-to-windows",
                        "ui.dev.le_keys_to_windows",
                        d.le_keys_to_windows,
                        false,
                        |s, v| s.dev_debug.le_keys_to_windows = v,
                        cx,
                    ))
                    .children(self.keys_compare(snap, cx)),
            )
    }

    /// The adapters the comparison can be run for, and the one chosen
    fn keys_adapter(&self, snap: &Snapshot) -> Option<String> {
        let adapters: Vec<&String> = snap.adapter.adapters.iter().filter_map(|a| a.address.as_ref()).collect();
        adapters
            .iter()
            .find(|a| Some(a.as_str()) == self.keys_chosen.as_deref())
            .or(adapters.first())
            .map(|a| (*a).clone())
    }

    fn compare_keys(&mut self, cx: &mut Context<Self>) {
        let Some(adapter) = self.core.read(cx).snap().and_then(|s| self.keys_adapter(s)) else { return };
        self.keys_busy = true;
        cx.notify();
        let task = app::request::<KeysComparison>(Method::DevCompareWindowsKeys { adapter }, cx);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let r = task.await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    Ok(k) => this.keys_result = Some(k),
                    Err(e) => app::notify(app::error_text(&e), app::NoticeKind::Error, cx),
                }
                this.keys_busy = false;
                cx.notify();
            });
        })
        .detach();
    }

    /// BlueCaster's pairing keys against Windows' for one adapter (issue
    /// #86): numbers only, so the result can be shared without addresses
    /// or keys
    fn keys_compare(&self, snap: &Snapshot, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        let adapters: Vec<_> = snap.adapter.adapters.iter().filter(|a| a.address.is_some()).collect();
        let adapter = self.keys_adapter(snap);
        if adapters.len() > 1 {
            let choices = adapters
                .iter()
                .map(|a| Choice::new(a.address.clone().unwrap_or_default(), a.display_name.clone()))
                .collect();
            out.push(
                ui::field(
                    t("ui.dev.keys_adapter"),
                    None,
                    None,
                    ui::select(
                        "dev-keys-adapter",
                        &adapter.clone().unwrap_or_default(),
                        choices,
                        false,
                        cx.listener(|this, v: &String, _, cx| {
                            this.keys_chosen = Some(v.clone());
                            this.keys_result = None;
                            cx.notify();
                        }),
                    ),
                    cx,
                )
                .into_any_element(),
            );
        }
        out.push(
            div()
                .flex()
                .child(
                    ui::button("dev-keys-compare", t("ui.dev.keys_compare"), ButtonKind::Default)
                        .icon(icon(IconName::Shield))
                        .loading(self.keys_busy)
                        .disabled(adapter.is_none())
                        .on_click(cx.listener(|this, _, _, cx| this.compare_keys(cx))),
                )
                .into_any_element(),
        );
        if let Some(r) = &self.keys_result {
            if !r.adapter_found {
                out.push(ui::muted(t("ui.dev.keys_no_windows"), cx).into_any_element());
            } else {
                let counts = |key: &str, c| {
                    let [a, b, c2, d] = key_count_args(c);
                    t!(key, a, b, c2, d)
                };
                out.push(div().child(counts("ui.dev.keys_link", &r.classic)).into_any_element());
                out.push(
                    div()
                        .child(
                            r.le.as_ref()
                                .map_or_else(|| t("ui.dev.keys_le_unreadable"), |le| counts("ui.dev.keys_le", le)),
                        )
                        .into_any_element(),
                );
                let mono_muted = |text: String| ui::muted(text, cx).font_family(theme::FONT_MONO).into_any_element();
                if !r.le_values.is_empty() {
                    out.push(mono_muted(t!("ui.dev.keys_le_values", value_names(&r.le_values))));
                }
                if !r.radio_values.is_empty() {
                    out.push(mono_muted(t!("ui.dev.keys_radio_values", value_names(&r.radio_values))));
                }
            }
        }
        out
    }
}

/// The tab's content, its sections one below the other
fn tab_panel() -> Div {
    div().flex().flex_col().gap(theme::S4)
}

/// A tab's intro, a warning while anything on it is changed
fn intro_banner(active: bool, intro: &str, cx: &App) -> gpui_kit::Stateful<Div> {
    if active {
        ui::banner(BannerKind::Warn, Some(t("ui.dev.active").into()), None, None, None, cx)
    } else {
        ui::banner(BannerKind::Info, Some(t(intro).into()), None, None, None, cx)
    }
}

impl Render for Developer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(snap) = self.core.read(cx).snap().cloned() else { return div().into_any_element() };
        let tabs = vec![
            Choice::new(Tab::App, t("ui.dev.app_settings")).icon(IconName::Settings),
            Choice::new(Tab::Overrides, t("ui.dev.codec_settings")).icon(IconName::Wrench),
            Choice::new(Tab::Vendors, t("ui.dev.vendors")).icon(IconName::Headphones),
            Choice::new(Tab::Logging, t("ui.dev.logging")).icon(IconName::Record),
            Choice::new(Tab::Receiver, t("receiver.title")).icon(IconName::Beaker),
            Choice::new(Tab::Auracast, t("ui.dev.auracast")).icon(IconName::Pulse),
            Choice::new(Tab::Tws, t("ui.dev.tws")).icon(IconName::Headphones),
            Choice::new(Tab::Chip, t("ui.dev.chip")).icon(IconName::UsbStick),
            Choice::new(Tab::Debug, t("ui.dev.debug")).icon(IconName::Layer),
        ];
        let panel: AnyElement = match self.tab {
            Tab::App => self.app_tab(&snap.settings, cx).into_any_element(),
            Tab::Overrides => self.overrides_tab(&snap, window, cx).into_any_element(),
            Tab::Vendors => self.vendors_tab(&snap, cx).into_any_element(),
            Tab::Logging => self.logging_tab(&snap, cx).into_any_element(),
            Tab::Receiver => self.receiver.clone().map_or_else(|| div().into_any_element(), |r| r.into_any_element()),
            Tab::Auracast => self.auracast.clone().map_or_else(|| div().into_any_element(), |a| a.into_any_element()),
            Tab::Tws => self.tws.clone().map_or_else(|| div().into_any_element(), |m| m.into_any_element()),
            Tab::Chip => self.chip.clone().map_or_else(|| div().into_any_element(), |c| c.into_any_element()),
            Tab::Debug => self.debug_tab(&snap, cx).into_any_element(),
        };
        ui::page("developer", t("ui.nav.developer"))
            .child(ui::tab_bar(
                "developer-tabs",
                &self.tab,
                tabs,
                cx.listener(|this, tab: &Tab, window, cx| this.select_tab(*tab, window, cx)),
            ))
            .child(panel)
            .into_any_element()
    }
}

#[cfg(test)]
mod interaction_tests;

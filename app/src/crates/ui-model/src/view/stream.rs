//! The decisions behind the streaming controls shared by Home, the flyout
//! and the navigation's status block: state labels, the codec's line, the
//! headset microphone's chip, link quality, the error panel's fix and the
//! bug report's text; also Home's entry state and headset chip, and the
//! main window's indicators (capture running, crash, notes, device pill).

use ipc::{AdapterInfo, CodecStop, CodecStopKind, CrashReport, DriverKind, LinkSample, Msg, Profile, Snapshot};
use ipc::{SourceInfo, StreamInfo, StreamState, StreamStatus};

use crate::codecdetails::live_bitpool;
use crate::codecs::{bitrate_note_key, codec_label_key};
use crate::format::{khz, mask_identifiers};
use crate::i18n::{t, tm};
use crate::links::{BugReport, bug_report_url};
use crate::profile::{AudioEntry, HeadsetMode, device_key, headset_mode};

/// The profile the big button connects: the one streaming, else the active one
pub fn current_profile(snap: &Snapshot) -> Option<&Profile> {
    let name = if snap.stream.state != StreamState::Idle {
        snap.stream.profile.as_ref().or(snap.active_profile.as_ref())
    } else {
        snap.active_profile.as_ref()
    };
    snap.profiles.iter().find(|p| Some(&p.name) == name).or(snap.profiles.first())
}

/// The state's name as the protocol writes it ("idle", "streaming", ...)
pub fn state_key(s: StreamState) -> &'static str {
    match s {
        StreamState::Idle => "idle",
        StreamState::Connecting => "connecting",
        StreamState::Streaming => "streaming",
        StreamState::Reconnecting => "reconnecting",
        StreamState::Error => "error",
    }
}

/// A state in words; streaming reads "Connected", as the core's own
/// message on Home and in the tray (issue #86)
pub fn state_label(s: StreamState) -> String {
    if s == StreamState::Streaming { t("status.connected") } else { t(&format!("status.{}", state_key(s))) }
}

/// A reconnect or a retry reads as plain connecting, and its end as
/// connected (issue #86): no "1/3" counts; the attempts stay in the log
fn plain_status(key: &str) -> Option<&'static str> {
    Some(match key {
        "status.reconnect_attempt" | "status.connecting_retry" => "status.connecting",
        "status.reconnected" => "status.connected",
        // Why another codec is used goes on the codec's line (issue #86)
        "status.connected_fallback" | "status.connected_fallback_in_use" | "status.connected_fallback_developer" => {
            "status.connected"
        }
        _ => return None,
    })
}

/// What the core says when the headset microphone does not open: the
/// stream plays on, so the status line stays plain and the microphone's
/// chip shows it (issue #86: Home was full of text)
pub const MIC_NOTES: [&str; 3] =
    ["error.headset_mic_loop", "status.headset_mic_failed", "status.headset_mic_output_failed"];

fn codec_stop_kind_key(kind: CodecStopKind) -> &'static str {
    match kind {
        CodecStopKind::Unsupported => "unsupported",
        CodecStopKind::SettingsUnsupported => "settings_unsupported",
        CodecStopKind::InUse => "in_use",
        CodecStopKind::Refused => "refused",
    }
}

/// A codec key or the core's codec name in words ("LDAC", "aptX HD")
fn codec_words(codec: &str) -> String {
    let key = codec_label_key(codec);
    if key.is_empty() { codec.to_owned() } else { t(key) }
}

/// An error the codec caused, in a few words ("LDAC is unsupported", issue
/// #86); None in any other state or for another error
pub fn codec_stop_text(state: StreamState, codec_stop: Option<&CodecStop>) -> Option<String> {
    let c = codec_stop.filter(|_| state == StreamState::Error)?;
    let key = format!("status.codec_stop_{}", codec_stop_kind_key(c.kind));
    Some(crate::t!(&key, codec_words(&c.codec)))
}

/// The device's status line: the core's message, else the state; an
/// error's text is in its panel, with the fix (one the codec caused named
/// in a few words)
pub fn status_text(state: StreamState, message: &Msg, codec_stop: Option<&CodecStop>) -> String {
    if state == StreamState::Error {
        return codec_stop_text(state, codec_stop).unwrap_or_else(|| t("status.error"));
    }
    if MIC_NOTES.contains(&message.key.as_str()) {
        return state_label(state);
    }
    if let Some(plain) = plain_status(&message.key) {
        return t(plain);
    }
    let text = tm(Some(message));
    if text.is_empty() { state_label(state) } else { text }
}

/// [`status_text`] of the stream as it is
pub fn stream_status_text(s: &StreamStatus) -> String {
    status_text(s.state, &s.message, s.codec_stop.as_ref())
}

/// A stream's codec by its translated name ("LDAC", "aptX HD")
pub fn codec_name(info: &StreamInfo) -> String {
    codec_words(&info.codec)
}

/// Whether the stream has a battery to show: a level, or "unknown" when
/// the headphones refused to report it (another device holds their HFP)
pub fn shows_battery(s: &StreamStatus) -> bool {
    s.battery.is_some() || s.battery_refused
}

/// The battery symbol's colour
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryTone {
    Unknown,
    Bad,
    Warn,
    Good,
}

/// The battery level shown (0..=100; unknown: 0)
pub fn battery_level(percent: Option<u8>) -> u8 {
    percent.unwrap_or(0).min(100)
}

pub fn battery_tone(percent: Option<u8>) -> BatteryTone {
    match percent.map(|p| p.min(100)) {
        None => BatteryTone::Unknown,
        Some(p) if p <= 15 => BatteryTone::Bad,
        Some(p) if p <= 40 => BatteryTone::Warn,
        Some(_) => BatteryTone::Good,
    }
}

/// "Battery 80%", "Battery unknown" (the battery's accessible name)
pub fn battery_text(percent: Option<u8>) -> String {
    match percent {
        Some(_) => crate::t!("ui.home.battery", battery_level(percent)),
        None => t("ui.home.battery_unknown"),
    }
}

// The stream's codec is the core's display name ("AAC"), not a profile key
fn is_aac(info: &StreamInfo) -> bool {
    info.codec == "AAC"
}

/// AAC runs a variable bit rate, the others an adaptive one
pub fn abr_label(info: &StreamInfo) -> &'static str {
    if is_aac(info) { "VBR" } else { "ABR" }
}

/// What the ABR / VBR mark explains: AAC's VBR, the others' ABR
pub fn abr_tooltip_key(info: &StreamInfo) -> &'static str {
    if is_aac(info) { "tooltip.vbr" } else { "tooltip.abr" }
}

/// SBC under ABR: "bitpool 41/53" (now / the highest it goes back to)
pub fn bitpool_text(info: &StreamInfo) -> Option<String> {
    live_bitpool(info).map(|bp| crate::t!("ui.codec.bitpool_live", bp.now, bp.max))
}

/// What the codec runs with, in short: "990 kbps", "48 kHz", "24-bit",
/// "ABR"; SBC under ABR "bitpool 53/53" after them, as it moves
pub fn codec_details(info: &StreamInfo) -> Vec<String> {
    let mut out = vec![format!("{} kbps", info.bitrate_kbps), format!("{} kHz", khz(info.sample_rate))];
    if info.bits > 0 {
        out.push(t(&format!("bitdepth.{}", info.bits)));
    }
    if info.abr {
        out.push(abr_label(info).to_owned());
    }
    if info.low_latency {
        out.push("LL".to_owned());
    }
    if let Some(bp) = bitpool_text(info) {
        out.push(bp);
    }
    out
}

/// "400 kbps — ABR on": why the bit rate is below the profile's
pub fn bitrate_note(info: &StreamInfo) -> Option<String> {
    bitrate_note_key(info.abr, info.bitrate_capped).map(|key| format!("{} kbps{}", info.bitrate_kbps, t(key)))
}

/// Beside the codec's name when it is not the one chosen: "(Opus is
/// unsupported)", in use elsewhere, or developer mode only (issue #86); the
/// reason is the core's message, the status line just says "Connected"
pub fn fallback_note_key(info: &StreamInfo, message_key: &str) -> Option<&'static str> {
    info.requested_codec.as_ref()?;
    if message_key == "status.connected_fallback_developer" {
        return Some("ui.codec.fallback_developer");
    }
    Some(if info.requested_in_use { "ui.codec.fallback_in_use" } else { "ui.codec.fallback_unsupported" })
}

/// The note beside a codec used instead of the one chosen ("(Opus is
/// unsupported)"); empty when it is the one chosen
pub fn fallback_note(info: &StreamInfo, message_key: &str) -> String {
    match (fallback_note_key(info, message_key), &info.requested_codec) {
        (Some(key), Some(req)) => crate::t!(key, codec_words(req)),
        _ => String::new(),
    }
}

/// The warning's tooltip beside a codec used instead of the one chosen;
/// empty when it is the one chosen
pub fn fallback_help(info: &StreamInfo, message_key: &str) -> String {
    let Some(req) = &info.requested_codec else { return String::new() };
    // Not the device's doing: developer mode is off (issue #86)
    let key = if message_key == "status.connected_fallback_developer" {
        "tooltip.codec_fallback_developer"
    } else {
        "tooltip.codec_fallback_help"
    };
    crate::t!(key, codec_words(req), codec_name(info))
}

/// How long the note beside a codec used instead of the one chosen stays
/// (issue #86: then only its warning icon, with the reason as its tooltip)
pub const FALLBACK_NOTE_MS: u64 = 5000;

/// The voice codec's translated name (StreamStatus.voice codec key)
pub fn voice_codec_name(codec: &str) -> String {
    match codec {
        "cvsd" | "msbc" | "lc3_swb" => t(&format!("codec.{codec}")),
        "lc3" => "LC3".to_owned(),
        _ => codec.to_owned(),
    }
}

/// The headset microphone of the device in use (`profile`) while its mode
/// is Auto or On with an output chosen, the same as its button (Home's
/// HeadsetChip)
pub fn mic_shown(snap: &Snapshot, profile: Option<&Profile>) -> bool {
    let s = &snap.stream;
    profile.is_some_and(|p| {
        s.state != StreamState::Idle
            && s.profile.as_deref() == Some(p.name.as_str())
            && !p.mic_device_id.is_empty()
            && headset_mode(&p.headset_mic, true) != HeadsetMode::Off
    })
}

/// The microphone's line while its link is not open: its mode (Off while
/// disabled on its button)
pub fn mic_idle_text(profile: &Profile, developer: bool) -> String {
    if profile.mic_muted {
        t("ui.headset_mic_off")
    } else {
        t(&format!("ui.headset_mic_{}", headset_mode(&profile.headset_mic, developer).as_str()))
    }
}

/// The symbol of what a stream captures
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceIcon {
    MusicNote2,
    DesktopSpeaker,
    Speaker2,
    Mic,
    WindowApps,
}

/// What a running stream captures, in words
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceText {
    pub icon: SourceIcon,
    pub text: String,
    pub waiting: bool,
}

/// What a running stream captures (StreamInfo.source), in words: the
/// profile's application name when the executable is the profile's
pub fn source_text(src: &SourceInfo, profile: Option<&Profile>) -> SourceText {
    let or = |name: &str, key: &str| if name.is_empty() { t(key) } else { name.to_owned() };
    let plain = |icon, text| SourceText { icon, text, waiting: false };
    match src.kind.as_str() {
        "tone" => plain(SourceIcon::MusicNote2, t("receiver.source_tone")),
        "virtual" => plain(SourceIcon::DesktopSpeaker, or(&src.name, "capture.virtual")),
        "output" => plain(SourceIcon::Speaker2, or(&src.name, "capture.output")),
        // A recording device (an Auracast broadcast's source)
        "input" => plain(SourceIcon::Mic, or(&src.name, "capture.input")),
        "app" => {
            let known = profile
                .filter(|p| !p.app_exe.is_empty() && p.app_exe.to_lowercase() == src.name.to_lowercase())
                .map(|p| p.app_name.clone())
                .unwrap_or_default();
            let name = if known.is_empty() { or(&src.name, "capture.app") } else { known };
            let text = if src.waiting { crate::t!("ui.source.waiting", &name) } else { name };
            SourceText { icon: SourceIcon::WindowApps, text, waiting: src.waiting }
        }
        _ => plain(SourceIcon::Speaker2, t("capture.loopback")),
    }
}

// ---------------------------------------------------------------------------
// Link quality
// ---------------------------------------------------------------------------

/// Samples (at 4 a second) the quality looks back over for drops: 10 s
const QUALITY_DROP_SAMPLES: usize = 40;
/// ... and for a send queue backing up: 4 s
const QUALITY_QUEUE_SAMPLES: usize = 16;
/// Milliseconds in the send queue that mean the radio fell behind
pub const QUALITY_QUEUE_BACKLOG_MS: u32 = 150;
/// Share of the radio's time above which dropouts become likely
pub const QUALITY_AIRTIME_MAX: f64 = 0.8;

/// What lowered the quality (Diagnostics names them)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityCause {
    Drops,
    Queue,
    Airtime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityStatus {
    Good,
    Warn,
    Bad,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkQuality {
    /// 0..=4 bars
    pub level: u8,
    pub status: QualityStatus,
    pub pc_busy: bool,
    pub causes: Vec<QualityCause>,
}

/// Link quality from the last samples: what reaches the headphones (packet
/// drops, the send queue backing up, air time) sets the bars. Not RSSI:
/// controllers report it differently (vs. the golden range or not) and the
/// headphones' power control moves it by 10-20 dB on a link that delivers
/// everything. Audio the PC itself lost at capture says nothing about the
/// radio (and counts frames, not packets): on a link that is fine, pc_busy.
pub fn link_quality(samples: &[LinkSample]) -> Option<LinkQuality> {
    let last = samples.last()?;
    if last.bytes_per_second == 0.0 && last.rssi.is_none() {
        return None;
    }
    let mut level = 4u8;
    let mut causes = Vec::new();
    let mut cap = |l: u8, cause: QualityCause| {
        level = level.min(l);
        causes.push(cause);
    };
    let old = &samples[samples.len().saturating_sub(QUALITY_DROP_SAMPLES)];
    let dropped = last.dropped.saturating_sub(old.dropped);
    if dropped > 5 {
        cap(1, QualityCause::Drops);
    } else if dropped > 0 {
        cap(2, QualityCause::Drops);
    }
    let backlog = samples[samples.len().saturating_sub(QUALITY_QUEUE_SAMPLES)..]
        .iter()
        .map(|s| s.queue_ms.unwrap_or(0))
        .max()
        .unwrap_or(0);
    if backlog >= QUALITY_QUEUE_BACKLOG_MS {
        cap(2, QualityCause::Queue);
    } else if backlog >= QUALITY_QUEUE_BACKLOG_MS / 2 {
        cap(3, QualityCause::Queue);
    }
    if last.airtime > QUALITY_AIRTIME_MAX {
        cap(2, QualityCause::Airtime);
    }
    let link_good = level >= 3;
    let pc_busy = link_good && last.capture_dropped > old.capture_dropped;
    let status = if pc_busy || (!link_good && level == 2) {
        QualityStatus::Warn
    } else if link_good {
        QualityStatus::Good
    } else {
        QualityStatus::Bad
    };
    Some(LinkQuality { level, status, pc_busy, causes })
}

/// The quality's label (the indicator's)
pub fn quality_label(q: Option<&LinkQuality>) -> String {
    match q {
        None => t("ui.quality.unknown"),
        Some(q) if q.pc_busy => t("ui.quality.pc_busy"),
        Some(q) => t(match q.status {
            QualityStatus::Good => "ui.quality.good",
            QualityStatus::Warn => "ui.quality.fair",
            QualityStatus::Bad => "ui.quality.poor",
        }),
    }
}

// ---------------------------------------------------------------------------
// Connect / Disconnect
// ---------------------------------------------------------------------------

/// Streaming or on its way: the button stops it
pub fn stream_active(state: StreamState) -> bool {
    matches!(state, StreamState::Streaming | StreamState::Connecting | StreamState::Reconnecting)
}

/// Disconnect while streaming, Cancel while connecting, else Connect
pub fn connect_label(state: StreamState) -> String {
    t(match state {
        StreamState::Streaming => "connection.disconnect",
        StreamState::Connecting | StreamState::Reconnecting => "modal.cancel",
        _ => "connection.connect",
    })
}

/// The stream state a device's card shows: the stream's for the device of
/// the profile streaming (or that last failed), idle for the others
pub fn entry_state(snap: &Snapshot, entry: &AudioEntry) -> StreamState {
    let s = &snap.stream;
    let Some(name) = s.profile.as_deref().filter(|_| s.state != StreamState::Idle) else { return StreamState::Idle };
    match snap.profiles.iter().find(|p| p.name == name) {
        Some(p) if device_key(&p.device_address, p.le_audio) == entry.key => s.state,
        _ => StreamState::Idle,
    }
}

/// The stack's start-up error while the adapter seems hung: AdapterHung
/// tells what to do instead of "check the WinUSB driver"
pub fn hidden_by_hung(snap: &Snapshot, message: Option<&Msg>) -> bool {
    snap.adapter.hung && message.is_some_and(|m| m.key.starts_with("error.btstack"))
}

// ---------------------------------------------------------------------------
// The headset microphone's chip (Home's HeadsetChip)
// ---------------------------------------------------------------------------

/// How the microphone's button shows
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadsetChip {
    pub muted: bool,
    /// Why the microphone did not open (MIC_NOTES), in words
    pub note: Option<String>,
    /// Not muted and nothing wrong
    pub on: bool,
    /// Auto with no microphone link open: dimmed
    pub waiting: bool,
    /// The microphone link is open
    pub linked: bool,
    /// "Disable microphone" / "Enable microphone"
    pub action: String,
    /// The note, else the action with the voice codec while linked
    pub title: String,
}

/// The headset microphone's mute at one click (Home's card and the
/// flyout): only for a device in use with headset mode on and its
/// microphone output chosen (None otherwise). Auto with no microphone link
/// open shows dimmed, and why the microphone did not open (MIC_NOTES) shows
/// on it, not as the card's status.
pub fn headset_chip(snap: &Snapshot, entry: &AudioEntry) -> Option<HeadsetChip> {
    let profile = entry.profile.as_ref();
    // Only while the device is connected or connecting (issue #86)
    if !mic_shown(snap, profile) {
        return None;
    }
    let p = profile?;
    let s = &snap.stream;
    let mode = headset_mode(&p.headset_mic, snap.settings.developer_mode);
    let muted = p.mic_muted;
    let ours = s.state != StreamState::Idle && s.profile.as_deref() == Some(p.name.as_str());
    let linked = ours && s.voice.is_some();
    let note = (ours && !muted && MIC_NOTES.contains(&s.message.key.as_str())).then(|| tm(Some(&s.message)));
    let action = t(if muted { "ui.headset_mic_unmute" } else { "ui.headset_mic_mute" });
    // The voice codec while the microphone link is open: "Disable microphone (mSBC)"
    let title = match (&note, s.voice.as_ref().filter(|_| linked)) {
        (Some(n), _) => n.clone(),
        (None, Some(v)) => format!("{action} ({})", voice_codec_name(&v.codec)),
        (None, None) => action.clone(),
    };
    let on = !muted && note.is_none();
    Some(HeadsetChip {
        muted,
        on,
        waiting: on && mode == HeadsetMode::Auto && !linked,
        linked: !muted && linked,
        note,
        action,
        title,
    })
}

// ---------------------------------------------------------------------------
// The navigation's status block (the device pill)
// ---------------------------------------------------------------------------

/// The state a device's block shows; without devices, the stream's
pub fn pill_state(snap: &Snapshot, entry: Option<&AudioEntry>) -> StreamState {
    entry.map_or(snap.stream.state, |e| entry_state(snap, e))
}

/// The block's state: an error the codec caused named (issue #86), else
/// the state in words
pub fn pill_state_text(snap: &Snapshot, state: StreamState) -> String {
    codec_stop_text(state, snap.stream.codec_stop.as_ref()).unwrap_or_else(|| state_label(state))
}

/// The device's name, else its address
pub fn entry_name(entry: &AudioEntry) -> &str {
    if entry.name.is_empty() { &entry.address } else { &entry.name }
}

/// The block's name (also while collapsed to the dot): the device, its
/// state, and the codec with what it runs with while live
pub fn pill_label(snap: &Snapshot, entry: Option<&AudioEntry>) -> String {
    let state = pill_state(snap, entry);
    let device = entry.map(entry_name).unwrap_or_default().to_owned();
    let codec = match snap.stream.info.as_ref().filter(|_| state != StreamState::Idle) {
        Some(i) => format!("{} {}", codec_name(i), codec_details(i).join(" ")),
        None => String::new(),
    };
    [device, pill_state_text(snap, state), codec].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(", ")
}

// ---------------------------------------------------------------------------
// Errors and their fixes
// ---------------------------------------------------------------------------

/// The next step for an error
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixAction {
    /// The adapter is not on WinUSB: assign it (the dialog says what it does)
    AssignWinusb(Box<AdapterInfo>),
    /// The Adapter screen
    Adapter,
    /// The Adapter screen's firmware section
    AdapterFirmware,
    /// The vendor's firmware download page
    FirmwarePage(&'static str),
    RestartAdapter,
    /// stream.start with this profile
    Retry(String),
    Codecs,
    Devices,
    Diagnostics,
}

impl FixAction {
    /// The button's text
    pub fn label(&self) -> String {
        t(match self {
            FixAction::AssignWinusb(_) => "ui.adapter.assign_winusb",
            FixAction::Adapter => "ui.nav.adapter",
            FixAction::AdapterFirmware => "ui.home.firmware_fix",
            FixAction::FirmwarePage(_) => "firmware.open_download_page",
            FixAction::RestartAdapter => "ui.adapter.restart",
            FixAction::Retry(_) => "ui.fix.retry",
            FixAction::Codecs => "ui.nav.codecs",
            FixAction::Devices => "ui.nav.devices",
            FixAction::Diagnostics => "ui.nav.diagnostics",
        })
    }
}

const RETRY_KEYS: [&str; 7] = [
    "error.connect_no_answer",
    "error.connect_link_lost",
    "error.connect_refused",
    "error.connect_auth_failed",
    "error.reconnect_failed",
    "error.pairing_hint",
    "error.stream_start",
];
const PROFILE_KEYS: [&str; 10] = [
    "error.codec_not_supported",
    "error.codec_adaptive_only",
    "error.codec_sep_in_use",
    "error.no_compatible_codec",
    "error.no_virtual_device_selected",
    "error.virtual_capture_init",
    "error.no_app_selected",
    "error.app_capture_unsupported",
    "error.app_capture_init",
    "error.aac_unavailable",
];

/// The adapter BlueCaster would use but cannot: not on WinUSB
pub fn adapter_needing_driver(snap: &Snapshot) -> Option<&AdapterInfo> {
    snap.adapter.adapters.iter().find(|a| a.driver != DriverKind::Winusb)
}

/// The adapters' drivers, to tell when one changed: a kept request error
/// ("no adapter on WinUSB") is out of date then
pub fn adapter_drivers(snap: &Snapshot) -> String {
    snap.adapter.adapters.iter().map(|a| format!("{}={:?}", a.instance_id, a.driver)).collect::<Vec<_>>().join("|")
}

/// The next step for an error: a code from a request, and/or the message key
pub fn fix_for(snap: &Snapshot, code: Option<&str>, key: &str) -> FixAction {
    let adapters = &snap.adapter.adapters;
    if code == Some("adapter.driver_missing") || key == "error.btstack_init" || key == "firmware.auto_no_adapter" {
        return match adapter_needing_driver(snap) {
            Some(a) => FixAction::AssignWinusb(Box::new(a.clone())),
            None => FixAction::Adapter,
        };
    }
    if key == "firmware.rom_running" {
        // The Adapter screen names the files and opens the download page
        return FixAction::AdapterFirmware;
    }
    if code == Some("adapter.firmware_missing") || key.starts_with("firmware.") {
        // The Adapter screen downloads the files where the core can fetch them
        if adapters.iter().any(|a| a.firmware_present == Some(false) && a.firmware_download) {
            return FixAction::AdapterFirmware;
        }
        let vendor = adapters.first().map_or("", |a| a.vendor.as_str());
        return FixAction::FirmwarePage(crate::links::firmware_url(vendor));
    }
    if key == "error.btstack_no_answer" || key.starts_with("error.btstack_crash") {
        return FixAction::RestartAdapter;
    }
    if RETRY_KEYS.contains(&key)
        && let Some(p) = current_profile(snap)
    {
        return FixAction::Retry(p.name.clone());
    }
    // The codec is chosen on Codecs, the capture source on Audio Devices
    if PROFILE_KEYS.contains(&key) {
        let codec =
            key.starts_with("error.codec") || key == "error.no_compatible_codec" || key == "error.aac_unavailable";
        return if codec { FixAction::Codecs } else { FixAction::Devices };
    }
    FixAction::Diagnostics
}

/// The text "copy the error" puts on the clipboard: the message, and the
/// code of a failed request (what an issue report needs)
pub fn error_copy_text(message: &Msg, code: Option<&str>) -> String {
    let text = tm(Some(message));
    let text = if text.is_empty() { message.key.clone() } else { text };
    match code.filter(|c| !c.is_empty()) {
        Some(c) => format!("{text}\n[{c}]"),
        None => text,
    }
}

/// Errors with a cause on the user's side (the device off or out of range,
/// pairing, the adapter's setup, a choice the profile lacks): their fix is
/// the answer, a bug report would only say "it does not connect"
const USER_SIDE_ERRORS: [&str; 26] = [
    "error.connect_no_answer",
    "error.connect_link_lost",
    "error.connect_refused",
    "error.connect_auth_failed",
    "error.pairing_hint",
    "error.reconnect_failed",
    "error.btstack_init",
    "error.btstack_no_answer",
    "error.codec_not_supported",
    "error.codec_adaptive_only",
    "error.codec_sep_in_use",
    "error.no_compatible_codec",
    "error.le_audio_disabled",
    "error.le_audio_developer",
    "error.le_audio_unavailable",
    "error.le_audio_connect",
    "error.le_audio_capture",
    "error.no_virtual_device_selected",
    "error.no_app_selected",
    "error.app_capture_unsupported",
    "error.uac_cancelled",
    "error.driver_assign_failed",
    "error.invalid_address",
    "error.invalid_address_format",
    "error.sbc_bitpool_range",
    "error.profile_not_found",
];
/// Request errors with such a cause: an input or choice to correct, a
/// prompt declined
const USER_SIDE_CODES: [&str; 10] = [
    "adapter.driver_missing",
    "adapter.uac_cancelled",
    "app.streaming",
    "device.invalid_address",
    "device.invalid_name",
    "profile.invalid",
    "profile.invalid_address",
    "profile.not_found",
    "stream.bad_codec",
    "stream.bad_source",
];

/// Whether an error points at BlueCaster itself (worth a bug report);
/// anything not known to be on the user's side is
pub fn is_app_fault(key: &str, code: Option<&str>) -> bool {
    !USER_SIDE_ERRORS.contains(&key) && !code.is_some_and(|c| USER_SIDE_CODES.contains(&c))
}

/// A bug report's URL with what the state tells (the issue form's fields)
/// around the diagnostic info; `extra`'s fields that are set win. Not the
/// device's name: it is often the owner's ("Taro's AirPods") and the issue
/// is public, so the headphones field stays for the user to fill in with
/// the model.
pub fn bug_report_for(snap: &Snapshot, info: &str, extra: BugReport) -> String {
    let profile = current_profile(snap);
    let adapters = &snap.adapter.adapters;
    let adapter = adapters.iter().find(|a| a.driver == DriverKind::Winusb).or(adapters.first());
    bug_report_url(&BugReport {
        version: snap.update.current_version.clone(),
        info: info.to_owned(),
        adapter: adapter.map(|a| format!("{} ({:04X}:{:04X})", a.display_name, a.vid, a.pid)),
        headphones: extra.headphones,
        codec: extra.codec.or_else(|| profile.map(|p| p.codec.clone())),
        capture_mode: extra.capture_mode.or_else(|| profile.map(|p| p.capture_mode.clone())),
        what_happened: extra.what_happened,
        bundle: extra.bundle,
    })
}

/// The core's diagnostic info (diag.info) without its code fence
pub fn diag_info_body(text: &str) -> String {
    let s = text.strip_prefix("```text\n").unwrap_or(text);
    match s.trim_end().strip_suffix("```") {
        Some(body) => format!("{}\n", body.strip_suffix('\n').unwrap_or(body)),
        None => s.to_owned(),
    }
}

/// What a report holds
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportText {
    /// All of it, for the clipboard
    pub text: String,
    /// The details and the info, for the issue's log field
    pub log: String,
}

/// A report from `head` (the error, the crash), `details` (a crash
/// report's text) and the diagnostic info with its warnings and recent log
/// (`info`, from [`diag_info_body`]; identifiers masked: it goes into a
/// public issue; the core masks its info, the UI the head and the details,
/// which name the user's folder in a crash report's dump path). Without the
/// core's info: what the UI has.
pub fn report_text(head: &str, details: &str, info: &str) -> ReportText {
    let head = mask_identifiers(head);
    let details = mask_identifiers(details.trim_end());
    let log = [details.as_str(), info].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n\n");
    let text = if log.is_empty() { head } else { format!("{head}\n\n```text\n{}\n```\n", log.trim_end()) };
    ReportText { text, log }
}

// ---------------------------------------------------------------------------
// What the main window shows around every screen
// ---------------------------------------------------------------------------

/// "HCI capture running" from v1's title suffix (" - [HCI capture running]")
pub fn capture_running_text() -> String {
    let s = t("debug.capture_title_suffix");
    let s = s.trim_start();
    let s = s.strip_prefix('-').map_or(s, str::trim_start);
    let s = s.strip_prefix('[').and_then(|x| x.strip_suffix(']')).unwrap_or(s);
    s.trim().to_owned()
}

/// The head of a crash's report: what stopped, when, why
pub fn crash_head(c: &CrashReport) -> String {
    format!("{}: {}, {}\n{}", t("ui.crash.title"), c.process, c.time, c.reason)
}

/// A line of a release's notes
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteLine {
    pub text: String,
    pub bullet: bool,
}

/// `[text](url)` -> text
fn strip_links(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let parsed = after.find(']').and_then(|close| {
            let tail = after[close + 1..].strip_prefix('(')?;
            let end = tail.find(')')?;
            Some((&after[..close], &tail[end + 1..]))
        });
        match parsed {
            Some((text, next)) => {
                out.push_str(&rest[..open]);
                out.push_str(text);
                rest = next;
            }
            None => {
                out.push_str(&rest[..=open]);
                rest = after;
            }
        }
    }
    out + rest
}

/// `<mark>text<mark>` -> text, where the text has no character of the mark
fn strip_marks(s: &str, mark: &str) -> String {
    let stop = mark.chars().next().unwrap_or('*');
    let mut out = String::new();
    let mut rest = s;
    while let Some(open) = rest.find(mark) {
        let after = &rest[open + mark.len()..];
        let end = after.find(stop);
        match end.filter(|&e| after[e..].starts_with(mark)) {
            Some(e) => {
                out.push_str(&rest[..open]);
                out.push_str(&after[..e]);
                rest = &after[e + mark.len()..];
            }
            None => {
                out.push_str(&rest[..open + stop.len_utf8()]);
                rest = &rest[open + stop.len_utf8()..];
            }
        }
    }
    out + rest
}

/// A release's in-app notes as lines to show: list marks become bullets,
/// Markdown emphasis and links plain text
pub fn note_lines(notes: &str) -> Vec<NoteLine> {
    notes
        .split('\n')
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| {
            let mut chars = l.chars();
            let first = chars.next();
            let bullet = matches!(first, Some('-' | '*' | '•')) && chars.next().is_some_and(char::is_whitespace);
            let text = if bullet { l[first.map_or(0, char::len_utf8)..].trim_start() } else { l };
            let text = if text.starts_with('#') { text.trim_start_matches('#').trim_start() } else { text };
            let text = strip_marks(&strip_marks(&strip_links(text), "**"), "`");
            NoteLine { text, bullet }
        })
        .collect()
}

#[cfg(test)]
mod tests;

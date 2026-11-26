//! The Diagnostics screen's texts and rules: the rows of the stream, radio,
//! capture and microphone tabs, the verbose logging choice, the Wi-Fi scan's
//! bands. The screen (app/src/ui: screens/diagnostics) only draws them.

use chrono::{Local, TimeZone};
use ipc::{
    LinkDetails, LinkMode, LinkRole, LinkSample, LogLevel, MicTestState, RemoteChip, StreamState, StreamStatus,
    VerboseState, VoiceStatus, WifiAccessPoint,
};

use crate::codecs::{bitrate_note_key, codec_label_key};
use crate::view::stream::voice_codec_name;

use crate::i18n::t;
use crate::series::{js_round, to_fixed};
use crate::t;
use crate::view::charts::Status;
use crate::view::stream::QUALITY_QUEUE_BACKLOG_MS;

/// An unknown value
pub const DASH: &str = "\u{2013}";

/// A number as JavaScript's `toFixed(digits)` writes it (the panels' figures)
pub fn fixed(x: f64, digits: usize) -> String {
    to_fixed(x, digits)
}

/// JavaScript's `Math.round` (a tie rounds up)
pub fn round(x: f64) -> f64 {
    js_round(x)
}

// ---- Diagnostics: the report card ---------------------------------------------

/// GitHub takes issue attachments up to this size
pub const GITHUB_ATTACHMENT_LIMIT: u64 = 25 * 1024 * 1024;

/// Verbose logging as the select offers it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerboseChoice {
    Off,
    Debug,
    Trace,
}

/// The select's value for the core's verbose state
pub fn verbose_choice(v: Option<&VerboseState>) -> VerboseChoice {
    match v {
        Some(v) if v.level == LogLevel::Trace => VerboseChoice::Trace,
        Some(_) => VerboseChoice::Debug,
        None => VerboseChoice::Off,
    }
}

/// The level diag.set_verbose asks for (None: back to info)
pub fn verbose_level(c: VerboseChoice) -> Option<LogLevel> {
    match c {
        VerboseChoice::Off => None,
        VerboseChoice::Debug => Some(LogLevel::Debug),
        VerboseChoice::Trace => Some(LogLevel::Trace),
    }
}

/// When verbose logging ends, as the local date and time ("11/24 15:05")
pub fn until_text(ms: u64) -> String {
    match Local.timestamp_millis_opt(ms as i64).earliest() {
        Some(d) => d.format("%-m/%-d %H:%M").to_string(),
        None => String::new(),
    }
}

/// The verbose row's hint
pub fn verbose_help(v: Option<&VerboseState>) -> String {
    match v {
        Some(v) => t!("ui.diag.verbose_until", until_text(v.until_ms)),
        None => t("ui.diag.verbose_help"),
    }
}

// ---- Stream tab ----------------------------------------------------------------

pub fn ms_text(v: f64) -> String {
    format!("{} ms", js_round(v))
}

pub fn count_text(v: f64) -> String {
    format!("{}", js_round(v))
}

/// "4.2 %", "12 %"
pub fn pct_text(v: f64) -> String {
    if v < 10.0 { format!("{} %", to_fixed(v, 1)) } else { format!("{} %", js_round(v)) }
}

/// "4.2 µs", "12 µs"
pub fn us_text(v: f64) -> String {
    if v < 10.0 { format!("{} µs", to_fixed(v, 1)) } else { format!("{} µs", js_round(v)) }
}

/// The send queue's colour: how full it is, by its wait where the core
/// measures it
pub fn queue_status(s: &LinkSample) -> Status {
    let f = match s.queue_ms {
        Some(ms) => ms as f64 / (2.0 * QUALITY_QUEUE_BACKLOG_MS as f64),
        None => s.queue_depth as f64 / s.queue_capacity.filter(|&c| c != 0).unwrap_or(64) as f64,
    };
    if f < 0.25 {
        Status::Good
    } else if f < 0.5 {
        Status::Warn
    } else {
        Status::Bad
    }
}

pub fn queue_text(s: Option<&LinkSample>) -> String {
    let Some(s) = s else { return DASH.into() };
    match s.queue_capacity.filter(|&c| c != 0) {
        Some(cap) => t!("quality.queue_value", s.queue_depth, cap),
        None => s.queue_depth.to_string(),
    }
}

pub fn wait_text(s: Option<&LinkSample>) -> String {
    match s.and_then(|s| s.queue_ms) {
        Some(ms) => ms_text(ms as f64),
        None => DASH.into(),
    }
}

/// The adapter's backlog: ms of audio, and the packets it held
pub fn backlog_text(s: Option<&LinkSample>) -> String {
    let Some(s) = s else { return DASH.into() };
    let Some(packets) = s.controller_backlog else { return DASH.into() };
    match s.controller_backlog_ms {
        Some(ms) => t!("ui.diag.backlog_value", ms_text(ms), packets),
        None => t!("ui.diag.backlog_packets", packets),
    }
}

/// The playout delay the device reported (not every device does)
pub fn sink_delay_text(s: Option<&LinkSample>) -> String {
    match s.and_then(|s| s.sink_delay_ms) {
        Some(d) => t!("quality.sink_delay_value", d),
        None => DASH.into(),
    }
}

/// The encoder's time per time of audio, and per frame
pub fn encode_text(s: Option<&LinkSample>) -> String {
    match s.and_then(|s| s.encode.as_deref()) {
        Some(e) => t!("ui.diag.encode_load_value", pct_text(e.load * 100.0), us_text(e.avg_us), us_text(e.max_us)),
        None => DASH.into(),
    }
}

/// Packets per second (and their average size) of the last sample sending
pub fn packets_text(s: Option<&LinkSample>) -> String {
    let Some(s) = s else { return DASH.into() };
    match s.avg_packet_bytes {
        Some(b) => t!("quality.packets_value", s.packets_per_second, b),
        None => format!("{} /s", to_fixed(s.packets_per_second, 1)),
    }
}

/// The call link of a classic device: A2DP is suspended meanwhile
pub fn voice_link(stream: &StreamStatus) -> Option<&VoiceStatus> {
    stream.voice.as_ref().filter(|v| stream.state == StreamState::Streaming && v.link == "hfp")
}

/// The codec row: the call link's voice codec, else the A2DP codec with its setup
pub fn codec_text(stream: &StreamStatus) -> String {
    if let Some(v) = voice_link(stream) {
        return t!("ui.diag.voice_codec_value", voice_codec_name(&v.codec), v.rate as f64 / 1000.0);
    }
    let Some(i) = stream.info.as_ref().filter(|_| stream.state == StreamState::Streaming) else { return DASH.into() };
    let key = codec_label_key(&i.codec);
    let name = if key.is_empty() { i.codec.clone() } else { t(key) };
    let note = bitrate_note_key(i.abr, i.bitrate_capped).map(t).unwrap_or_default();
    t!("quality.codec_value", name, i.sample_rate as f64 / 1000.0, i.channels, i.bitrate_kbps) + &note
}

/// An ISO counter, or the dash without counters
pub fn iso_count(v: Option<u64>) -> String {
    v.map_or_else(|| DASH.into(), |v| v.to_string())
}

// ---- Radio tab -----------------------------------------------------------------

/// The highest the golden receive range's lower edge may sit (Core spec):
/// a reading below the range is at most this plus the reading
pub const GOLDEN_LOW_MAX_DBM: i32 = -56;

/// BR/EDR hops over 79 channels
pub const BR_CHANNELS: f64 = 79.0;

pub fn rssi_db(v: f64) -> String {
    format!("{}{} dB", if v > 0.0 { "+" } else { "" }, js_round(v))
}

/// The reading, and below the range the most the actual level can be
pub fn rssi_value(v: f64) -> String {
    if v < 0.0 { t!("ui.diag.rssi_value", rssi_db(v), js_round(v) as i32 + GOLDEN_LOW_MAX_DBM) } else { rssi_db(v) }
}

/// The chip's fastest packet type and its rate ("3-DH5 (3 Mbps)")
pub fn chip_packet_text(c: Option<&RemoteChip>) -> String {
    let Some(f) = c.and_then(|c| c.features.as_ref()) else { return DASH.into() };
    t!(
        "ui.diag.chip_packet_value",
        &f.largest_packet,
        if f.edr_3m {
            3
        } else if f.edr_2m {
            2
        } else {
            1
        }
    )
}

/// "20 s", "1.25 s"
pub fn seconds_text(ms: f64) -> String {
    let s = to_fixed(ms / 1000.0, 2);
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_owned() } else { s };
    format!("{s} s")
}

/// Role, with the changes since the stream started
pub fn role_text(d: &LinkDetails) -> String {
    let Some(role) = d.role else { return DASH.into() };
    let role = t(if role == LinkRole::Central { "ui.diag.role_central" } else { "ui.diag.role_peripheral" });
    if d.role_changes > 0 { t!("ui.diag.changes_value", role, d.role_changes) } else { role }
}

/// Mode (sniff with its interval), with the sniff entries since the stream started
pub fn mode_text(d: &LinkDetails) -> String {
    let Some(mode) = d.mode else { return DASH.into() };
    let mode = match (mode, d.sniff_interval_ms) {
        (LinkMode::Sniff, Some(i)) => t!("ui.diag.mode_sniff_value", js_round(i)),
        (LinkMode::Active, _) => t("ui.diag.mode_active"),
        (LinkMode::Hold, _) => t("ui.diag.mode_hold"),
        (LinkMode::Sniff, None) => t("ui.diag.mode_sniff"),
        (LinkMode::Park, _) => t("ui.diag.mode_park"),
    };
    if d.sniff_entries > 0 { t!("ui.diag.sniff_entries_value", mode, d.sniff_entries) } else { mode }
}

/// Failed contacts since the stream started, and their rate
pub fn failed_text(d: &LinkDetails) -> String {
    let Some(n) = d.failed_contacts_stream else { return DASH.into() };
    match d.failed_contacts_per_second {
        Some(rate) if n > 0 && rate > 0.0 => format!("{n} ({} /s)", to_fixed(rate, 1)),
        _ => n.to_string(),
    }
}

pub fn link_quality_text(d: &LinkDetails) -> String {
    d.link_quality.map_or_else(|| DASH.into(), |q| t!("ui.diag.of_255", q))
}

pub fn key_size_text(d: &LinkDetails) -> String {
    d.encryption_key_size.map_or_else(|| DASH.into(), |k| t!("ui.diag.bytes_value", k))
}

pub fn supervision_text(d: &LinkDetails) -> String {
    d.supervision_timeout_ms.map_or_else(|| DASH.into(), seconds_text)
}

/// Channel hopping: in use, usable, and the Wi-Fi channels avoided
pub fn afh_text(s: Option<&LinkSample>) -> String {
    let Some(s) = s else { return DASH.into() };
    let mut parts = Vec::new();
    if let Some(n) = s.afh_in_use {
        parts.push(t!("radio.afh", n));
    }
    if let Some(n) = s.afh_usable {
        parts.push(t!("ui.diag.afh_usable", n));
    }
    if parts.is_empty() {
        return DASH.into();
    }
    let avoid = if s.afh_usable.is_some() && !s.afh_avoided.is_empty() {
        t!("radio.afh_avoid", &s.afh_avoided)
    } else {
        String::new()
    };
    parts.join(&t("radio.separator")) + &avoid
}

/// The share of the radio's time the stream takes, with its packet type
pub fn air_text(s: Option<&LinkSample>) -> String {
    let Some(s) = s else { return DASH.into() };
    let p = s.airtime * 100.0;
    match &s.acl_packet {
        Some(a) => t!("quality.airtime_value", p, &a.name, a.slots),
        None => format!("{} %", to_fixed(p, 1)),
    }
}

// ---- Audio capture tab --------------------------------------------------------

pub fn khz_text(v: f64) -> String {
    format!("{} kHz", to_fixed(v / 1000.0, 1))
}

/// Capture losses in ms: a decimal below 10
pub fn capture_ms_text(v: f64) -> String {
    if v < 10.0 && v > 0.0 { format!("{} ms", to_fixed(v, 1)) } else { format!("{} ms", js_round(v)) }
}

/// Windows' gaps: the count, and the audio lost in them when known
pub fn gaps_text(gaps: u64, lost_ms: Option<f64>) -> String {
    match lost_ms {
        Some(ms) if ms > 0.0 => t!("ui.diag.capture_gaps_value", gaps, capture_ms_text(ms)),
        _ => gaps.to_string(),
    }
}

// ---- Microphone tab -----------------------------------------------------------

/// Segments of the level bar
pub const MIC_SEGMENTS: usize = 48;
/// The bar's floor in dBFS: quieter is dark
pub const MIC_FLOOR_DB: f64 = -54.0;
/// How fast the bar falls back per poll (peak hold)
pub const MIC_FALL: f64 = 0.82;

/// A peak level 0..1 as the lit share of the bar (dB scale)
pub fn bar_share(level: f64) -> f64 {
    if level <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * level.log10();
    ((db - MIC_FLOOR_DB) / -MIC_FLOOR_DB).clamp(0.0, 1.0)
}

fn mic_seconds(ms: u32) -> String {
    format!("{} s", to_fixed(ms as f64 / 1000.0, 1))
}

/// The test's time: recorded of the longest, played of the recording
pub fn mic_progress(s: &MicTestState) -> String {
    match s.phase.as_str() {
        "recording" => format!("{} / {}", mic_seconds(s.recorded_ms), mic_seconds(s.max_ms)),
        "playing" => format!("{} / {}", mic_seconds(s.played_ms), mic_seconds(s.recorded_ms)),
        "recorded" => mic_seconds(s.recorded_ms),
        _ => String::new(),
    }
}

// ---- Connection flow tab ------------------------------------------------------

/// "+1.234 s" since the attempt started
pub fn elapsed_text(ms: Option<u64>) -> String {
    ms.map_or_else(String::new, |ms| format!("+{} s", to_fixed(ms as f64 / 1000.0, 3)))
}

// ---- Radio tab: the Wi-Fi scan ------------------------------------------------

/// A Wi-Fi band
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    G2_4,
    G5,
    G6,
}

impl Band {
    /// "2.4", "5", "6"
    pub fn as_str(self) -> &'static str {
        match self {
            Band::G2_4 => "2.4",
            Band::G5 => "5",
            Band::G6 => "6",
        }
    }
}

/// The scan table's filter: 2.4 GHz first, the only band Bluetooth shares
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BandFilter {
    #[default]
    G2_4,
    G5,
    G6,
    All,
}

pub fn band_of(mhz: u32) -> Band {
    if mhz < 2500 {
        Band::G2_4
    } else if mhz < 5925 {
        Band::G5
    } else {
        Band::G6
    }
}

/// The access points the band drawing can show (2.4 GHz)
pub fn near(scan: &[WifiAccessPoint]) -> Vec<&WifiAccessPoint> {
    scan.iter().filter(|ap| ap.mhz < 2500).collect()
}

/// The table's rows
pub fn shown(scan: &[WifiAccessPoint], filter: BandFilter) -> Vec<&WifiAccessPoint> {
    scan.iter()
        .filter(|ap| {
            let band = band_of(ap.mhz);
            match filter {
                BandFilter::All => true,
                BandFilter::G2_4 => band == Band::G2_4,
                BandFilter::G5 => band == Band::G5,
                BandFilter::G6 => band == Band::G6,
            }
        })
        .collect()
}

/// The text under an empty table: the 2.4 GHz band has its own
pub fn scan_empty_key(filter: BandFilter) -> &'static str {
    if filter == BandFilter::G2_4 { "ui.radio.scan_empty" } else { "ui.radio.scan_none" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipc::{EncodeLoad, RemoteChipFeatures};

    fn features(edr_2m: bool, edr_3m: bool, largest: &str) -> RemoteChip {
        RemoteChip {
            company_id: None,
            company: None,
            version: None,
            lmp_version: None,
            lmp_subversion: None,
            features: Some(RemoteChipFeatures {
                edr_2m,
                edr_3m,
                three_slot: true,
                five_slot: true,
                edr_three_slot: true,
                edr_five_slot: true,
                largest_packet: largest.into(),
            }),
        }
    }

    // The series of the capture and radio tabs
    #[test]
    fn gaps_row_is_the_count_with_the_audio_lost_when_known() {
        assert_eq!(gaps_text(0, Some(0.0)), "0");
        assert_eq!(gaps_text(3, Some(0.0)), "3");
        assert_eq!(gaps_text(3, None), "3");
        assert_eq!(gaps_text(2, Some(40.0)), "2 (40 ms lost)");
    }

    #[test]
    fn names_the_remote_chips_fastest_packet_type_with_its_rate() {
        assert_eq!(chip_packet_text(Some(&features(true, true, "3-DH5"))), "3-DH5 (3 Mbps)");
        assert_eq!(chip_packet_text(Some(&features(true, false, "2-DH5"))), "2-DH5 (2 Mbps)");
        assert_eq!(chip_packet_text(Some(&features(false, false, "DH5"))), "DH5 (1 Mbps)");
        let company_only = RemoteChip { company: Some("Airoha".into()), features: None, ..features(true, true, "") };
        assert_eq!(chip_packet_text(Some(&company_only)), DASH);
        assert_eq!(chip_packet_text(None), DASH);
    }

    // The Diagnostics screen
    #[test]
    fn verbose_logging_choice_follows_the_cores_state() {
        assert_eq!(verbose_choice(None), VerboseChoice::Off);
        let v = VerboseState { level: LogLevel::Debug, until_ms: 0 };
        assert_eq!(verbose_choice(Some(&v)), VerboseChoice::Debug);
        assert_eq!(verbose_choice(Some(&VerboseState { level: LogLevel::Trace, ..v })), VerboseChoice::Trace);
        assert_eq!(verbose_level(VerboseChoice::Debug), Some(LogLevel::Debug));
        assert_eq!(verbose_level(VerboseChoice::Off), None);
        assert!(verbose_help(Some(&v)).starts_with("On until "));
        assert_eq!(verbose_help(None), t("ui.diag.verbose_help"));
    }

    #[test]
    fn shows_the_links_details_in_words() {
        let d = LinkDetails {
            role: Some(LinkRole::Central),
            role_changes: 0,
            mode: Some(LinkMode::Active),
            sniff_interval_ms: None,
            sniff_entries: 0,
            encryption_key_size: Some(16),
            supervision_timeout_ms: Some(20000.0),
            failed_contacts: Some(0),
            failed_contacts_stream: Some(3),
            failed_contacts_per_second: Some(0.25),
            link_quality: Some(231),
        };
        assert_eq!(role_text(&d), "Central");
        assert_eq!(mode_text(&d), "Active");
        assert_eq!(key_size_text(&d), "16 bytes");
        assert_eq!(supervision_text(&d), "20 s");
        assert_eq!(link_quality_text(&d), "231 / 255");
        assert_eq!(failed_text(&d), "3 (0.3 /s)");
        let sniff = LinkDetails {
            role: Some(LinkRole::Peripheral),
            role_changes: 2,
            mode: Some(LinkMode::Sniff),
            sniff_interval_ms: Some(52.5),
            sniff_entries: 1,
            supervision_timeout_ms: Some(1250.0),
            ..d.clone()
        };
        assert_eq!(role_text(&sniff), "Peripheral (changes: 2)");
        assert_eq!(mode_text(&sniff), "Sniff, every 53 ms (sniff entries: 1)");
        assert_eq!(seconds_text(1250.0), "1.25 s");
        let none = LinkDetails { role: None, mode: None, link_quality: None, failed_contacts_stream: None, ..d };
        assert_eq!(role_text(&none), DASH);
        assert_eq!(mode_text(&none), DASH);
        assert_eq!(link_quality_text(&none), DASH);
        assert_eq!(failed_text(&none), DASH);
    }

    #[test]
    fn stream_rows() {
        let s = LinkSample {
            controller_backlog: Some(4),
            controller_backlog_ms: Some(31.6),
            queue_depth: 3,
            queue_capacity: Some(64),
            queue_ms: Some(20),
            packets_per_second: 87.5,
            encode: Some(Box::new(EncodeLoad { load: 0.042, avg_us: 3.25, max_us: 120.0 })),
            ..Default::default()
        };
        // The backlog in ms, the packets beside it
        assert_eq!(backlog_text(Some(&s)), "32 ms (4 packets)");
        assert_eq!(backlog_text(Some(&LinkSample { controller_backlog_ms: None, ..s.clone() })), "4 packets");
        assert_eq!(backlog_text(None), DASH);
        assert_eq!(queue_text(Some(&s)), "3 / 64 packets");
        assert_eq!(wait_text(Some(&s)), "20 ms");
        assert_eq!(queue_status(&s), Status::Good);
        assert_eq!(queue_status(&LinkSample { queue_ms: Some(100), ..s.clone() }), Status::Warn);
        assert_eq!(queue_status(&LinkSample { queue_ms: Some(150), ..s.clone() }), Status::Bad);
        assert_eq!(queue_status(&LinkSample { queue_ms: None, queue_depth: 40, ..s.clone() }), Status::Bad);
        assert_eq!(packets_text(Some(&s)), "87.5 /s");
        assert_eq!(encode_text(Some(&s)), "4.2 % (per frame: avg 3.3 µs, max 120 µs)");
        assert_eq!(pct_text(12.4), "12 %");
    }

    #[test]
    fn radio_rows() {
        assert_eq!(rssi_db(3.0), "+3 dB");
        assert_eq!(rssi_db(0.0), "0 dB");
        assert_eq!(rssi_value(-10.0), "-10 dB (≤ -66 dBm)");
        let s = LinkSample { afh_in_use: Some(52), afh_usable: Some(79), ..Default::default() };
        assert_eq!(afh_text(Some(&s)), "AFH 52/79 ch, 79 usable by host classification");
        assert_eq!(afh_text(Some(&LinkSample::default())), DASH);
        assert_eq!(air_text(Some(&LinkSample { airtime: 0.123, ..Default::default() })), "12.3 %");
    }

    #[test]
    fn capture_rows() {
        assert_eq!(khz_text(48000.0), "48.0 kHz");
        assert_eq!(capture_ms_text(4.25), "4.3 ms");
        assert_eq!(capture_ms_text(0.0), "0 ms");
    }

    #[test]
    fn microphone_bar_and_time() {
        assert_eq!(bar_share(0.0), 0.0);
        assert_eq!(bar_share(1.0), 1.0);
        assert!((bar_share(0.5) - (1.0 - 6.0206 / 54.0)).abs() < 1e-3);
        assert_eq!(bar_share(0.0001), 0.0);
        let st = MicTestState { phase: "recording".into(), recorded_ms: 1500, max_ms: 10000, ..Default::default() };
        assert_eq!(mic_progress(&st), "1.5 s / 10.0 s");
        assert_eq!(mic_progress(&MicTestState { phase: "recorded".into(), ..st.clone() }), "1.5 s");
        assert_eq!(mic_progress(&MicTestState { phase: "idle".into(), ..st }), "");
        assert_eq!(elapsed_text(Some(1234)), "+1.234 s");
        assert_eq!(elapsed_text(None), "");
    }

    #[test]
    fn scans_wifi_by_band() {
        let ap = |ssid: &str, channel: u8, mhz: u32| WifiAccessPoint { ssid: ssid.into(), channel, mhz, rssi: -50 };
        let scan = vec![ap("home-2g", 6, 2437), ap("home-5g", 36, 5180), ap("x", 1, 2412), ap("", 11, 2462)];
        // 2.4 GHz first, the drawing 2.4 GHz only
        assert_eq!(shown(&scan, BandFilter::G2_4).len(), 3);
        assert_eq!(shown(&scan, BandFilter::G5).len(), 1);
        assert_eq!(shown(&scan, BandFilter::G6).len(), 0);
        assert_eq!(shown(&scan, BandFilter::All).len(), 4);
        assert_eq!(near(&scan).len(), 3);
        assert_eq!(band_of(5180).as_str(), "5");
        assert_eq!(band_of(5955), Band::G6);
        assert_eq!(scan_empty_key(BandFilter::G6), "ui.radio.scan_none");
    }
}

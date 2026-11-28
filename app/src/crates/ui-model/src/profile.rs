//! A new profile's defaults and the checks before profile.save. The values
//! are those of v1's profile dialog; the core checks again.
//!
//! Since the Codecs screen (issue #86) the UI keeps one profile per device
//! entry (its address, and whether it is the LE Audio entry): the codec card
//! chosen changes that profile, and the settings of the codecs it is not
//! using wait in its codec_settings. The core folds older extra profiles
//! into it at start (crates/core/src/profiles.rs consolidate).
//!
//! [`app_capture_support_from`] takes the platform and its version as
//! values, so the tests can give any.

use ipc::{
    AudioDevice, CodecValues, Device, HeadsetMicMode, MediaEq, Method, MicProcessing, Profile, Settings, Snapshot,
    StreamState, StreamStatus,
};

use crate::codecs::{
    CodecCaps, CodecFields, PACKET_DEFAULT, PACKET_MAX, PACKET_MIN, card_codec, card_of, carry_to_codec, fit_to_codec,
};
use crate::format::{js_space, js_trim, parse_address};
use crate::i18n::Arg;

/// What belongs to the audio device rather than the codec: the capture
/// source. Set on the Audio Devices screen and kept the same in all the
/// device's profiles (issue #86). The max media packet size is the device's
/// too, set on the Developer page; a profile's own Windows Volume Sync is
/// no longer used (Settings.volume_sync, for every device).
pub const DEVICE_FIELDS: [&str; 6] =
    ["capture_mode", "audio_device_id", "audio_device_name", "auto_switch_device", "app_exe", "app_name"];

/// The fields of [`DEVICE_FIELDS`]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeviceFields {
    pub capture_mode: String,
    pub audio_device_id: String,
    pub audio_device_name: String,
    pub auto_switch_device: bool,
    pub app_exe: String,
    pub app_name: String,
}

impl DeviceFields {
    /// Sets them in a profile
    pub fn apply_to(&self, p: &mut Profile) {
        p.capture_mode = self.capture_mode.clone();
        p.audio_device_id = self.audio_device_id.clone();
        p.audio_device_name = self.audio_device_name.clone();
        p.auto_switch_device = self.auto_switch_device;
        p.app_exe = self.app_exe.clone();
        p.app_name = self.app_name.clone();
    }
}

pub fn device_fields(p: &Profile) -> DeviceFields {
    DeviceFields {
        capture_mode: p.capture_mode.clone(),
        audio_device_id: p.audio_device_id.clone(),
        audio_device_name: p.audio_device_name.clone(),
        auto_switch_device: p.auto_switch_device,
        app_exe: p.app_exe.clone(),
        app_name: p.app_name.clone(),
    }
}

/// The device's profiles (address in any case)
pub fn profiles_of<'a>(profiles: &'a [Profile], address: &str) -> Vec<&'a Profile> {
    let a = address.to_uppercase();
    profiles.iter().filter(|p| p.device_address.to_uppercase() == a).collect()
}

/// A device's settings are complete: a virtual or output device or an app chosen when captured from
pub fn device_fields_complete(f: &DeviceFields) -> bool {
    match f.capture_mode.as_str() {
        "virtual" | "output" => !f.audio_device_id.is_empty(),
        "app" => !f.app_exe.is_empty(),
        _ => true,
    }
}

/// The device's settings as its profiles have them (the first one's), for a new profile of it
pub fn device_defaults(profiles: &[Profile], address: &str) -> Option<DeviceFields> {
    profiles_of(profiles, address).first().map(|p| device_fields(p))
}

/// Headset mode (Profile.headset_mic): Auto opens the
/// microphone link only while an app records from the cable; the core
/// switches it
pub type HeadsetMode = HeadsetMicMode;

/// Every mode a profile may hold
pub const HEADSET_MODES: [HeadsetMode; 3] = [HeadsetMode::Off, HeadsetMode::Auto, HeadsetMode::On];

/// The modes offered: On (the microphone kept open) only in developer mode
/// (issue #86)
pub fn headset_modes(developer: bool) -> &'static [HeadsetMode] {
    if developer { &HEADSET_MODES } else { &HEADSET_MODES[..2] }
}

/// A profile's headset mode in force (`headset_mic`: Profile.headset_mic);
/// anything unknown reads as Off. On outside developer mode works as Auto,
/// as the core takes it, and comes back with developer mode (the saved value
/// stays).
pub fn headset_mode(headset_mic: &str, developer: bool) -> HeadsetMode {
    let mode = HEADSET_MODES.into_iter().find(|m| m.as_str() == headset_mic).unwrap_or(HeadsetMode::Off);
    if mode == HeadsetMode::On && !developer { HeadsetMode::Auto } else { mode }
}

/// The headset microphone's processing all off: the microphone as it comes
pub const MIC_PROCESSING_OFF: MicProcessing =
    MicProcessing { enabled: false, gain_db: 0, suppression: 0, gate_db: -80 };

pub fn new_profile() -> Profile {
    Profile {
        name: String::new(),
        device_address: String::new(),
        device_name: String::new(),
        codec: "auto".into(),
        quality: "hq".into(),
        abr: false,
        lhdcv5_low_latency: false,
        codec_fallback: true,
        codec_forced: false,
        unlisted_options: false,
        sample_rate: 0,
        bit_depth: 0,
        // Every new device captures System Loopback (issue #86: it plays with
        // nothing installed); a virtual device or one app is chosen on Audio
        // Devices
        capture_mode: "loopback".into(),
        audio_device_id: String::new(),
        audio_device_name: String::new(),
        // As v1 and the core: follow the virtual device as the default output
        auto_switch_device: true,
        app_exe: String::new(),
        app_name: String::new(),
        volume_sync: "auto".into(),
        volume_device_id: String::new(),
        volume_device_name: String::new(),
        max_media_payload: PACKET_DEFAULT,
        le_audio: false,
        // Not a hearing aid, and none beside it
        hearing_aid: false,
        hearing_aid_beside: String::new(),
        hearing_aid_delay_ms: 0,
        hearing_aid_volume: -20,
        // LE Audio: LC3 unless LC3 LL is chosen (issue #86)
        le_low_latency: false,
        // LE Audio: the channels as planned; an earbud alone plays mono
        le_swap_channels: false,
        le_alone_channel: String::new(),
        headset_mic: "auto".into(),
        mic_device_id: String::new(),
        mic_device_name: String::new(),
        mic_muted: false,
        mic_default_input: true,
        voice_codec: String::new(),
        mic_rate: 0,
        mic_processing: MIC_PROCESSING_OFF,
        sbc_bitpool: 0,
        sbcxq_bitpool: 0,
        sbc_min_bitpool: 0,
        sbcxq_min_bitpool: 0,
        sbc_blocks: 0,
        sbc_subbands: 0,
        sbc_allocation: String::new(),
        sbc_channel_mode: String::new(),
        codec_settings: Default::default(),
        codec_order: Vec::new(),
        media_eq: MediaEq::default(),
    }
}

/// One saved audio device as the Codecs screen, Home and the sidebar list
/// it: its entry and its profile (None until a codec is chosen or it is
/// connected)
#[derive(Debug, Clone, PartialEq)]
pub struct AudioEntry {
    pub key: String,
    pub address: String,
    pub name: String,
    pub le_audio: bool,
    pub device: Device,
    pub profile: Option<Profile>,
}

/// What [`device_profile`] and [`own_default_output`] read of the snapshot
/// (Pick<Snapshot, "profiles" | "active_profile" | "stream">)
#[derive(Debug, Clone, Copy)]
pub struct ProfilesView<'a> {
    pub profiles: &'a [Profile],
    pub active_profile: Option<&'a str>,
    pub stream: &'a StreamStatus,
}

impl<'a> From<&'a Snapshot> for ProfilesView<'a> {
    fn from(s: &'a Snapshot) -> Self {
        ProfilesView { profiles: &s.profiles, active_profile: s.active_profile.as_deref(), stream: &s.stream }
    }
}

/// The profile of a device entry: the one streaming, else the active one,
/// else the first (the core keeps one per entry; more only until it starts
/// again)
pub fn device_profile<'a>(snap: ProfilesView<'a>, address: &str, le_audio: bool) -> Option<&'a Profile> {
    let mine: Vec<&Profile> =
        profiles_of(snap.profiles, address).into_iter().filter(|p| p.le_audio == le_audio).collect();
    let streaming = if snap.stream.state != StreamState::Idle { snap.stream.profile.as_deref() } else { None };
    mine.iter()
        .find(|p| Some(p.name.as_str()) == streaming)
        .or_else(|| mine.iter().find(|p| Some(p.name.as_str()) == snap.active_profile))
        .or(mine.first())
        .copied()
}

/// LE Audio in force: its setting counts only in developer mode (issue
/// #86), as the core has it
pub fn le_audio_on(s: &Settings) -> bool {
    s.le_audio_experimental && s.developer_mode
}

/// Hearing aids in force: their own setting, only in developer mode, as
/// the core has it (with or without LE Audio)
pub fn hearing_aids_on(s: &Settings) -> bool {
    s.hearing_aids && s.developer_mode
}

/// The saved devices with their profiles, in the saved list's order; LE
/// Audio entries only with LE Audio on, hearing aids only with hearing aids
/// on.
pub fn audio_entries(snap: &Snapshot) -> Vec<AudioEntry> {
    listed_devices(&snap.devices, le_audio_on(&snap.settings), hearing_aids_on(&snap.settings))
        .into_iter()
        .filter(|d| d.saved)
        .map(|d| AudioEntry {
            key: device_key(&d.address, d.le_audio),
            address: d.address.to_uppercase(),
            name: d.name.clone(),
            le_audio: d.le_audio,
            profile: device_profile(snap.into(), &d.address, d.le_audio).cloned(),
            device: d,
        })
        .collect()
}

/// A list's entry by its key, kept for a moment after it left the list
/// (lists of devices are keyed: a device keeps its component across updates)
#[derive(Debug, Clone)]
pub struct EntryByKey {
    pub key: String,
    last: Option<AudioEntry>,
}

impl EntryByKey {
    pub fn new(key: impl Into<String>) -> EntryByKey {
        EntryByKey { key: key.into(), last: None }
    }

    /// The entry in `list`, else the last one seen
    pub fn get(&mut self, list: &[AudioEntry]) -> Option<&AudioEntry> {
        if let Some(e) = list.iter().find(|e| e.key == self.key) {
            self.last = Some(e.clone());
        }
        self.last.as_ref()
    }
}

/// The profile a device entry starts from before it has its own: Auto,
/// and the device settings of its other entry's profile, else the defaults
/// (System Loopback)
pub fn entry_base(address: &str, name: &str, le_audio: bool, profiles: &[Profile]) -> Profile {
    let mut p =
        Profile { device_address: address.to_uppercase(), device_name: name.to_owned(), le_audio, ..new_profile() };
    if let Some(f) = device_defaults(profiles, address) {
        f.apply_to(&mut p);
    }
    p
}

/// [`entry_base`] for a device of the list: a hearing aid's profile says so
pub fn device_base(device: &Device, profiles: &[Profile]) -> Profile {
    let mut p = entry_base(&device.address, &device.name, device.le_audio, profiles);
    p.hearing_aid = device.le_audio && device.hearing_aid;
    p
}

/// The saved hearing aids (ASHA, a proof of concept), listed while hearing
/// aids are on: what a classic device can have beside it, for the other ear
pub fn saved_hearing_aids(snap: &Snapshot) -> Vec<&Device> {
    if !hearing_aids_on(&snap.settings) {
        return Vec::new();
    }
    snap.devices.iter().filter(|d| d.saved && d.le_audio && d.hearing_aid).collect()
}

/// The entry a profile belongs to
pub fn entry_of<'a>(entries: &'a [AudioEntry], p: Option<&Profile>) -> Option<&'a AudioEntry> {
    let p = p?;
    let key = device_key(&p.device_address, p.le_audio);
    entries.iter().find(|e| e.key == key)
}

/// The settings of the codec a profile uses
pub fn codec_values(p: &Profile) -> CodecValues {
    CodecValues {
        quality: p.quality.clone(),
        sample_rate: p.sample_rate,
        bit_depth: p.bit_depth,
        abr: p.abr,
        lhdcv5_low_latency: p.lhdcv5_low_latency,
        sbc_bitpool: p.sbc_bitpool,
        sbcxq_bitpool: p.sbcxq_bitpool,
        sbc_min_bitpool: p.sbc_min_bitpool,
        sbcxq_min_bitpool: p.sbcxq_min_bitpool,
        sbc_blocks: p.sbc_blocks,
        sbc_subbands: p.sbc_subbands,
        sbc_allocation: p.sbc_allocation.clone(),
        sbc_channel_mode: p.sbc_channel_mode.clone(),
        unlisted_options: p.unlisted_options,
    }
}

/// Settings an earlier version kept for aptX Adaptive in Low Latency (the
/// core moves them to aptX Adaptive LL's card when it loads the profile)
fn legacy_ll(v: Option<&CodecValues>) -> bool {
    v.is_some_and(|v| v.quality == "ll")
}

/// The settings kept for a codec card (aptX Adaptive LL: those an earlier
/// version kept for aptX Adaptive in Low Latency, until it has its own; never
/// aptX Adaptive's own, issue #86)
pub fn kept_for<'a>(p: &'a Profile, card: &str) -> Option<&'a CodecValues> {
    let kept = &p.codec_settings;
    if card == "aptxadll" && !kept.contains_key(card) && legacy_ll(kept.get("aptxad")) {
        return kept.get("aptxad");
    }
    if card == "aptxad" && legacy_ll(kept.get("aptxad")) {
        return None;
    }
    kept.get(card)
}

/// The profile with the codec of card `codec` chosen: the settings of the
/// card it used go to its codec_settings, and `codec` gets `values` (its
/// "..." dialog's), else what it was set to before (issue #86: AAC's VBR came
/// back off), else what carries over from the codec it leaves ([`carry_to_codec`]).
/// aptX Adaptive LL's card is saved as aptX Adaptive in Low Latency.
pub fn with_codec(p: &Profile, codec: &str, values: Option<&CodecValues>) -> Profile {
    let from = card_of(&p.codec, &p.quality).to_owned();
    let mut kept = p.codec_settings.clone();
    let back = values.or_else(|| kept_for(p, codec)).cloned();
    // An earlier version's aptX Adaptive in Low Latency is LL's (issue #86)
    if legacy_ll(kept.get("aptxad")) {
        let old = kept.remove("aptxad").expect("checked");
        kept.entry("aptxadll".into()).or_insert(old);
    }
    if from != codec {
        kept.insert(from, codec_values(p));
    }
    kept.remove(codec);
    let base = Profile { codec_settings: kept, ..p.clone() };
    // Values of its own come back as they were; else what carries over from the codec it leaves
    let next = match back {
        Some(v) => {
            let mut b = base;
            let mut c = b.codec_choice();
            c.codec = codec.to_owned();
            c.quality = v.quality;
            c.sample_rate = v.sample_rate;
            c.bit_depth = v.bit_depth;
            c.abr = v.abr;
            c.lhdcv5_low_latency = v.lhdcv5_low_latency;
            c.sbc_bitpool = v.sbc_bitpool;
            c.sbcxq_bitpool = v.sbcxq_bitpool;
            c.sbc_min_bitpool = v.sbc_min_bitpool;
            c.sbcxq_min_bitpool = v.sbcxq_min_bitpool;
            c.sbc_blocks = v.sbc_blocks;
            c.sbc_subbands = v.sbc_subbands;
            c.sbc_allocation = v.sbc_allocation;
            c.sbc_channel_mode = v.sbc_channel_mode;
            c.unlisted_options = v.unlisted_options;
            b.set_codec_choice(c);
            fit_to_codec(&b)
        }
        None => carry_to_codec(&base, codec),
    };
    card_codec(&next)
}

/// The output device's name inside "Windows default (...)" (issue #86).
/// Windows' friendly name already ends in "(adapter)", so not that. A
/// virtual device goes by its adapter ("VB-Audio Hi-Fi Cable": its own
/// "Hi-Fi Cable Input" reads as a microphone) unless another output has the
/// same adapter (CABLE Input and CABLE In 16ch) or it holds parentheses; any
/// other device by its own name ("Speakers": display audio and Realtek
/// adapters say nothing of which device).
pub fn default_output_name(d: &AudioDevice, all: &[AudioDevice]) -> String {
    let adapter = d.adapter_name.as_deref().map(js_trim).filter(|a| !a.is_empty());
    if let Some(adapter) = adapter
        && d.is_virtual
        && !adapter.contains(['(', ')'])
        && !all.iter().any(|o| o.id != d.id && o.adapter_name.as_deref().map(js_trim) == Some(adapter))
    {
        return adapter.to_owned();
    }
    match d.short_name.as_deref().map(js_trim) {
        Some(s) if !s.is_empty() => s.to_owned(),
        _ => d.name.clone(),
    }
}

/// A device profile's name: "<device> | <codec>" (v2's own, as the setup
/// makes it; "<device> LE Audio" for LE Audio) while it has that form, the
/// user's own otherwise. A name another profile has gets " (2)", " (3)"...
pub fn followed_name<S: AsRef<str>>(p: &Profile, codec_label: &str, taken: &[S]) -> String {
    let base = if p.device_name.is_empty() { &p.device_address } else { &p.device_name };
    let own = !p.name.is_empty()
        && p.name != *base
        && !p.name.starts_with(&format!("{base} | "))
        && p.name != format!("{base} LE Audio");
    if own {
        return p.name.clone();
    }
    let is_taken = |n: &str| taken.iter().any(|t| t.as_ref() == n);
    let name = default_profile_name(&p.device_name, &p.device_address, p.le_audio, codec_label);
    if !is_taken(&name) {
        return name;
    }
    (2..).map(|n| format!("{name} ({n})")).find(|n| !is_taken(n)).expect("a free number")
}

/// A problem with a field: translation key and its arguments
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub key: &'static str,
    pub args: Vec<Arg>,
}

impl Problem {
    fn new(key: &'static str) -> Problem {
        Problem { key, args: Vec::new() }
    }
}

/// The problems of the fields that have one
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProfileProblems {
    pub name: Option<Problem>,
    pub device: Option<Problem>,
    pub packet: Option<Problem>,
    pub audio_device: Option<Problem>,
    pub app: Option<Problem>,
}

impl ProfileProblems {
    /// No problem: the profile can be saved
    pub fn is_empty(&self) -> bool {
        *self == ProfileProblems::default()
    }
}

/// `taken`: the names of the other profiles (a rename must not overwrite
/// one of them)
pub fn validate_profile<S: AsRef<str>>(p: &Profile, taken: &[S]) -> ProfileProblems {
    let mut out = ProfileProblems::default();
    let name = js_trim(&p.name);
    if name.is_empty() {
        out.name = Some(Problem::new("ui.profile.err_name_empty"));
    } else if taken.iter().any(|t| t.as_ref() == name) {
        out.name = Some(Problem::new("ui.profile.err_name_taken"));
    }
    if js_trim(&p.device_address).is_empty() {
        out.device = Some(Problem::new("ui.profile.err_device"));
    } else if parse_address(&p.device_address).is_none() {
        out.device = Some(Problem::new("error.invalid_address_format"));
    }
    if p.max_media_payload < PACKET_MIN || p.max_media_payload > PACKET_MAX {
        out.packet = Some(Problem { key: "ui.profile.err_packet", args: vec![PACKET_MIN.into(), PACKET_MAX.into()] });
    }
    if p.capture_mode == "virtual" && p.audio_device_id.is_empty() {
        out.audio_device = Some(Problem::new("error.no_virtual_device_selected"));
    }
    if p.capture_mode == "app" && p.app_exe.is_empty() {
        out.app = Some(Problem::new("error.no_app_selected"));
    }
    // LE Audio streams loopback and virtual device capture only (the core
    // refuses the stream otherwise)
    if p.le_audio && p.capture_mode == "app" {
        out.app = Some(Problem::new("error.le_audio_capture"));
    }
    out
}

/// The profile as it is saved: trimmed name, normalised address
pub fn normalize_profile(p: &Profile) -> Profile {
    Profile {
        name: js_trim(&p.name).to_owned(),
        device_address: parse_address(&p.device_address).unwrap_or_else(|| js_trim(&p.device_address).to_owned()),
        ..p.clone()
    }
}

/// The classic entry of the device at `address` (as typed)
fn classic_device<'a>(devices: &'a [Device], address: &str) -> Option<&'a Device> {
    let a = parse_address(address)?;
    devices.iter().find(|d| !d.le_audio && d.address.to_uppercase() == a)
}

/// The codec keys a device offered when it last connected (empty: not known)
pub fn known_codecs<'a>(devices: &'a [Device], address: &str) -> &'a [String] {
    classic_device(devices, address).map(|d| d.codecs.as_slice()).unwrap_or_default()
}

/// The codec keys a device may offer only once chosen (issue #30)
pub fn codecs_on_request<'a>(devices: &'a [Device], address: &str) -> &'a [String] {
    classic_device(devices, address).map(|d| d.codecs_on_request.as_slice()).unwrap_or_default()
}

/// What each codec a device offered lists (rates, bit depths, largest bit
/// rate: Device.codec_caps; empty while not known, issue #86)
pub fn known_codec_caps<'a>(devices: &'a [Device], address: &str) -> &'a [CodecCaps] {
    classic_device(devices, address).map(|d| d.codec_caps.as_slice()).unwrap_or_default()
}

/// A device entry's identity: a dual-mode device has a classic entry and an
/// LE Audio one with the same address
pub fn device_key(address: &str, le_audio: bool) -> String {
    format!("{}{}", address.to_uppercase(), if le_audio { "/le" } else { "" })
}

/// VB-Cable's playback side ("CABLE Input", "CABLE In 16ch"; as the core's
/// is_vb_cable)
pub fn is_vb_cable(d: &AudioDevice) -> bool {
    d.is_virtual && d.name.to_lowercase().contains("cable") && !is_hifi_cable(d)
}

/// VB-Cable's playback device among Windows' outputs
pub fn vb_cable<'a>(devices: impl IntoIterator<Item = &'a AudioDevice>) -> Option<&'a AudioDevice> {
    let cables: Vec<&AudioDevice> = devices.into_iter().filter(|d| is_vb_cable(d)).collect();
    // The standard one before the 16-channel one
    cables.iter().find(|d| !multichannel_cable(d)).or(cables.first()).copied()
}

/// VB-Audio's Hi-Fi Cable ("Hi-Fi Cable Input"): the cable for the sound,
/// with less delay than VB-Cable (as the core's is_hifi_cable)
pub fn is_hifi_cable(d: &AudioDevice) -> bool {
    // hi-?fi[ -]cable, any case
    let n = d.name.to_lowercase();
    n.match_indices("hi").any(|(i, _)| {
        let rest = &n[i + 2..];
        let rest = rest.strip_prefix('-').unwrap_or(rest);
        rest.strip_prefix("fi")
            .and_then(|r| r.strip_prefix(' ').or_else(|| r.strip_prefix('-')))
            .is_some_and(|r| r.starts_with("cable"))
    })
}

/// The Hi-Fi Cable's playback device among Windows' outputs
pub fn hifi_cable(devices: &[AudioDevice]) -> Option<&AudioDevice> {
    devices.iter().find(|d| is_hifi_cable(d))
}

/// Windows' default output as a device's System Loopback takes it: none
/// while the stream made its own Output Source the default (issue #86: one
/// device's Output Source took cables from another device's lists; it goes
/// back when that stream ends). As the core's default_output_for_cables.
pub fn own_default_output(devices: &[AudioDevice], snap: ProfilesView<'_>) -> String {
    let id = devices.iter().find(|d| d.is_default).map(|d| d.id.as_str()).unwrap_or_default();
    let running =
        matches!(snap.stream.state, StreamState::Connecting | StreamState::Streaming | StreamState::Reconnecting);
    let streaming = if running {
        snap.profiles.iter().find(|p| Some(p.name.as_str()) == snap.stream.profile.as_deref())
    } else {
        None
    };
    let switches = streaming.is_some_and(|p| p.capture_mode == "virtual" || p.capture_mode == "output");
    if !id.is_empty() && switches && streaming.is_some_and(|p| p.audio_device_id == id) {
        String::new()
    } else {
        id.to_owned()
    }
}

/// The real output the sound is captured from beside the one cable, which
/// is the microphone's (issue #86): a monitor's or TV's audio (nothing plays
/// aloud beside the headphones on most; of several, the one that is Windows'
/// default output), else Windows' default output, else the first, virtual
/// devices aside. As the core's real_output.
pub fn real_output(devices: &[AudioDevice]) -> Option<&AudioDevice> {
    let real: Vec<&AudioDevice> = devices.iter().filter(|d| !d.is_virtual).collect();
    let display: Vec<&AudioDevice> = real.iter().copied().filter(|d| d.is_display).collect();
    display
        .iter()
        .find(|d| d.is_default)
        .or(display.first())
        .or_else(|| real.iter().find(|d| d.is_default))
        .or(real.first())
        .copied()
}

/// The sound card the microphone plays into where there is no cable: an
/// output that is neither a display's nor virtual, Windows' default output
/// before the first. As the core's sound_card.
pub fn sound_card(devices: &[AudioDevice]) -> Option<&AudioDevice> {
    let cards: Vec<&AudioDevice> = devices.iter().filter(|d| !d.is_virtual && !d.is_display).collect();
    cards.iter().find(|d| d.is_default).or(cards.first()).copied()
}

/// What a new device gets (issue #86): neither left empty where a cable
/// allows, and the same whatever Windows' default output is (the source is
/// made the default while the device is connected). Both cables: the Hi-Fi
/// Cable for the sound, VB-Cable for the microphone. One: the microphone,
/// and the sound a real output ([`real_output`]), or the cable itself with no
/// microphone where there is none. None: a monitor's or TV's audio for the
/// sound and the sound card for the microphone (turned off: Auto cannot tell
/// who listens to a sound card) where Windows has both. As the core's
/// Cables::sound_for and Bare.
/// `sound`: a cable captured as a virtual device, a real output as an
/// Output Device; None: System Loopback / no microphone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetupAudio<'a> {
    pub sound: Option<&'a AudioDevice>,
    pub mic: Option<&'a AudioDevice>,
}

pub fn setup_audio(devices: &[AudioDevice]) -> SetupAudio<'_> {
    let hifi = hifi_cable(devices);
    let vb = vb_cable(devices.iter().filter(|d| !multichannel_cable(d)));
    if let (Some(hifi), Some(vb)) = (hifi, vb) {
        return SetupAudio { sound: Some(hifi), mic: Some(vb) };
    }
    let cable = hifi.or(vb);
    let real = real_output(devices);
    let Some(cable) = cable else {
        let card = sound_card(devices);
        return match (real, card) {
            (Some(real), Some(card)) if real.is_display => SetupAudio { sound: Some(real), mic: Some(card) },
            _ => SetupAudio { sound: None, mic: None },
        };
    };
    match real {
        Some(real) => SetupAudio { sound: Some(real), mic: Some(cable) },
        None => SetupAudio { sound: Some(cable), mic: None },
    }
}

/// A 16-channel cable ("CABLE In 16ch"): no use as a source or a
/// microphone for most people (issue #86), listed in developer mode
pub fn multichannel_cable(d: &AudioDevice) -> bool {
    // 16\s*ch, any case
    let n = d.name.to_lowercase();
    n.match_indices("16").any(|(i, _)| n[i + 2..].trim_start_matches(js_space).starts_with("ch"))
}

/// The entries to offer (issue #86: one device, not an "Audio" and an "LE
/// Audio" one). LE Audio off: the classic entries. On: a device with both
/// is its LE Audio entry alone, saved when either is, under the classic
/// name when it has none; "Use LE Audio when available" decides which of
/// the two connects. Hearing aids (LE entries of their own) only with
/// `aids`, with or without LE Audio.
pub fn listed_devices(devices: &[Device], le_audio: bool, aids: bool) -> Vec<Device> {
    if !le_audio {
        return devices.iter().filter(|d| !d.le_audio || (aids && d.hearing_aid)).cloned().collect();
    }
    let mut out = Vec::new();
    for d in devices {
        let other = twin_of(devices, &d.address, d.le_audio);
        match other {
            Some(_) if !d.le_audio => continue,
            Some(o) => out.push(Device {
                name: if d.name.is_empty() { o.name.clone() } else { d.name.clone() },
                saved: d.saved || o.saved,
                audio: true,
                rssi: d.rssi.or(o.rssi),
                ..d.clone()
            }),
            None => out.push(d.clone()),
        }
    }
    if !aids {
        out.retain(|d| !(d.le_audio && d.hearing_aid));
    }
    out
}

/// The device's entry of the other kind (classic / LE Audio), same address
pub fn twin_of<'a>(devices: &'a [Device], address: &str, le_audio: bool) -> Option<&'a Device> {
    let a = address.to_uppercase();
    devices.iter().find(|x| x.le_audio != le_audio && x.address.to_uppercase() == a)
}

/// A profile's name until the user types one: the device, "<device> LE Audio" for LE Audio (v1).
/// `codec`: the codec's name, "" for none
pub fn default_profile_name(device_name: &str, address: &str, le_audio: bool, codec: &str) -> String {
    let base = if device_name.is_empty() { address } else { device_name };
    if le_audio {
        return if base.is_empty() { String::new() } else { format!("{base} LE Audio") };
    }
    if !base.is_empty() && !codec.is_empty() { format!("{base} | {codec}") } else { base.to_owned() }
}

/// The render device a capture mode records: the default output device
/// (loopback), the chosen virtual or output device, or none (application
/// capture: Windows converts to the rate asked for)
pub fn capture_device<'a>(
    capture_mode: &str,
    audio_device_id: &str,
    devices: &'a [AudioDevice],
) -> Option<&'a AudioDevice> {
    match capture_mode {
        "loopback" => devices.iter().find(|d| d.is_default),
        "virtual" | "output" => devices.iter().find(|d| d.id == audio_device_id),
        _ => None,
    }
}

/// Which of the profile's sample rate and bit depth differ from the capture
/// device's format
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatMismatch {
    pub rate: bool,
    pub bits: bool,
}

/// Which of the profile's sample rate and bit depth differ from the capture
/// device's format (v1 update_format_info; Auto never differs); `mix_rate`,
/// `mix_bits`: AudioDevice's
pub fn format_mismatch(
    sample_rate: u32,
    bit_depth: u32,
    mix_rate: Option<u32>,
    mix_bits: Option<u16>,
) -> FormatMismatch {
    FormatMismatch {
        rate: sample_rate != 0 && mix_rate.is_some_and(|r| r != sample_rate),
        bits: bit_depth != 0 && mix_bits.is_some_and(|b| u32::from(b) != bit_depth),
    }
}

/// A device name the core saves: not empty, on one line, without "|"
pub fn valid_device_name(name: &str) -> bool {
    let n = js_trim(name);
    !n.is_empty() && !n.contains(['\r', '\n', '|'])
}

/// Whether this Windows can capture one application (build 19041 or later)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppCaptureSupport {
    Yes,
    No,
    Unknown,
}

/// From User-Agent Client Hints: on Windows the major of platformVersion is
/// the UniversalApiContract version, 10 from Windows 10 version 2004 (build
/// 19041) and 13+ on Windows 11; 1-8 are the earlier Windows 10 versions.
/// Unknown elsewhere or without the hints (the core still refuses the
/// stream with error.app_capture_unsupported).
pub fn app_capture_support_from(platform: Option<&str>, platform_version: Option<&str>) -> AppCaptureSupport {
    let Some(version) = platform_version.filter(|v| platform == Some("Windows") && !v.is_empty()) else {
        return AppCaptureSupport::Unknown;
    };
    // Number.parseInt: optional sign, then the leading digits
    let v = version.trim_start_matches(js_space);
    let (neg, digits) = match v.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, v.strip_prefix('+').unwrap_or(v)),
    };
    let digits: String = digits.chars().take_while(char::is_ascii_digit).collect();
    let Ok(major) = digits.parse::<i64>() else { return AppCaptureSupport::Unknown };
    let major = if neg { -major } else { major };
    if major >= 10 { AppCaptureSupport::Yes } else { AppCaptureSupport::No }
}

/// stream.start_custom for the core's test tone (left 1 kHz, right
/// 1.5 kHz, as v1's) to a profile's headphones, with its sound settings
pub fn tone_params(p: &Profile) -> Method {
    Method::StreamStartCustom {
        device: p.device_address.clone(),
        device_name: (!p.device_name.is_empty()).then(|| p.device_name.clone()),
        codec: p.codec.clone(),
        quality: p.quality.clone(),
        sample_rate: (p.sample_rate != 0).then_some(p.sample_rate),
        bit_depth: (p.bit_depth != 0).then_some(p.bit_depth),
        source: "tone".into(),
        source_id: None,
        source_name: None,
        max_media_payload: (p.max_media_payload != 0).then_some(p.max_media_payload),
        abr: Some(p.abr),
        lhdcv5_low_latency: Some(p.lhdcv5_low_latency),
        // As a profile: a codec the device does not offer falls back (issue #86)
        codec_fallback: Some(true),
    }
}

#[cfg(test)]
#[path = "tests/profile.rs"]
mod tests;

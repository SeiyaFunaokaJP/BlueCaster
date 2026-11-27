//! The rules behind the Audio Devices screen and the device components (the
//! device picker and settings, the input source, the capture picker, the
//! pairing dialog, importing pairings, the device switcher): which devices
//! are listed, the choices of the capture source and the microphone, the
//! warnings and the keys shown. The window (app/src/ui) draws what these
//! return.

use ipc::{
    AudioApp, AudioDevice, Device, DevicePairing, DriverKind, Profile, Snapshot, StreamState, WindowsImport,
    WindowsPairedDevice,
};

use crate::codecs::codec_spec;
use crate::format::js_trim;
use crate::i18n::t;
use crate::profile::{
    AudioEntry, DeviceFields, HeadsetMode, ProfilesView, audio_entries, capture_device, default_output_name,
    device_fields, followed_name, format_mismatch, headset_mode, headset_modes, hifi_cable, is_hifi_cable, le_audio_on,
    listed_devices, multichannel_cable, new_profile, normalize_profile, own_default_output, vb_cable,
};
use crate::t;

// ---- DevicePicker -----------------------------------------------------------

/// How long a scan looks for devices (issue #86: 10 s found too few)
pub const SCAN_SECONDS: u32 = 30;
/// The first-run setup looks until a device is added (issue #86): scans of
/// the core's longest, one after another
pub const SETUP_SCAN_SECONDS: u32 = 600;

pub fn sort_devices(devices: &[Device]) -> Vec<Device> {
    // Saved first, then audio devices by signal; other kinds (keyboards...) last
    let mut out = devices.to_vec();
    out.sort_by(|a, b| {
        b.audio.cmp(&a.audio).then(b.saved.cmp(&a.saved)).then(b.rssi.unwrap_or(-127).cmp(&a.rssi.unwrap_or(-127)))
    });
    out
}

/// A name not yet taken by a profile: `base`, else "base (2)"...
pub fn free_name<S: AsRef<str>>(taken: &[S], base: &str) -> String {
    let is_taken = |n: &str| taken.iter().any(|t| t.as_ref() == n);
    if !is_taken(base) {
        return base.to_owned();
    }
    (2..).map(|n| format!("{base} ({n})")).find(|n| !is_taken(n)).expect("a free number")
}

/// The name a device found by a scan or typed is saved under: the name
/// given, else the one it is saved with (a saved device typed by its
/// address keeps its name), else its other entry's, else its address
pub fn saved_name(devices: &[Device], address: &str, name: &str, le_audio: bool) -> String {
    let a = address.to_uppercase();
    let saved_as = devices.iter().find(|d| d.saved && d.le_audio == le_audio && d.address.to_uppercase() == a);
    let twin = crate::profile::twin_of(devices, address, le_audio);
    [Some(js_trim(name)), saved_as.map(|d| d.name.as_str()), twin.map(|d| d.name.as_str())]
        .into_iter()
        .flatten()
        .find(|n| !n.is_empty())
        .unwrap_or(address)
        .to_owned()
}

/// The entry `le_audio` of the device at `address` is saved with `name`
/// already (no device.rename needed)
pub fn saved_with(devices: &[Device], address: &str, le_audio: bool, name: &str) -> bool {
    let a = address.to_uppercase();
    devices.iter().any(|d| d.saved && d.le_audio == le_audio && d.address.to_uppercase() == a && d.name == name)
}

/// LE Audio entries with the setting on and an adapter known to stream it
pub fn le_listed(snap: &Snapshot, classic_only: bool) -> bool {
    le_audio_on(&snap.settings) && snap.adapter.le_audio == Some(true) && !classic_only
}

/// The devices a picker lists: the saved ones only for the profile editor
/// (`saved_only`), the unsaved ones where the saved stand in a list of their
/// own (`paired`), else all
pub fn picker_devices(snap: &Snapshot, classic_only: bool, saved_only: bool, paired: bool) -> Vec<Device> {
    // A hearing aid needs Bluetooth LE, not an adapter that streams LE Audio
    let le = le_audio_on(&snap.settings) && !classic_only;
    let le_audio = le_listed(snap, classic_only);
    let all: Vec<Device> = sort_devices(&listed_devices(&snap.devices, le))
        .into_iter()
        .filter(|d| !d.le_audio || le_audio || d.hearing_aid)
        .collect();
    if saved_only {
        return all.into_iter().filter(|d| d.saved).collect();
    }
    if paired { all.into_iter().filter(|d| !d.saved).collect() } else { all }
}

/// The Audio Devices screen's saved devices: those Home and Codecs show (LE
/// Audio entries on an adapter not known to stream it could not be removed
/// here otherwise)
pub fn managed_saved(devices: &[Device], le_audio_on: bool, classic_only: bool) -> Vec<Device> {
    sort_devices(&listed_devices(devices, le_audio_on && !classic_only)).into_iter().filter(|d| d.saved).collect()
}

/// The setup's paired devices: the classic saved ones
pub fn paired_list(devices: &[Device]) -> Vec<Device> {
    sort_devices(&listed_devices(devices, false)).into_iter().filter(|d| d.saved).collect()
}

/// A device in use: the scan asks to disconnect it first (issue #86)
pub fn in_use(state: StreamState) -> bool {
    matches!(state, StreamState::Connecting | StreamState::Reconnecting | StreamState::Streaming)
}

/// The name of the device in use, for the question before a scan
pub fn in_use_name(snap: &Snapshot) -> String {
    let profile = snap.stream.profile.as_deref().unwrap_or_default();
    snap.profiles
        .iter()
        .find(|p| p.name == profile)
        .map(|p| p.device_name.as_str())
        .filter(|n| !n.is_empty())
        .unwrap_or(profile)
        .to_owned()
}

/// The text above a picker's list: what the scan does while it runs, else
/// how many devices there are
pub fn picker_head_text(snap: &Snapshot, scan_text: Option<&str>, count: usize) -> String {
    if !snap.scanning {
        return t!("ui.devices.count", count);
    }
    match scan_text {
        Some(text) => text.to_owned(),
        None => t(if snap.le_scanning { "status.le_scanning" } else { "connection.scanning_hint" }),
    }
}

/// The address and kind under the rename dialog's field
pub fn rename_hint(d: &Device) -> String {
    if d.le_audio { format!("{} ({})", d.address, t(le_tag(d))) } else { d.address.clone() }
}

/// The kind of an LE entry: a hearing aid, or LE Audio
pub fn le_tag(d: &Device) -> &'static str {
    if d.hearing_aid { "device.tag_hearing_aid" } else { "device.tag_le_audio" }
}

/// A row's name: the device's, else "Unknown device"
pub fn device_label(d: &Device) -> String {
    if d.name.is_empty() { t("device.name_unknown") } else { d.name.clone() }
}

/// The tag of a row's kind: LE Audio (a set or not), else Audio for an
/// audio device, else none
pub fn kind_tag(d: &Device) -> Option<String> {
    if d.le_audio && d.hearing_aid {
        Some(t("device.tag_hearing_aid"))
    } else if d.le_audio {
        Some(t(if d.le_audio_set { "device.tag_le_audio_set" } else { "device.tag_le_audio" }))
    } else if d.audio {
        Some(t("device.tag_audio"))
    } else {
        None
    }
}

/// The WinUSB adapter with a known address: the one Windows' pairings are
/// asked about
pub fn winusb_adapter(snap: &Snapshot) -> Option<String> {
    snap.adapter
        .adapters
        .iter()
        .find(|a| a.driver == DriverKind::Winusb && a.address.as_deref().is_some_and(|a| !a.is_empty()))
        .and_then(|a| a.address.clone())
}

/// `address` is among the addresses Windows' Bluetooth paired
pub fn windows_paired<S: AsRef<str>>(paired: &[S], address: &str) -> bool {
    paired.iter().any(|a| a.as_ref().eq_ignore_ascii_case(address))
}

// ---- ImportPairings -----------------------------------------------------------

/// What Windows paired through the adapter that BlueCaster has not saved
pub fn unsaved_windows(windows: &[WindowsPairedDevice], devices: &[Device]) -> Vec<WindowsPairedDevice> {
    let saved: Vec<String> = devices.iter().filter(|d| d.saved).map(|d| d.address.to_uppercase()).collect();
    windows.iter().filter(|d| !saved.contains(&d.address.to_uppercase())).cloned().collect()
}

/// The notice after an import
pub fn import_notice(r: &WindowsImport) -> String {
    if !r.adapter_found {
        return t("ui.adapter.import_none");
    }
    let kept =
        if r.kept.is_empty() { String::new() } else { format!(" {}", t!("ui.adapter.import_kept", r.kept.len())) };
    format!("{}{kept}", t!("ui.adapter.import_done", r.imported.len()))
}

// ---- DeviceSwitcher -----------------------------------------------------------

/// The flyout's choices: one entry per audio device with a profile (its
/// profile's name, the device's name)
pub fn switcher_choices(snap: &Snapshot) -> Vec<(String, String)> {
    audio_entries(snap)
        .into_iter()
        .filter_map(|e| {
            let p = e.profile?;
            let name = if e.name.is_empty() { e.address.clone() } else { e.name.clone() };
            let label = if e.le_audio { format!("{name} ({})", t(le_tag(&e.device))) } else { name };
            Some((p.name, label))
        })
        .collect()
}

// ---- CapturePicker ------------------------------------------------------------

/// One capture source of the list
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureChoice {
    pub value: String,
    pub label: String,
}

impl CaptureChoice {
    fn new(value: impl Into<String>, label: impl Into<String>) -> CaptureChoice {
        CaptureChoice { value: value.into(), label: label.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CaptureChoices {
    pub virtual_devices: Vec<CaptureChoice>,
    pub output: Vec<CaptureChoice>,
    pub apps: Vec<CaptureChoice>,
}

/// The value of the profile's capture source among the choices
pub fn capture_value(p: &Profile) -> String {
    match p.capture_mode.as_str() {
        "virtual" => format!("virtual:{}", p.audio_device_id),
        "output" => format!("output:{}", p.audio_device_id),
        "app" => format!("app:{}", p.app_exe.to_lowercase()),
        _ => "loopback".into(),
    }
}

/// The choices: virtual devices, every output device, and playing apps, the
/// profile's own also when it is not there now. No apps for LE Audio
/// (error.le_audio_capture) or on a Windows that cannot capture one app.
/// Neither the 16-channel cable nor the microphone's cable (`mic_id`, it
/// would be mixed into the sound) unless chosen or `all` (Developer > Show
/// every audio device, issue #86).
pub fn capture_choices(
    p: &Profile,
    devices: &[AudioDevice],
    apps: &[AudioApp],
    apps_ok: bool,
    all: bool,
    mic_id: &str,
) -> CaptureChoices {
    let own = capture_value(p);
    let offered = |d: &AudioDevice| {
        all || (!multichannel_cable(d) && !(!mic_id.is_empty() && d.id == mic_id))
            || own == format!("virtual:{}", d.id)
            || own == format!("output:{}", d.id)
    };
    let missing = || {
        t!(
            "capture.volume_sync_missing",
            if p.audio_device_name.is_empty() { &p.audio_device_id } else { &p.audio_device_name }
        )
    };
    let mut virtual_devices: Vec<CaptureChoice> = devices
        .iter()
        .filter(|d| d.is_virtual && offered(d))
        .map(|d| CaptureChoice::new(format!("virtual:{}", d.id), d.name.clone()))
        .collect();
    if p.capture_mode == "virtual" && !p.audio_device_id.is_empty() && !virtual_devices.iter().any(|c| c.value == own) {
        virtual_devices.insert(0, CaptureChoice::new(own.clone(), missing()));
    }
    // The other outputs: each device is listed once, virtual ones under
    // Virtual Device
    let mut output: Vec<CaptureChoice> = devices
        .iter()
        .filter(|d| !d.is_virtual && offered(d))
        .map(|d| CaptureChoice::new(format!("output:{}", d.id), d.name.clone()))
        .collect();
    if p.capture_mode == "output" && !p.audio_device_id.is_empty() && !output.iter().any(|c| c.value == own) {
        output.insert(0, CaptureChoice::new(own.clone(), missing()));
    }
    let mut list: Vec<CaptureChoice> = Vec::new();
    if !p.le_audio && apps_ok {
        for a in apps {
            let value = format!("app:{}", a.exe.to_lowercase());
            if !list.iter().any(|c| c.value == value) {
                list.push(CaptureChoice::new(value, a.name.clone()));
            }
        }
    }
    if p.capture_mode == "app" && !p.app_exe.is_empty() && !list.iter().any(|c| c.value == own) {
        let name = if p.app_name.is_empty() { &p.app_exe } else { &p.app_name };
        list.insert(0, CaptureChoice::new(own.clone(), t!("capture.app_not_playing", name)));
    }
    CaptureChoices { virtual_devices, output, apps: list }
}

/// The profile with the chosen capture source
pub fn with_capture(p: &Profile, value: &str, devices: &[AudioDevice], apps: &[AudioApp]) -> Profile {
    let mut out = p.clone();
    let device = |id: &str, mode: &str, out: &mut Profile| {
        let d = devices.iter().find(|x| x.id == id);
        out.capture_mode = mode.into();
        out.audio_device_id = id.into();
        if let Some(d) = d {
            out.audio_device_name = d.name.clone();
        }
    };
    if let Some(id) = value.strip_prefix("virtual:") {
        device(id, "virtual", &mut out);
    } else if let Some(id) = value.strip_prefix("output:") {
        device(id, "output", &mut out);
    } else if let Some(exe) = value.strip_prefix("app:") {
        out.capture_mode = "app".into();
        if p.app_exe.to_lowercase() != exe {
            let a = apps.iter().find(|x| x.exe.to_lowercase() == exe);
            out.app_exe = a.map(|a| a.exe.clone()).unwrap_or_else(|| exe.to_owned());
            out.app_name = a.map(|a| a.name.clone()).unwrap_or_else(|| exe.to_owned());
        }
    } else {
        out.capture_mode = "loopback".into();
    }
    out
}

// ---- DeviceSettings -----------------------------------------------------------

/// One entry of a list with groups (the capture source, the microphone's
/// output)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListOption {
    pub value: String,
    pub label: String,
    pub group: Option<String>,
    pub disabled: bool,
}

impl ListOption {
    fn new(value: impl Into<String>, label: impl Into<String>) -> ListOption {
        ListOption { value: value.into(), label: label.into(), group: None, disabled: false }
    }

    fn in_group(mut self, group: &str) -> ListOption {
        self.group = Some(group.to_owned());
        self
    }

    fn disabled(mut self) -> ListOption {
        self.disabled = true;
        self
    }
}

/// The profile the shown settings start from (a device without one: the defaults)
pub fn newest_base(list: &[&Profile]) -> Profile {
    list.first().map(|p| (*p).clone()).unwrap_or_else(new_profile)
}

/// The device's settings are LE Audio only: an LE Audio entry, or a device
/// whose profiles are all LE Audio
pub fn le_only(device: &Device, profiles: &[&Profile]) -> bool {
    device.le_audio || (!profiles.is_empty() && profiles.iter().all(|p| p.le_audio))
}

/// What Windows' lists hold, as far as read: the capture source's choices
/// depend on them
#[derive(Debug, Clone, Copy)]
pub struct AudioLists<'a> {
    /// Windows' outputs (None: not read yet)
    pub devices: Option<&'a [AudioDevice]>,
    /// The outputs were read (not only failed to be): the "no virtual device"
    /// line relies on it
    pub listed: bool,
    /// The playing apps (None: not read yet)
    pub apps: Option<&'a [AudioApp]>,
    /// This Windows may capture one app (not known to be too old)
    pub apps_ok: bool,
}

/// The sound devices (the Windows default first), the virtual devices and
/// the playing apps in one list (issue #86: no "Loopback"). `mic_id`: the
/// device the microphone plays into, not offered.
pub fn device_sources(p: &Profile, lists: AudioLists<'_>, all: bool, mic_id: &str) -> Vec<ListOption> {
    let devices = lists.devices.unwrap_or_default();
    let c = capture_choices(p, devices, lists.apps.unwrap_or_default(), lists.apps_ok, all, mic_id);
    let virtual_group = t("capture.virtual");
    let mut virtual_devices: Vec<ListOption> =
        c.virtual_devices.into_iter().map(|o| ListOption::new(o.value, o.label).in_group(&virtual_group)).collect();
    // VB-Cable is suggested only when Windows has no capture source of that
    // kind at all: no virtual device (issue #86)
    let no_virtual = lists.listed && !devices.iter().any(|d| d.is_virtual);
    if virtual_devices.is_empty() && no_virtual {
        virtual_devices
            .push(ListOption::new("virtual:", t("capture.no_virtual_device")).in_group(&virtual_group).disabled());
    }
    let sound_group = t("capture.sound_devices");
    let outputs = c.output.into_iter().map(|o| ListOption::new(o.value, o.label).in_group(&sound_group));
    let app_group = t("capture.app");
    let mut apps: Vec<ListOption> =
        c.apps.into_iter().map(|o| ListOption::new(o.value, o.label).in_group(&app_group)).collect();
    if apps.is_empty() && lists.apps.is_some() && !p.le_audio && lists.apps_ok {
        apps.push(ListOption::new("app:", t("capture.no_app_playing")).in_group(&app_group).disabled());
    }
    // Whatever Windows' default output is, by its present name, first and in
    // no group: it follows the default, which may be a virtual device too
    // (issue #86: under Sound Devices it read as a repeated entry)
    let output =
        devices.iter().find(|d| d.is_default).map(|d| default_output_name(d, devices)).filter(|n| !n.is_empty());
    let loopback = ListOption::new(
        "loopback",
        match output {
            Some(o) => t!("capture.loopback_default", o),
            None => t("capture.loopback_default_none"),
        },
    );
    let mut out = vec![loopback];
    out.extend(outputs);
    out.extend(virtual_devices);
    out.extend(apps);
    out
}

/// What the capture source does, below it
pub fn capture_hint(mode: &str) -> String {
    match mode {
        "virtual" => t("capture.hint_virtual"),
        "output" => t("capture.hint_output"),
        "app" => format!("{}\n{}", t("capture.hint_app"), t("capture.app_hint")),
        _ => t("capture.hint_loopback"),
    }
}

/// The device the microphone plays into, while it is on (not muted): not
/// offered as the source (issue #86)
pub fn mic_device(entry: Option<&AudioEntry>) -> String {
    match entry.and_then(|e| e.profile.as_ref()) {
        Some(p) if !p.mic_muted && headset_mode(&p.headset_mic, true) != HeadsetMode::Off => p.mic_device_id.clone(),
        _ => String::new(),
    }
}

/// The first profile of a device without one, with the settings chosen
/// (Auto, as the Codecs screen would make it)
pub fn first_device_profile(device: &Device, next: &DeviceFields, profiles: &[Profile]) -> Profile {
    let mut base = crate::profile::device_base(device, profiles);
    next.apply_to(&mut base);
    let codec = if device.le_audio { String::new() } else { t(codec_spec("auto").label) };
    let taken: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
    base.name = followed_name(&base, &codec, &taken);
    normalize_profile(&base)
}

/// The device's profiles with the settings chosen, those that change: an LE
/// Audio profile cannot capture one application, so it keeps its own ->
/// (the profile, the name it is saved under)
pub fn device_profile_updates(profiles: &[&Profile], next: &DeviceFields) -> Vec<(Profile, String)> {
    profiles
        .iter()
        .filter(|p| !(p.le_audio && next.capture_mode == "app"))
        .filter_map(|p| {
            let mut updated = (*p).clone();
            next.apply_to(&mut updated);
            let updated = normalize_profile(&updated);
            (device_fields(&updated) != device_fields(p)).then(|| (updated, p.name.clone()))
        })
        .collect()
}

/// The rate and bit depth a profile streams with (0: Auto): what streams
/// now while the device is connected, else its own values; Auto-select
/// streams Auto's (unless developer mode keeps each codec's, issue #86)
pub fn streamed_format(p: &Profile, snap: &Snapshot) -> (u32, u32) {
    let streaming = snap.stream.state == StreamState::Streaming && snap.stream.profile.as_deref() == Some(&p.name);
    let info = if streaming { snap.stream.info.as_ref() } else { None };
    let s = &snap.settings;
    let own = !(s.auto_codec && !p.le_audio) || (s.developer_mode && s.dev_overrides.auto_codec_settings);
    let (rate, bits) = if own { (p.sample_rate, p.bit_depth) } else { (0, 0) };
    match info {
        Some(i) if i.le.is_none() => (i.sample_rate, bits),
        _ => (rate, bits),
    }
}

/// The format Windows mixes the captured device in, and the first profile
/// of the device that streams another one
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureFormat {
    pub rate: u32,
    pub bits: u16,
    /// That profile's (sample rate, bit depth), 0 for Auto
    pub off: Option<(u32, u32)>,
}

/// The format Windows mixes the captured device in (None while unknown),
/// and what a profile of the device streams with otherwise
pub fn capture_format(
    fields: &DeviceFields,
    profiles: &[&Profile],
    devices: &[AudioDevice],
    snap: &Snapshot,
) -> Option<CaptureFormat> {
    let d = capture_device(&fields.capture_mode, &fields.audio_device_id, devices)?;
    let (rate, bits) = (d.mix_rate?, d.mix_bits?);
    let off = profiles.iter().map(|p| streamed_format(p, snap)).find(|&(r, b)| {
        let m = format_mismatch(r, b, d.mix_rate, d.mix_bits);
        m.rate || m.bits
    });
    Some(CaptureFormat { rate, bits, off })
}

/// "24-bit" (the translated 16 and 24-bit)
pub fn bits_text(bits: u32) -> String {
    if bits == 16 || bits == 24 { t(&format!("bitdepth.{bits}")) } else { format!("{bits}-bit") }
}

/// "48000 Hz / 24-bit"
pub fn format_text(rate: u32, bits: u32) -> String {
    if bits != 0 { format!("{rate} Hz / {}", bits_text(bits)) } else { format!("{rate} Hz") }
}

/// Why the format box is amber: it still streams, converted (issue #86),
/// with both formats. It speaks of the codec, never of the profile by its
/// name.
pub fn format_warning(c: &CaptureFormat) -> Option<String> {
    let (rate, bits) = c.off?;
    let rate = if rate != 0 { rate } else { c.rate };
    Some(t!("ui.profile.format_differs", format_text(rate, bits), format_text(c.rate, u32::from(c.bits))))
}

// ---- InputSource --------------------------------------------------------------

/// The render endpoint the stream records with this source (the default
/// output for System Loopback): the microphone must not go there
pub fn source_device(capture_mode: &str, audio_device_id: &str, default_output: &str) -> String {
    match capture_mode {
        "virtual" | "output" => audio_device_id.to_owned(),
        "loopback" => default_output.to_owned(),
        _ => String::new(),
    }
}

/// The device's output source as shown (a loop is refused)
#[derive(Debug, Clone, Copy)]
pub struct OutputSource<'a> {
    pub capture_mode: &'a str,
    pub audio_device_id: &'a str,
    /// Connecting makes the source Windows' default output (the device's
    /// "Set as Windows default" in force for a switchable source)
    pub switches: bool,
}

/// What a change of the microphone's mode saves
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicPatch {
    /// mic_muted: the mode stays, the microphone's button opens it again
    Mute,
    /// headset_mic, and mic_muted off
    Mode(HeadsetMode),
}

/// Off mutes a device whose mode is Auto or On, as its button does
pub fn choose_mode(chosen: HeadsetMode, shown: HeadsetMode, saved: HeadsetMode) -> Option<MicPatch> {
    if chosen == shown {
        return None;
    }
    Some(if chosen == HeadsetMode::Off && saved != HeadsetMode::Off { MicPatch::Mute } else { MicPatch::Mode(chosen) })
}

/// The output source and what records with it (the other side of the same
/// cable): the microphone would reach the headphones or be mixed with what
/// plays, so these are listed but cannot be chosen
pub fn used_by_source(devices: &[AudioDevice], looped: &str, id: &str) -> bool {
    let Some(source) = devices.iter().find(|d| d.id == looped) else { return false };
    if id.is_empty() {
        return false;
    }
    let d = devices.iter().find(|x| x.id == id);
    id == source.id || d.is_some_and(|d| d.capture_name.is_some() && d.capture_name == source.capture_name)
}

/// The microphone's output choices. Cables first (VB-Cable, the
/// microphone's own, then the Hi-Fi Cable and the 16-channel one), then what
/// only plays the microphone; a group's options next to each other. The
/// list names each device by the side apps pick ("CABLE Output").
pub fn mic_options(devices: &[AudioDevice], shown: &Profile, looped: &str, all: bool) -> Vec<ListOption> {
    let chosen = shown.mic_device_id.as_str();
    let rank = |d: &AudioDevice| {
        (if d.capture_name.is_some() || d.is_virtual { 0 } else { 2 })
            + u8::from(multichannel_cable(d) || is_hifi_cable(d))
    };
    let mut list: Vec<&AudioDevice> = devices
        .iter()
        .filter(|d| {
            d.id == chosen
                || all
                || (d.is_virtual
                    && d.capture_name.as_deref().is_some_and(|c| !c.is_empty())
                    && !multichannel_cable(d)
                    && !used_by_source(devices, looped, &d.id))
        })
        .collect();
    list.sort_by_key(|d| rank(d));
    // Outputs sharing a recording side ("CABLE Input" and "CABLE In 16ch"
    // both record as "CABLE Output"): only the first keeps that name, the
    // others add their own (the 16-channel one is after, so the standard
    // cable keeps it)
    let label = |d: &AudioDevice| -> String {
        // A sound card by its own name: its recording side is another input
        let capture = d.capture_name.as_deref().unwrap_or_default();
        if capture.is_empty() || !d.is_virtual {
            return d.name.clone();
        }
        let first = list.iter().find(|x| x.capture_name == d.capture_name);
        if first.is_some_and(|f| f.id == d.id) {
            capture.to_owned()
        } else {
            format!("{capture} · {}", strip_adapter(&d.name))
        }
    };
    let mut out: Vec<ListOption> = list
        .iter()
        .map(|d| {
            let used = used_by_source(devices, looped, &d.id);
            let mut o =
                ListOption::new(d.id.clone(), if used { t!("capture.mic_used_by_source", label(d)) } else { label(d) });
            o.disabled = used;
            // Every output listed: those with no recording side only play the
            // microphone (issue #86: they read as microphones)
            if all {
                let recording = d.capture_name.as_deref().is_some_and(|c| !c.is_empty()) || d.is_virtual;
                o.group = Some(t(if recording { "capture.virtual" } else { "capture.mic_monitoring" }));
            }
            o
        })
        .collect();
    // One chosen before and not there now: by its name
    if !chosen.is_empty() && !list.iter().any(|d| d.id == chosen) {
        let name = if shown.mic_device_name.is_empty() { chosen } else { &shown.mic_device_name };
        out.insert(0, ListOption::new(chosen, t!("capture.volume_sync_missing", name)));
    }
    if chosen.is_empty() {
        out.insert(0, ListOption::new("", t("ui.codec.none")).disabled());
    }
    out
}

/// "CABLE In 16ch (VB-Audio Virtual Cable)" -> "CABLE In 16ch"
fn strip_adapter(name: &str) -> &str {
    match name.find('(') {
        Some(i) if name.ends_with(')') => name[..i].trim_end(),
        _ => name,
    }
}

/// What the Input Source part shows of a device
#[derive(Debug, Clone, PartialEq)]
pub struct InputSourceView {
    pub entry: AudioEntry,
    /// The profile shown: a classic device without one, its first one's defaults
    pub shown: Profile,
    /// The mode shown: Off while muted (the cards' button)
    pub mode: HeadsetMode,
    /// The mode saved: what the microphone's button opens again
    pub saved: HeadsetMode,
    pub modes: &'static [HeadsetMode],
    /// The output chosen ("" for none)
    pub chosen: String,
    pub options: Vec<ListOption>,
    /// On with no output chosen
    pub no_output: bool,
    /// On into the output source or its other side
    pub loops: bool,
    /// The Hi-Fi Cable chosen with its two sides at different sample rates:
    /// (playback, recording)
    pub hifi_rates: Option<(u32, u32)>,
    /// Windows plays its sound into the chosen device
    pub plays_default: bool,
}

/// The Input Source of the device with `key`; None when it is not a saved
/// device
pub fn input_source_view(
    snap: &Snapshot,
    key: &str,
    devices: &[AudioDevice],
    source: OutputSource<'_>,
    all: bool,
) -> Option<InputSourceView> {
    let entry = audio_entries(snap).into_iter().find(|e| e.key == key)?;
    let developer = snap.settings.developer_mode;
    // Not another device's Output Source made the default while it streams
    let default_output = own_default_output(devices, ProfilesView::from(snap));
    let looped = source_device(source.capture_mode, source.audio_device_id, &default_output);
    let shown = match &entry.profile {
        Some(p) => p.clone(),
        None => {
            let mut base = crate::profile::device_base(&entry.device, &snap.profiles);
            // VB-Cable first: the Hi-Fi Cable, with less delay, stays free for the sound
            let cable = [vb_cable(devices), hifi_cable(devices)].into_iter().flatten().find(|c| c.id != looped);
            if let Some(cable) = cable {
                // LE Audio takes Auto as On: its microphone stays Off with the cable
                base.headset_mic = if entry.le_audio { "off" } else { "auto" }.into();
                base.mic_device_id = cable.id.clone();
                base.mic_device_name = cable.name.clone();
            }
            base
        }
    };
    let saved = entry.profile.as_ref().map(|p| headset_mode(&p.headset_mic, developer)).unwrap_or(HeadsetMode::Off);
    let mode = if shown.mic_muted { HeadsetMode::Off } else { headset_mode(&shown.headset_mic, developer) };
    let chosen = shown.mic_device_id.clone();
    let chosen_device = devices.iter().find(|d| d.id == chosen);
    let loops = used_by_source(devices, &looped, &chosen);
    // Windows plays its sound into the chosen device (issue #86: the
    // microphone's cable left as the default output): apps hear it mixed
    // with the microphone, and an Output Source that is another device gets
    // nothing. Not while connecting makes the Output Source the default.
    let plays_default = chosen_device
        .is_some_and(|d| d.id == default_output && !source.switches && !used_by_source(devices, &looped, &d.id));
    let hifi_rates = chosen_device.filter(|d| is_hifi_cable(d)).and_then(|d| match (d.mix_rate, d.capture_rate) {
        (Some(m), Some(c)) if m != 0 && c != 0 && m != c => Some((m, c)),
        _ => None,
    });
    let on = mode != HeadsetMode::Off;
    Some(InputSourceView {
        options: mic_options(devices, &shown, &looped, all),
        no_output: on && chosen.is_empty(),
        loops: on && loops,
        hifi_rates: hifi_rates.filter(|_| on),
        plays_default: on && plays_default,
        modes: headset_modes(developer),
        entry,
        shown,
        mode,
        saved,
        chosen,
    })
}

// ---- PairingDialog ------------------------------------------------------------

/// How often and how long the services are asked again while another query
/// runs (issue #86: the stack's own SDP queries while connecting, or the
/// last one of this dialog; the core gives one up after 15 s)
pub const BUSY_RETRY_MS: u64 = 500;
pub const BUSY_RETRIES: u32 = 30;

/// One key: its hex digits (upper case) in the order Bluetooth sends them,
/// or None for "not available"
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRow {
    pub name: String,
    pub value: Option<String>,
    pub tag: Option<String>,
}

/// The keys' rows. An LE Audio entry has a link key only when its LE
/// pairing gave one (issue #86).
pub fn key_rows(le: bool, p: &DevicePairing) -> Vec<KeyRow> {
    let hex = |v: Option<&String>| v.filter(|v| !v.is_empty()).map(|v| v.to_uppercase());
    let mut rows = Vec::new();
    if !le || p.link_key.is_some() {
        rows.push(KeyRow {
            name: t("ui.pairing.link_key"),
            value: hex(p.link_key.as_ref().map(|k| &k.key)),
            tag: p.link_key.as_ref().map(|k| t!("ui.pairing.key_type", k.key_type)),
        });
    }
    let le_keys = p.le_keys.as_ref();
    rows.push(KeyRow {
        name: t("ui.pairing.ltk"),
        value: hex(le_keys.and_then(|k| k.long_term_key.as_ref())),
        tag: le_keys.map(|k| t!("ui.pairing.key_size", k.key_size)),
    });
    rows.push(KeyRow {
        name: t("ui.pairing.irk"),
        value: hex(le_keys.and_then(|k| k.identity_resolving_key.as_ref())),
        tag: None,
    });
    rows
}

/// Why there are no LE keys (issue #86), under a link key
pub fn le_missing_text(p: &DevicePairing) -> Option<String> {
    if p.link_key.is_none() || p.le_keys.is_some() {
        return None;
    }
    let why = serde_json::to_value(p.le_keys_missing?).ok()?;
    Some(t(&format!("ui.pairing.le_missing.{}", why.as_str()?)))
}

/// A refused services query is asked again (another query runs)
pub fn retry_services(code: &str, retries_left: u32) -> bool {
    code == "device.busy" && retries_left > 0
}

#[cfg(test)]
#[path = "../tests/devices.rs"]
mod tests;

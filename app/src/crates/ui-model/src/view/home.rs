//! Home's rules: the bars over the cards (firmware, another adapter, Hi-Res),
//! the error shown, and what each device's card shows and does. The parts
//! other screens use are in `view::stream` (entry_state, hidden_by_hung,
//! headset_chip).

use ipc::{AdapterInfo, AdapterStatus, DriverKind, ErrorInfo, Msg, Snapshot, StreamState};

use crate::codecs::{CARD_ORDER, card_of, card_order, offered_codecs};
use crate::i18n::t;
use crate::profile::{AudioEntry, codecs_on_request, known_codecs};
use crate::view::codecs::{codec_shown, picked_codec, shown_profile, streaming_codec};
use crate::view::stream::{
    adapter_drivers, entry_name, entry_state, hidden_by_hung, shows_battery, source_text, status_text,
};

/// Adapters whose firmware files are missing from the settings folder or
/// invalid (v1's "Firmware not found!" / "Firmware file is invalid!" bar).
/// Only those on WinUSB: one Windows runs, or that is disabled, needs none
/// yet (issue #86)
pub fn adapters_missing_firmware(snap: &Snapshot) -> Vec<&AdapterInfo> {
    snap.adapter
        .adapters
        .iter()
        .filter(|a| a.firmware_present == Some(false) && a.driver == DriverKind::Winusb)
        .collect()
}

/// The bar over Home while an adapter lacks its firmware files, or the
/// running stack found its Realtek adapter on the ROM code (issue #81)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FirmwareBar<'a> {
    /// Files missing or invalid: the adapters, the title's key, the one whose
    /// files the core can fetch (the first: one download at a time)
    Missing { adapters: Vec<&'a AdapterInfo>, title_key: &'static str, downloadable: Option<&'a AdapterInfo> },
    /// The files are there, the adapter runs its ROM code
    Rom,
}

/// Which firmware bar Home shows, if any
pub fn firmware_bar(snap: &Snapshot) -> Option<FirmwareBar<'_>> {
    let missing = adapters_missing_firmware(snap);
    if missing.is_empty() {
        return snap.adapter.rom_firmware.then_some(FirmwareBar::Rom);
    }
    let title_key = if missing.iter().any(|a| a.firmware_invalid.is_some()) {
        "firmware.invalid_short"
    } else {
        "firmware.missing_manual"
    };
    let downloadable = missing.iter().find(|a| a.firmware_download).copied();
    Some(FirmwareBar::Missing { adapters: missing, title_key, downloadable })
}

/// Another Bluetooth adapter is active (issue #86): its line, until dismissed
pub fn other_radio_text(snap: &Snapshot) -> Option<String> {
    let r = snap.adapter.other_radio.as_ref()?;
    let key = if r.paired { "ui.home.other_adapter_paired" } else { "ui.home.other_adapter_unpaired" };
    Some(crate::t!(key, &r.name))
}

/// OPPO earbuds turned Hi-Res on and reconnected once for it (issue #237):
/// said for the stream it happened in, until dismissed
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HiResBar {
    dismissed: bool,
}

impl HiResBar {
    /// Follows the stream: a stream without it forgets the dismissal
    pub fn update(&mut self, snap: &Snapshot) {
        if !hi_res_set(snap) {
            self.dismissed = false;
        }
    }

    pub fn dismiss(&mut self) {
        self.dismissed = true;
    }

    pub fn shown(&self, snap: &Snapshot) -> bool {
        hi_res_set(snap) && !self.dismissed
    }
}

fn hi_res_set(snap: &Snapshot) -> bool {
    snap.stream.info.as_ref().is_some_and(|i| i.hi_res_set)
}

/// The error over the cards: its request's code (if a request failed) and
/// its message
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeError {
    pub code: Option<String>,
    pub message: Msg,
}

/// The error Home shows: a failed request (Connect) until the state moves
/// on, else the stream's, else the adapter's; none while the adapter seems
/// hung and the error is the stack's start-up one (AdapterHung says what to do)
pub fn home_error(snap: &Snapshot, request: Option<&ErrorInfo>) -> Option<HomeError> {
    let e = if let Some(e) = request {
        HomeError { code: Some(e.code.clone()), message: e.message.clone() }
    } else if snap.stream.state == StreamState::Error {
        HomeError { code: None, message: snap.stream.message.clone() }
    } else {
        let a = &snap.adapter;
        let m = a.error.as_ref().filter(|_| a.status == AdapterStatus::Error)?;
        HomeError { code: None, message: m.clone() }
    };
    (!hidden_by_hung(snap, Some(&e.message))).then_some(e)
}

/// What clears a failed request's error when it changes: the stream's state
/// and the adapters' drivers (the error's own button put one on WinUSB)
pub fn request_error_key(snap: &Snapshot) -> (StreamState, String) {
    (snap.stream.state, adapter_drivers(snap))
}

/// A lone card does not stretch over the page: "Add device" beside it
/// (issue #86)
pub fn offers_add(entries: &[AudioEntry]) -> bool {
    entries.len() == 1
}

/// What a device's card shows
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCard {
    pub state: StreamState,
    /// Its device is the one in use (connecting, streaming, or failed)
    pub live: bool,
    /// Its button stops the stream
    pub running: bool,
    /// The device's name, else its address
    pub name: String,
    /// The battery beside the state
    pub battery: bool,
    /// The status dot: not for an error (a red dot read as recording, issue #86)
    pub dot: bool,
    pub status: String,
    /// The button's label: Disconnect, Cancel or Connect
    pub label: String,
    /// The application it captures is not playing yet, in words
    pub waiting: Option<String>,
}

/// One device: ring, name, battery and state, codec, volume, link quality
/// while streaming, and its own button, all in one card (issue #86)
pub fn device_card(snap: &Snapshot, entry: &AudioEntry) -> DeviceCard {
    let s = &snap.stream;
    let state = entry_state(snap, entry);
    let live = state != StreamState::Idle;
    let running = matches!(state, StreamState::Streaming | StreamState::Connecting | StreamState::Reconnecting);
    let label = t(match state {
        StreamState::Streaming => "connection.disconnect",
        StreamState::Connecting | StreamState::Reconnecting => "modal.cancel",
        _ => "connection.connect",
    });
    // An error's text is in the panel above, with its fix
    let status = if live { status_text(state, &s.message, s.codec_stop.as_ref()) } else { t("status.ready") };
    let waiting = s
        .info
        .as_ref()
        .and_then(|i| i.source.as_ref())
        .filter(|src| live && src.waiting)
        .map(|src| source_text(src, entry.profile.as_ref()).text);
    DeviceCard {
        state,
        live,
        running,
        name: entry_name(entry).to_owned(),
        battery: live && shows_battery(s),
        dot: state != StreamState::Error,
        status,
        label,
        waiting,
    }
}

/// What a card's button does
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardPress {
    /// Stops the stream (Disconnect, Cancel)
    Stop,
    /// Starts the device's profile; another device streaming gives way
    /// (stream.start stops it first)
    Start(String),
    /// A device without a profile: Auto is made for it first, then started
    StartAuto,
}

pub fn card_press(snap: &Snapshot, entry: &AudioEntry) -> CardPress {
    if device_card(snap, entry).running {
        CardPress::Stop
    } else {
        entry.profile.as_ref().map_or(CardPress::StartAuto, |p| CardPress::Start(p.name.clone()))
    }
}

// ---------------------------------------------------------------------------
// The codec chosen on the card (Home's CodecPicker, also in the flyout)
// ---------------------------------------------------------------------------

/// What the card's codec list shows
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecPicker {
    /// The codec shown chosen; None: "None" (not known yet)
    pub value: Option<String>,
    /// The codecs to choose from, in the device's order
    pub choices: Vec<String>,
}

/// The codec of the device in use, chosen on its card (Home, the flyout;
/// issue #86): one of the codecs it supports, as on Codecs, by name only.
/// It shows the codec streaming (the warning beside it says why another
/// one than chosen), else Auto's pick or the one chosen; choosing a codec turns
/// Auto-select off, which is turned back on on Codecs. An LE Audio device
/// has LC3 only (no list: the window shows the name).
pub fn codec_picker(snap: &Snapshot, entry: &AudioEntry) -> CodecPicker {
    let developer = snap.settings.developer_mode;
    let auto_all = snap.settings.auto_codec;
    let codec = entry_codec(snap, entry);
    let order = picker_order(entry, auto_all);
    let picked = picked_codec(snap, entry, auto_all, &order);
    // What it streams with; else Auto's pick, or the one used instead of a
    // codec it does not offer
    let value = streaming_codec(snap, entry).map(str::to_owned).or(picked.clone()).or_else(|| {
        (codec != "auto" && !offered_codecs(&[codec.as_str()], developer).is_empty()).then(|| codec.clone())
    });
    // A quick choice: what the device supports, never the codecs Codecs hides
    let known = known_codecs(&snap.devices, &entry.address);
    let on_request = codecs_on_request(&snap.devices, &entry.address);
    let mut choices: Vec<String> = offered_codecs(&order, developer)
        .into_iter()
        .filter(|id| codec_shown(id, picked.as_deref(), known, true, on_request))
        .map(str::to_owned)
        .collect();
    if let Some(v) = value.as_ref().filter(|v| !choices.contains(v)) {
        choices.insert(0, v.clone());
    }
    CodecPicker { value, choices }
}

/// What choosing a codec on the card does
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecPick {
    /// The one shown, or the one chosen already while another one streams
    /// instead (in use elsewhere): the list goes back to what it shows
    Keep,
    /// Saved with the device's profile; `auto_off`: Auto-select (one setting
    /// for every device, as on Codecs) is turned off first, so the device
    /// connects again with it
    Save { auto_off: bool },
}

pub fn codec_pick(snap: &Snapshot, entry: &AudioEntry, id: &str) -> CodecPick {
    let auto_all = snap.settings.auto_codec;
    if codec_picker(snap, entry).value.as_deref() == Some(id) || (!auto_all && id == entry_codec(snap, entry)) {
        return CodecPick::Keep;
    }
    CodecPick::Save { auto_off: auto_all }
}

/// The stream starts again with the codec chosen: the next choice waits for
/// it, as on Codecs (issue #86)
pub fn codec_restarts(snap: &Snapshot, entry: &AudioEntry) -> bool {
    !matches!(entry_state(snap, entry), StreamState::Idle | StreamState::Error)
}

/// The card the device's profile is on (Auto for a device without one)
fn entry_codec(snap: &Snapshot, entry: &AudioEntry) -> String {
    let p = shown_profile(snap, entry);
    card_of(&p.codec, &p.quality).to_owned()
}

/// The device's card order; Auto-select's default under it
fn picker_order(entry: &AudioEntry, auto_all: bool) -> Vec<&'static str> {
    if auto_all {
        CARD_ORDER.to_vec()
    } else {
        card_order(entry.profile.as_ref().map_or(&[][..], |p| p.codec_order.as_slice()))
    }
}

#[cfg(test)]
mod tests;

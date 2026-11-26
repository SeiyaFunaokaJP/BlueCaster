//! The decisions behind the Auracast tab (Developer screen) and its matrix
//! mixer: the broadcast's choices and the request they make, when the
//! broadcast counts as live, the length checks, the adapter list, and the
//! mixer's fields.

use std::collections::{BTreeMap, HashMap};

use ipc::{
    AdapterInfo, AudioDevice, AuracastReceiverCandidate, AuracastReceiverStatus, BroadcastReceiver, DriverKind, Method,
    SourceInfo, StreamState, StreamStatus,
};

use super::adapter::adapter_name;
use crate::format::js_trim;
use crate::i18n::{js_number, t, tm};
use crate::series::to_fixed;
use crate::t;

/// What is broadcast: the test tone, the system audio, the loopback of an
/// output device or a recording device (each by its endpoint id)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Source {
    Tone,
    Loopback,
    Output(String),
    Input(String),
}

/// A device source's kind
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Output,
    Input,
}

impl DeviceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DeviceKind::Output => "output",
            DeviceKind::Input => "input",
        }
    }
}

impl Source {
    /// A device source's kind and endpoint id; None: the tone or system audio
    pub fn device(&self) -> Option<(DeviceKind, &str)> {
        match self {
            Source::Output(id) => Some((DeviceKind::Output, id)),
            Source::Input(id) => Some((DeviceKind::Input, id)),
            Source::Tone | Source::Loopback => None,
        }
    }
}

/// The names of the devices listed so far, by id, for a chosen one that is gone
pub type DeviceNames = HashMap<String, String>;

/// The source as stream.start_broadcast and auracast.set_source take it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceParams {
    pub source: String,
    pub source_id: Option<String>,
    pub source_name: Option<String>,
}

impl SourceParams {
    pub fn set_source_request(&self) -> Method {
        Method::AuracastSetSource {
            source: self.source.clone(),
            source_id: self.source_id.clone(),
            source_name: self.source_name.clone(),
        }
    }
}

/// The source as stream.start_broadcast and auracast.set_source take it
/// (`names`: the names of devices listed before, for one that is gone)
pub fn source_params(
    source: &Source,
    outputs: &[AudioDevice],
    inputs: &[AudioDevice],
    names: &DeviceNames,
) -> SourceParams {
    let Some((kind, id)) = source.device() else {
        let source = if *source == Source::Tone { "tone" } else { "loopback" };
        return SourceParams { source: source.to_owned(), source_id: None, source_name: None };
    };
    let list = if kind == DeviceKind::Output { outputs } else { inputs };
    let name = list.iter().find(|d| d.id == id).map(|d| d.name.clone()).or_else(|| names.get(id).cloned());
    SourceParams { source: kind.as_str().to_owned(), source_id: Some(id.to_owned()), source_name: name }
}

/// Whether the broadcast's stream info names the source sent as `params`
/// (a device by the name sent, or by its id when none was)
pub fn plays_source(info: Option<&SourceInfo>, params: &SourceParams) -> bool {
    let Some(info) = info else { return false };
    let kind = if info.kind == "virtual" { "output" } else { info.kind.as_str() };
    if kind != params.source {
        return false;
    }
    match &params.source_id {
        None => true,
        Some(id) => info.name == *params.source_name.as_ref().unwrap_or(id),
    }
}

/// The choice of what the broadcast plays (its stream info), a device by
/// its name (the first of two with the same name); None: none of them
pub fn source_of_info(info: Option<&SourceInfo>, outputs: &[AudioDevice], inputs: &[AudioDevice]) -> Option<Source> {
    let info = info?;
    let named = |list: &[AudioDevice]| list.iter().find(|d| d.name == info.name).map(|d| d.id.clone());
    match info.kind.as_str() {
        "tone" => Some(Source::Tone),
        "loopback" => Some(Source::Loopback),
        "output" | "virtual" => named(outputs).map(Source::Output),
        "input" => named(inputs).map(Source::Input),
        _ => None,
    }
}

/// One entry of the source list; entries with the same group (next to each
/// other) go under one heading
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceOption {
    pub value: Source,
    pub label: String,
    pub group: Option<String>,
}

/// The sources to choose from: the tone, the system audio, Windows' output
/// and recording devices, and a chosen device that is not there now (by
/// its name in `names`, the devices listed before)
pub fn source_options(
    chosen: &Source,
    outputs: &[AudioDevice],
    inputs: &[AudioDevice],
    names: &DeviceNames,
) -> Vec<SourceOption> {
    let plain = |value: Source, key: &str| SourceOption { value, label: t(key), group: None };
    let device = |make: fn(String) -> Source, group: String| {
        move |d: &AudioDevice| SourceOption {
            value: make(d.id.clone()),
            label: d.name.clone(),
            group: Some(group.clone()),
        }
    };
    let mut options =
        vec![plain(Source::Tone, "receiver.source_tone"), plain(Source::Loopback, "receiver.source_system")];
    options.extend(outputs.iter().map(device(Source::Output, t("ui.dev.auracast_outputs"))));
    options.extend(inputs.iter().map(device(Source::Input, t("ui.dev.auracast_inputs"))));
    if !options.iter().any(|o| o.value == *chosen) {
        let d = chosen.device();
        let input = matches!(d, Some((DeviceKind::Input, _)));
        let group = t(if input { "ui.dev.auracast_inputs" } else { "ui.dev.auracast_outputs" });
        let name = d
            .and_then(|(_, id)| names.get(id))
            .filter(|n| !n.is_empty())
            .cloned()
            .unwrap_or_else(|| t(if input { "capture.input" } else { "capture.output" }));
        options.push(SourceOption {
            value: chosen.clone(),
            label: t!("ui.dev.auracast_device_gone", name),
            group: Some(group),
        });
    }
    options
}

/// A source whose Windows volume the broadcast can follow (not the tone,
/// nor a recording device: its recording level is in the audio already)
pub fn followable(s: &Source) -> bool {
    !matches!(s, Source::Tone | Source::Input(_))
}

/// The broadcast's rate: 48 kHz (high), 32 kHz, 24 kHz (standard)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    Hq,
    Mq,
    Sq,
}

impl Quality {
    pub fn as_str(self) -> &'static str {
        match self {
            Quality::Hq => "hq",
            Quality::Mq => "mq",
            Quality::Sq => "sq",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channels {
    Stereo,
    Mono,
    Split,
    Matrix,
    /// The matrix mixer's four outputs in one BIG of four BIS
    Single,
}

/// How [`Channels::Single`] lays out its four BIS (le_audio::broadcast::SingleMode)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SingleLayout {
    /// FL FR, the rear without a location, one subgroup
    RearMono,
    /// FL FR BL BR, one subgroup
    Located,
    /// Two subgroups of FL FR
    Pairs,
    /// A subgroup per BIS, none located
    PerBis,
}

impl SingleLayout {
    pub const ALL: [SingleLayout; 4] =
        [SingleLayout::RearMono, SingleLayout::Located, SingleLayout::Pairs, SingleLayout::PerBis];

    /// The layout's name as stream.start_broadcast takes it
    pub fn layout(self) -> &'static str {
        match self {
            SingleLayout::RearMono => "single",
            SingleLayout::Located => "single_located",
            SingleLayout::Pairs => "single_pairs",
            SingleLayout::PerBis => "single_per_bis",
        }
    }

    pub fn label_key(self) -> &'static str {
        match self {
            SingleLayout::RearMono => "ui.dev.auracast_single_rear_mono",
            SingleLayout::Located => "ui.dev.auracast_single_located",
            SingleLayout::Pairs => "ui.dev.auracast_single_pairs",
            SingleLayout::PerBis => "ui.dev.auracast_single_per_bis",
        }
    }
}

/// Which receivers play which outputs (one or more each, as an Auracast
/// assistant tells them)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrangement {
    /// One receiver for all: stereo, or the mono one
    One,
    /// A left and a right receiver
    LeftRight,
    /// Four mono receivers: FL FR RL RR
    FourMono,
    /// A stereo front and two mono rears
    StereoTwoMono,
    /// A stereo front and a stereo rear
    TwoStereo,
}

impl Arrangement {
    pub fn label_key(self) -> &'static str {
        match self {
            Arrangement::One => "ui.dev.auracast_rx_one",
            Arrangement::LeftRight => "ui.dev.auracast_rx_left_right",
            Arrangement::FourMono => "ui.dev.auracast_rx_four_mono",
            Arrangement::StereoTwoMono => "ui.dev.auracast_rx_stereo_two_mono",
            Arrangement::TwoStereo => "ui.dev.auracast_rx_two_stereo",
        }
    }
}

/// The arrangements a layout offers, the first its default
pub fn arrangements(channels: Channels) -> &'static [Arrangement] {
    match channels {
        Channels::Stereo => &[Arrangement::One, Arrangement::LeftRight],
        Channels::Mono => &[Arrangement::One],
        Channels::Split => &[Arrangement::LeftRight],
        Channels::Matrix | Channels::Single => {
            &[Arrangement::FourMono, Arrangement::StereoTwoMono, Arrangement::TwoStereo]
        }
    }
}

/// A place for receivers: the outputs they play
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    /// Its key in [`Choices::receivers`] (kept from one arrangement to another)
    pub key: &'static str,
    /// The outputs, as stream.start_broadcast's receivers name them
    pub channels: &'static [&'static str],
    pub label_key: &'static str,
}

const FRONT: Slot = Slot { key: "FL+FR", channels: &["FL", "FR"], label_key: "ui.dev.auracast_slot_front" };
const REAR: Slot = Slot { key: "RL+RR", channels: &["RL", "RR"], label_key: "ui.dev.auracast_slot_rear" };
const STEREO: Slot = Slot { key: "FL+FR", channels: &["FL", "FR"], label_key: "ui.dev.auracast_slot_stereo" };
const MONO: Slot = Slot { key: "M", channels: &["M"], label_key: "ui.dev.auracast_slot_mono" };
const LEFT: Slot = Slot { key: "FL", channels: &["FL"], label_key: "ui.dev.auracast_slot_left" };
const RIGHT: Slot = Slot { key: "FR", channels: &["FR"], label_key: "ui.dev.auracast_slot_right" };
const FL: Slot = Slot { key: "FL", channels: &["FL"], label_key: "ui.dev.auracast_slot_fl" };
const FR: Slot = Slot { key: "FR", channels: &["FR"], label_key: "ui.dev.auracast_slot_fr" };
const RL: Slot = Slot { key: "RL", channels: &["RL"], label_key: "ui.dev.auracast_slot_rl" };
const RR: Slot = Slot { key: "RR", channels: &["RR"], label_key: "ui.dev.auracast_slot_rr" };

/// A receiver chosen for a slot
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiverPick {
    pub address: String,
    pub name: String,
}

/// Saved LE Audio devices to offer as receivers: those without BASS left
/// out, those not known yet marked
pub fn receiver_options(candidates: &[AuracastReceiverCandidate]) -> Vec<(String, String)> {
    let mut v = vec![(String::new(), t("ui.dev.auracast_rx_none"))];
    v.extend(candidates.iter().filter(|c| c.bass != Some(false)).map(|c| {
        let name = if c.name.is_empty() { c.address.clone() } else { c.name.clone() };
        let label = if c.bass.is_none() { t!("ui.dev.auracast_rx_unchecked", name) } else { name };
        (c.address.clone(), label)
    }));
    v
}

/// A receiver's state as the list of the broadcast's receivers shows it
pub fn receiver_status_text(r: &AuracastReceiverStatus) -> String {
    let who = if r.name.is_empty() { r.address.clone() } else { r.name.clone() };
    let bis = |v: &[u8]| v.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(", ");
    let phase = match r.phase.as_str() {
        "playing" => t!("ui.dev.auracast_rx_playing", bis(&r.bis)),
        p @ ("waiting" | "looking" | "connecting" | "ready" | "added" | "synced" | "done" | "not_found" | "no_bass"
        | "failed") => t(&format!("ui.dev.auracast_rx_{p}")),
        other => other.to_owned(),
    };
    let asked = if r.asked.is_empty() { String::new() } else { t!("ui.dev.auracast_rx_asked", bis(&r.asked)) };
    let detail = if r.detail.is_empty() { String::new() } else { format!(" ({})", r.detail) };
    format!("{who}: {phase}{detail}{asked}")
}

/// How a pair of matrix outputs is sent (le_audio::broadcast::PairMode)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairMode {
    Stereo,
    Separate,
    Subgroups,
    Off,
}

impl PairMode {
    pub fn as_str(self) -> &'static str {
        match self {
            PairMode::Stereo => "stereo",
            PairMode::Separate => "separate",
            PairMode::Subgroups => "subgroups",
            PairMode::Off => "off",
        }
    }
}

/// The BIGs (broadcasts) a pair takes
pub fn pair_bigs(m: PairMode) -> u32 {
    match m {
        PairMode::Separate => 2,
        PairMode::Off => 0,
        PairMode::Stereo | PairMode::Subgroups => 1,
    }
}

/// How long after a change of source the stream info names the one that
/// plays (the core switches at its next 200 ms tick, then opens the capture)
pub const SOURCE_SETTLE_MS: u64 = 1500;
/// BIGs the ASUS BT600 runs at most (a third: LE Create BIG 0x0D)
pub const MAX_BIGS: u32 = 2;

/// The core's limits (le_audio::broadcast): UTF-8 bytes
pub const NAME_BYTES: (usize, usize) = (4, 32);
pub const CODE_BYTES: (usize, usize) = (4, 16);

/// The form's choices; they outlive the view (one set per core connection)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choices {
    pub name: String,
    pub code: String,
    pub quality: Quality,
    pub channels: Channels,
    /// Matrix: the front (FL FR) and the rear (RL RR), each with its rate
    pub front: PairMode,
    pub rear: PairMode,
    pub front_quality: Quality,
    pub rear_quality: Quality,
    /// [`Channels::Single`]: how its BIS are laid out
    pub single: SingleLayout,
    /// Who plays what (of [`arrangements`] for the channels)
    pub arrangement: Arrangement,
    /// The receivers of each slot by its key (one each for now)
    pub receivers: BTreeMap<String, Vec<ReceiverPick>>,
    pub source: Source,
    /// The broadcast's volume follows the Windows volume
    pub follow: bool,
}

impl Default for Choices {
    fn default() -> Choices {
        Choices {
            name: "BlueCaster".to_owned(),
            code: String::new(),
            quality: Quality::Hq,
            channels: Channels::Stereo,
            front: PairMode::Stereo,
            rear: PairMode::Stereo,
            front_quality: Quality::Hq,
            rear_quality: Quality::Hq,
            single: SingleLayout::RearMono,
            arrangement: Arrangement::One,
            receivers: BTreeMap::new(),
            source: Source::Tone,
            follow: false,
        }
    }
}

/// The layout as stream.start_broadcast takes it
pub fn layout_of(channels: Channels, front: PairMode, rear: PairMode) -> Option<String> {
    match channels {
        Channels::Mono => None,
        Channels::Matrix => Some(format!("matrix_{}_{}", front.as_str(), rear.as_str())),
        Channels::Stereo => Some("stereo".to_owned()),
        Channels::Split => Some("split".to_owned()),
        Channels::Single => Some("single".to_owned()),
    }
}

/// The error of a name or code field, None when it is fine (an empty code: none)
pub fn length_error(text: &str, (min, max): (usize, usize), optional: bool) -> Option<String> {
    let n = if optional { text.len() } else { js_trim(text).len() };
    if optional && n == 0 {
        return None;
    }
    (n < min || n > max).then(|| t!("ui.dev.auracast_err_length", min, max))
}

impl Choices {
    pub fn matrix(&self) -> bool {
        self.channels == Channels::Matrix
    }

    /// Chooses the channels. One BIG of four BIS goes from high quality to
    /// medium: at 48 kHz its four BIS lost most of their audio on the
    /// adapter tried (an ASUS BT600), at 32 kHz none. High stays choosable.
    pub fn set_channels(&mut self, channels: Channels) {
        if channels == Channels::Single && self.channels != Channels::Single && self.quality == Quality::Hq {
            self.quality = Quality::Mq;
        }
        self.channels = channels;
    }

    /// Fed by the matrix mixer: a matrix, or its outputs in one BIG
    pub fn mixed(&self) -> bool {
        matches!(self.channels, Channels::Matrix | Channels::Single)
    }

    /// The layout as stream.start_broadcast takes it
    pub fn layout(&self) -> Option<String> {
        match self.channels {
            Channels::Single => Some(self.single.layout().to_owned()),
            _ => layout_of(self.channels, self.front, self.rear),
        }
    }

    /// The arrangement in force: the one chosen if the channels offer it,
    /// else their first
    pub fn arrangement(&self) -> Arrangement {
        let offered = arrangements(self.channels);
        if offered.contains(&self.arrangement) { self.arrangement } else { offered[0] }
    }

    /// The slots for receivers: of the arrangement, those whose outputs
    /// the layout sends (a matrix pair off sends none). A matrix pair sent
    /// separately is two broadcasts, and a receiver plays one: its stereo
    /// slot is the two mono ones.
    pub fn slots(&self) -> Vec<Slot> {
        let all: &[Slot] = match self.arrangement() {
            Arrangement::One if self.channels == Channels::Mono => &[MONO],
            Arrangement::One => &[STEREO],
            Arrangement::LeftRight => &[LEFT, RIGHT],
            Arrangement::FourMono => &[FL, FR, RL, RR],
            Arrangement::StereoTwoMono => &[FRONT, RL, RR],
            Arrangement::TwoStereo => &[FRONT, REAR],
        };
        let sent = |s: &Slot| {
            !self.matrix()
                || s.channels
                    .iter()
                    .all(|c| if c.starts_with('R') { self.rear != PairMode::Off } else { self.front != PairMode::Off })
        };
        let apart = |pair: PairMode| self.matrix() && pair == PairMode::Separate;
        all.iter()
            .copied()
            .filter(sent)
            .flat_map(|s| match s.key {
                "FL+FR" if apart(self.front) => vec![FL, FR],
                "RL+RR" if apart(self.rear) => vec![RL, RR],
                _ => vec![s],
            })
            .collect()
    }

    /// The receiver chosen for a slot (its first)
    pub fn receiver_of(&self, slot: &Slot) -> Option<&ReceiverPick> {
        self.receivers.get(slot.key).and_then(|v| v.first())
    }

    /// Sets the one receiver of a slot (None: none)
    pub fn set_receiver(&mut self, key: &str, pick: Option<ReceiverPick>) {
        match pick {
            Some(p) => {
                self.receivers.insert(key.to_owned(), vec![p]);
            }
            None => {
                self.receivers.remove(key);
            }
        }
    }

    /// The receivers to tell, of the slots in force
    pub fn receivers_request(&self) -> Vec<BroadcastReceiver> {
        self.slots()
            .iter()
            .flat_map(|s| {
                self.receivers.get(s.key).into_iter().flatten().map(move |r| BroadcastReceiver {
                    address: r.address.clone(),
                    name: r.name.clone(),
                    channels: s.channels.iter().map(|c| (*c).to_owned()).collect(),
                })
            })
            .collect()
    }

    /// A receiver chosen for two slots: an error (it can be told one source)
    pub fn receiver_error(&self) -> Option<String> {
        let list = self.receivers_request();
        let twice = list.iter().enumerate().find(|(i, r)| list[..*i].iter().any(|o| o.address == r.address))?;
        let r = twice.1;
        Some(t!("ui.dev.auracast_rx_twice", if r.name.is_empty() { r.address.clone() } else { r.name.clone() }))
    }

    /// The BIGs a matrix takes (0 for the other layouts)
    pub fn bigs(&self) -> u32 {
        if self.matrix() { pair_bigs(self.front) + pair_bigs(self.rear) } else { 0 }
    }

    pub fn name_error(&self) -> Option<String> {
        length_error(&self.name, NAME_BYTES, false)
    }

    pub fn code_error(&self) -> Option<String> {
        length_error(&self.code, CODE_BYTES, true)
    }

    /// Start is off while a field is wrong or a matrix sends nothing
    pub fn start_disabled(&self, live: bool) -> bool {
        !live
            && (self.name_error().is_some()
                || self.code_error().is_some()
                || self.receiver_error().is_some()
                || (self.matrix() && self.bigs() == 0))
    }

    /// stream.start_broadcast with the source as `source` names it (see
    /// [`source_params`]); a matrix's rates come from its front and rear
    /// rows, not Quality. The volume is left to the core (the last one).
    pub fn start_request(&self, source: SourceParams) -> Method {
        let matrix = self.matrix();
        Method::StreamStartBroadcast {
            name: js_trim(&self.name).to_owned(),
            code: (!self.code.is_empty()).then(|| self.code.clone()),
            quality: if matrix { self.front_quality } else { self.quality }.as_str().to_owned(),
            rear_quality: matrix.then(|| self.rear_quality.as_str().to_owned()),
            mono: Some(self.channels == Channels::Mono),
            split: None,
            layout: self.layout(),
            source: source.source,
            source_id: source.source_id,
            source_name: source.source_name,
            volume: None,
            follow_volume: Some(self.follow),
            receivers: Some(self.receivers_request()).filter(|v| !v.is_empty()),
        }
    }
}

/// A broadcast asked for here, as the stream goes
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Session {
    /// A broadcast asked for here, until the stream it started ends or
    /// another takes over
    pub mine: bool,
    /// Why the last broadcast asked for here ended with an error
    pub failed: Option<String>,
    /// stream.start_broadcast not answered yet: the stream before it still shows
    pub asking: bool,
    /// The broadcast asked for here went on air. Until then an Idle is the
    /// session before it ending: the core answers stream.start_broadcast,
    /// then stops what ran before (Idle), then sets the broadcast up
    pub on_air: bool,
}

impl Session {
    /// Follows the stream's state
    pub fn follow(&mut self, st: &StreamStatus) {
        if !self.mine || self.asking {
            return;
        }
        let broadcasting = st.message.key == "status.broadcasting";
        match st.state {
            StreamState::Error => {
                self.failed = Some(tm(Some(&st.message)));
                self.mine = false;
            }
            StreamState::Streaming if broadcasting => self.on_air = true,
            // Stopped, or another stream took over
            StreamState::Streaming => self.mine = false,
            StreamState::Idle if self.on_air => self.mine = false,
            _ => {}
        }
    }

    /// The running stream is the broadcast
    pub fn live(&self, st: &StreamStatus) -> bool {
        match st.state {
            StreamState::Streaming => st.message.key == "status.broadcasting",
            // Setting up, or starting again after the adapter stopped working
            StreamState::Connecting | StreamState::Reconnecting => self.mine && st.profile.is_none(),
            _ => false,
        }
    }

    /// What to show under the form: the broadcast's state, or why it ended
    /// (true: an error)
    pub fn status(&self, st: &StreamStatus) -> Option<(String, bool)> {
        if self.live(st) {
            return Some((tm(Some(&st.message)), false));
        }
        self.failed.clone().map(|f| (f, true))
    }

    /// The form's fields are locked
    pub fn locked(&self, st: &StreamStatus) -> bool {
        self.live(st) || self.asking
    }

    /// While its broadcasts come up the source cannot change yet
    pub fn source_locked(&self, st: &StreamStatus) -> bool {
        self.asking || (self.live(st) && st.state != StreamState::Streaming)
    }
}

/// After a change of source from `before` to `asked` (each with what was
/// sent for it) and SOURCE_SETTLE_MS: a source that cannot be captured
/// leaves the broadcast on the one before, which its stream info then still
/// names -> the source to show again. By what was sent, not by name alone:
/// two devices can share a name.
pub fn settled_source(
    live: bool,
    chosen: &Source,
    (asked, sent): (&Source, &SourceParams),
    (before, sent_before): (&Source, &SourceParams),
    info: Option<&SourceInfo>,
    outputs: &[AudioDevice],
    inputs: &[AudioDevice],
) -> Option<Source> {
    if !live || chosen != asked || plays_source(info, sent) {
        return None;
    }
    let playing =
        if plays_source(info, sent_before) { Some(before.clone()) } else { source_of_info(info, outputs, inputs) };
    playing.filter(|p| p != asked)
}

fn hex4(n: u16) -> String {
    format!("{n:04X}")
}

/// The adapter for broadcasts (Settings::auracast_adapter_id): the current
/// one first, then each adapter on WinUSB by name and MAC address (two
/// adapters can share a name, "Bluetooth Controller"), VID:PID while its
/// address is not known yet, and a chosen one that is not there. The value
/// returned is the option the setting names, whatever its case.
pub fn auracast_adapter_choices(list: &[AdapterInfo], chosen: &str) -> (String, Vec<(String, String)>) {
    let winusb: Vec<&AdapterInfo> = list.iter().filter(|a| a.driver == DriverKind::Winusb).collect();
    let id = js_trim(chosen);
    let mut options = vec![(String::new(), t("ui.dev.auracast_adapter_current"))];
    options.extend(winusb.iter().map(|a| {
        let at = a.address.clone().unwrap_or_else(|| format!("{}:{}", hex4(a.vid), hex4(a.pid)));
        (a.instance_id.clone(), format!("{} · {at}", adapter_name(a)))
    }));
    let found = winusb.iter().find(|a| !id.is_empty() && a.instance_id.to_lowercase() == id.to_lowercase());
    if !id.is_empty() && found.is_none() {
        options.push((id.to_owned(), t("ui.dev.auracast_adapter_missing")));
    }
    (found.map(|a| a.instance_id.clone()).unwrap_or_else(|| id.to_owned()), options)
}

/// The adapter choice is offered while several adapters are on WinUSB, or
/// while one is chosen (to go back to the current one)
pub fn show_adapter(list: &[AdapterInfo], chosen: &str) -> bool {
    list.iter().filter(|a| a.driver == DriverKind::Winusb).count() > 1 || !js_trim(chosen).is_empty()
}

// ---- The matrix mixer ------------------------------------------------------

pub const INPUT_NAMES: [&str; 8] = ["FL", "FR", "FC", "LFE", "BL", "BR", "SL", "SR"];
pub const OUTPUT_NAMES: [&str; 4] = ["FL", "FR", "RL", "RR"];
/// Sound in air at about 20 °C: metres per millisecond
pub const M_PER_MS: f64 = 0.343;
pub const MAX_DELAY_MS: f64 = 500.;
pub const MAX_GAIN: f64 = 4.;
/// Changes go out after this pause (typing, dragging)
pub const MIX_SEND_MS: u64 = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Ms,
    M,
}

/// `Math.round`: halves go up
fn js_round(n: f64) -> f64 {
    (n + 0.5).floor()
}

/// A gain as the grid shows it
pub fn gain_text(g: f64) -> String {
    if g == 0. {
        return "0".to_owned();
    }
    let s = to_fixed(g, 3);
    let s = s.trim_end_matches('0');
    s.strip_suffix('.').unwrap_or(s).to_owned()
}

/// A delay field's value in `unit` (ms kept to 0.01)
pub fn delay_in(ms: f64, unit: Unit) -> String {
    match unit {
        Unit::Ms => js_number(js_round(ms * 100.) / 100.),
        Unit::M => js_number(js_round(ms * M_PER_MS * 10000.) / 10000.),
    }
}

/// A field's number ("0,25" too), None when it is none
fn number(text: &str) -> Option<f64> {
    let text = text.replacen(',', ".", 1);
    let s = js_trim(&text);
    if s.is_empty() {
        return None;
    }
    s.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// A delay typed in `unit`, in ms within range (None: not a number)
pub fn delay_from(text: &str, unit: Unit) -> Option<f64> {
    let v = number(text)?;
    let ms = match unit {
        Unit::Ms => v,
        Unit::M => v / M_PER_MS,
    };
    Some((js_round(ms * 100.) / 100.).clamp(0., MAX_DELAY_MS))
}

/// A number field's value within lo..=hi (None: not a number)
pub fn num(text: &str, lo: f64, hi: f64) -> Option<f64> {
    number(text).map(|v| v.clamp(lo, hi))
}

/// A preset's name in the list: the built-in ones translated
pub fn preset_label(name: &str, builtin: bool) -> String {
    if builtin { t(&format!("ui.dev.auracast_mix_preset_{name}")) } else { name.to_owned() }
}

/// A name, and not a built-in preset's (the core refuses those)
pub fn can_save_preset(name: &str, builtin_names: &[&str]) -> bool {
    let name = js_trim(name);
    !name.is_empty() && !builtin_names.contains(&name)
}

#[cfg(test)]
mod tests {
    use ipc::Msg;

    use super::*;

    // The broadcast form
    #[test]
    fn names_the_layout_as_the_core_takes_it() {
        let (f, r) = (PairMode::Stereo, PairMode::Separate);
        assert_eq!(layout_of(Channels::Stereo, f, r).as_deref(), Some("stereo"));
        assert_eq!(layout_of(Channels::Mono, f, r), None);
        assert_eq!(layout_of(Channels::Split, f, r).as_deref(), Some("split"));
        assert_eq!(layout_of(Channels::Matrix, f, r).as_deref(), Some("matrix_stereo_separate"));
    }

    #[test]
    fn counts_the_bigs_of_each_pair_mode() {
        let all = [PairMode::Stereo, PairMode::Separate, PairMode::Subgroups, PairMode::Off];
        assert_eq!(all.map(pair_bigs), [1, 2, 1, 0]);
    }

    // The matrix mixer
    #[test]
    fn shows_delays_in_ms_or_metres_and_reads_them_back_in_ms() {
        assert_eq!(delay_in(1.234, Unit::Ms), "1.23");
        assert_eq!(delay_in(10., Unit::M), "3.43");
        assert_eq!(delay_from("3.43", Unit::M), Some(10.));
        assert_eq!(delay_from("0,25", Unit::Ms), Some(0.25));
        assert_eq!(delay_from("900", Unit::Ms), Some(500.));
        assert_eq!(delay_from("-1", Unit::Ms), Some(0.));
        assert_eq!(delay_from("", Unit::Ms), None);
        assert_eq!(delay_from("x", Unit::M), None);
    }

    #[test]
    fn shows_gains_without_trailing_zeros() {
        assert_eq!(gain_text(0.), "0");
        assert_eq!(gain_text(1.), "1");
        assert_eq!(gain_text(0.70794), "0.708");
        assert_eq!(gain_text(-0.5), "-0.5");
        assert_eq!(gain_text(10.), "10");
    }

    #[test]
    fn fields_within_range() {
        assert_eq!(num("13", -60., 12.), Some(12.));
        assert_eq!(num("-3,5", -60., 12.), Some(-3.5));
        assert_eq!(num(" ", -60., 12.), None);
    }

    #[test]
    fn presets() {
        assert_eq!(preset_label("Mine", false), "Mine");
        assert!(can_save_preset(" Mine ", &["stereo"]));
        assert!(!can_save_preset("stereo", &["stereo"]));
        assert!(!can_save_preset("  ", &[]));
    }

    // The Auracast tab of the Developer screen
    #[test]
    fn takes_a_matrix_rates_from_the_front_and_rear_rows_not_quality() {
        let c = Choices {
            quality: Quality::Sq,
            channels: Channels::Matrix,
            rear_quality: Quality::Sq,
            ..Choices::default()
        };
        let tone = source_params(&c.source, &[], &[], &DeviceNames::new());
        let Method::StreamStartBroadcast { quality, rear_quality, layout, .. } = c.start_request(tone) else {
            panic!("not a broadcast")
        };
        assert_eq!((quality.as_str(), rear_quality.as_deref()), ("hq", Some("sq")));
        assert_eq!(layout.as_deref(), Some("matrix_stereo_stereo"));
    }

    #[test]
    fn starts_a_broadcast_with_the_checked_name_and_code() {
        let mut c = Choices { name: "abc".into(), ..Choices::default() };
        assert_eq!(c.name_error(), Some(t!("ui.dev.auracast_err_length", 4, 32)));
        assert!(c.start_disabled(false));
        assert!(!c.start_disabled(true));
        c.name = "Living room".into();
        // Name and password are checked in bytes
        c.code = "ああああああ".into();
        assert_eq!(c.code_error(), Some(t!("ui.dev.auracast_err_length", 4, 16)));
        c.code = "secret".into();
        assert!(!c.start_disabled(false));
        c.quality = Quality::Sq;
        c.channels = Channels::Mono;
        assert_eq!(
            c.start_request(source_params(&c.source, &[], &[], &DeviceNames::new())),
            Method::StreamStartBroadcast {
                name: "Living room".into(),
                code: Some("secret".into()),
                quality: "sq".into(),
                rear_quality: None,
                mono: Some(true),
                split: None,
                layout: None,
                source: "tone".into(),
                source_id: None,
                source_name: None,
                volume: None,
                follow_volume: Some(false),
                receivers: None,
            }
        );
        // A matrix with both pairs off sends nothing
        let off = Choices { channels: Channels::Matrix, front: PairMode::Off, rear: PairMode::Off, ..c };
        assert!(off.start_disabled(false));
    }

    #[test]
    fn one_big_of_four_bis_starts_at_medium_quality() {
        let mut c = Choices::default();
        assert_eq!(c.quality, Quality::Hq);
        c.set_channels(Channels::Matrix);
        assert_eq!(c.quality, Quality::Hq);
        c.set_channels(Channels::Single);
        assert_eq!((c.channels, c.quality), (Channels::Single, Quality::Mq));
        // High chosen for it stays, and so does standard
        c.quality = Quality::Hq;
        c.set_channels(Channels::Single);
        assert_eq!(c.quality, Quality::Hq);
        let mut c = Choices { quality: Quality::Sq, ..Choices::default() };
        c.set_channels(Channels::Single);
        assert_eq!(c.quality, Quality::Sq);
    }

    #[test]
    fn a_pair_sent_separately_has_a_slot_per_speaker() {
        let keys = |c: &Choices| c.slots().iter().map(|s| s.key).collect::<Vec<_>>();
        let mut c = Choices {
            channels: Channels::Matrix,
            front: PairMode::Stereo,
            rear: PairMode::Stereo,
            arrangement: Arrangement::TwoStereo,
            ..Choices::default()
        };
        assert_eq!(keys(&c), ["FL+FR", "RL+RR"]);
        // Two broadcasts: no receiver plays both
        c.rear = PairMode::Separate;
        assert_eq!(keys(&c), ["FL+FR", "RL", "RR"]);
        c.front = PairMode::Separate;
        assert_eq!(keys(&c), ["FL", "FR", "RL", "RR"]);
        // Subgroups are one broadcast
        c.front = PairMode::Subgroups;
        assert_eq!(keys(&c), ["FL+FR", "RL", "RR"]);
        c.rear = PairMode::Off;
        assert_eq!(keys(&c), ["FL+FR"]);
        // One BIG: every output in the one broadcast, whatever the pairs say
        c.channels = Channels::Single;
        c.front = PairMode::Separate;
        assert_eq!(keys(&c), ["FL+FR", "RL+RR"]);
    }

    fn stream(state: StreamState, key: &str, profile: Option<&str>) -> StreamStatus {
        StreamStatus { state, message: Msg::new(key), profile: profile.map(str::to_owned), ..StreamStatus::default() }
    }

    #[test]
    fn keeps_a_broadcast_whose_adapter_stopped_working_while_it_starts_again() {
        let mut s = Session { mine: true, ..Session::default() };
        let on_air = stream(StreamState::Streaming, "status.broadcasting", None);
        s.follow(&on_air);
        assert!(s.on_air && s.live(&on_air));
        let again = stream(StreamState::Reconnecting, "status.reconnect_attempt", None);
        s.follow(&again);
        assert!(s.mine && s.live(&again) && s.locked(&again) && s.source_locked(&again));
        s.follow(&on_air);
        assert!(s.live(&on_air) && !s.source_locked(&on_air));
        // Stopped
        s.follow(&stream(StreamState::Idle, "status.idle", None));
        assert!(!s.mine);
    }

    #[test]
    fn an_idle_before_going_on_air_is_the_stream_before_ending() {
        let mut s = Session { mine: true, ..Session::default() };
        s.follow(&stream(StreamState::Idle, "status.idle", None));
        assert!(s.mine);
        // Another stream took over
        s.follow(&stream(StreamState::Streaming, "status.streaming", Some("Headphones")));
        assert!(!s.mine);
        // A profile's reconnect is not the broadcast
        let mut s = Session { mine: true, ..Session::default() };
        assert!(!s.live(&stream(StreamState::Connecting, "status.connecting", Some("Headphones"))));
        assert!(s.live(&stream(StreamState::Connecting, "status.connecting", None)));
        s.asking = true;
        s.follow(&stream(StreamState::Error, "error.btstack_init", None));
        assert!(s.mine && s.failed.is_none());
    }

    #[test]
    fn shows_why_a_broadcast_did_not_start() {
        let mut s = Session { mine: true, ..Session::default() };
        let failed = stream(StreamState::Error, "error.btstack_init", None);
        s.follow(&failed);
        assert!(!s.mine);
        assert_eq!(s.status(&failed), Some((t("error.btstack_init"), true)));
        assert_eq!(s.status(&stream(StreamState::Idle, "status.idle", None)), Some((t("error.btstack_init"), true)));
    }

    #[test]
    fn a_source_that_cannot_be_captured_shows_the_one_that_plays() {
        let none = DeviceNames::new();
        let params = |s: &Source| source_params(s, &[], &[], &none);
        let (tone, loopback) = (Source::Tone, Source::Loopback);
        let asked = (&loopback, &params(&loopback));
        let before = (&tone, &params(&tone));
        let settled = |live: bool, chosen: &Source, kind: &str| {
            settled_source(live, chosen, asked, before, Some(&info(kind, "")), &[], &[])
        };
        assert_eq!(settled(true, &loopback, "tone"), Some(Source::Tone));
        assert_eq!(settled(true, &loopback, "loopback"), None);
        assert_eq!(settled(false, &loopback, "tone"), None);
        assert_eq!(settled(true, &tone, "tone"), None);
        assert_eq!(settled(true, &loopback, "app"), None);
    }

    #[test]
    fn a_device_that_cannot_be_captured_goes_back_to_the_one_sent_before_not_its_twin() {
        let none = DeviceNames::new();
        let twins = [dev("{mic1}", "Microphone"), dev("{mic2}", "Microphone")];
        let (first, second) = (Source::Input("{mic1}".into()), Source::Input("{mic2}".into()));
        let sent_first = source_params(&first, &[], &twins, &none);
        let sent_second = source_params(&second, &[], &twins, &none);
        let mic = info("input", "Microphone");
        // From the second microphone to the tone, which did not start: the
        // second one plays on, though by name alone it would be the first
        let tone = Source::Tone;
        let sent_tone = source_params(&tone, &[], &twins, &none);
        let back = settled_source(true, &tone, (&tone, &sent_tone), (&second, &sent_second), Some(&mic), &[], &twins);
        assert_eq!(back, Some(second.clone()));
        // The device asked for plays (the name sent)
        let kept =
            settled_source(true, &second, (&second, &sent_second), (&first, &sent_first), Some(&mic), &[], &twins);
        assert_eq!(kept, None);
    }

    fn dev(id: &str, name: &str) -> AudioDevice {
        AudioDevice {
            id: id.into(),
            name: name.into(),
            short_name: None,
            adapter_name: None,
            is_default: false,
            is_virtual: false,
            is_display: false,
            mix_rate: None,
            mix_channels: None,
            mix_bits: None,
            capture_name: None,
            capture_rate: None,
        }
    }

    fn info(kind: &str, name: &str) -> SourceInfo {
        SourceInfo { kind: kind.into(), name: name.into(), waiting: false }
    }

    fn params(source: &str, id: Option<&str>, name: Option<&str>) -> SourceParams {
        SourceParams { source: source.into(), source_id: id.map(str::to_owned), source_name: name.map(str::to_owned) }
    }

    // Auracast sources (Auracast.test.ts)
    #[test]
    fn names_a_device_with_its_id_and_name() {
        let (outputs, inputs, none) = ([dev("{spk}", "Speakers")], [dev("{mic}", "Microphone")], DeviceNames::new());
        assert_eq!(source_params(&Source::Tone, &outputs, &inputs, &none), params("tone", None, None));
        assert_eq!(
            source_params(&Source::Output("{spk}".into()), &outputs, &inputs, &none),
            params("output", Some("{spk}"), Some("Speakers"))
        );
        assert_eq!(
            source_params(&Source::Input("{mic}".into()), &outputs, &inputs, &none),
            params("input", Some("{mic}"), Some("Microphone"))
        );
        // Gone: the core opens it by its id, under the name it had if it was listed
        let old = Source::Input("{old}".into());
        assert_eq!(source_params(&old, &outputs, &inputs, &none).source_name, None);
        let names = DeviceNames::from([("{old}".to_owned(), "Headset".to_owned())]);
        assert_eq!(source_params(&old, &outputs, &inputs, &names).source_name.as_deref(), Some("Headset"));
    }

    #[test]
    fn tells_what_was_sent_from_another_device_of_the_same_name() {
        let (outputs, inputs, none) = ([dev("{spk}", "Speakers")], [dev("{mic}", "Microphone")], DeviceNames::new());
        let twins = [dev("{mic1}", "Microphone"), dev("{mic2}", "Microphone")];
        let second = source_params(&Source::Input("{mic2}".into()), &outputs, &twins, &none);
        // By name alone it would be the first one
        assert_eq!(
            source_of_info(Some(&info("input", "Microphone")), &outputs, &twins),
            Some(Source::Input("{mic1}".into()))
        );
        assert!(plays_source(Some(&info("input", "Microphone")), &second));
        assert!(!plays_source(Some(&info("output", "Microphone")), &second));
        assert!(!plays_source(Some(&info("input", "Headset")), &second));
        let tone = source_params(&Source::Tone, &outputs, &twins, &none);
        assert!(plays_source(Some(&info("tone", "")), &tone));
        assert!(!plays_source(Some(&info("loopback", "")), &tone));
        // An older client's virtual device is an output; no name sent: the id shows
        let spk = source_params(&Source::Output("{spk}".into()), &outputs, &inputs, &none);
        assert!(plays_source(Some(&info("virtual", "Speakers")), &spk));
        let old = source_params(&Source::Input("{old}".into()), &outputs, &inputs, &none);
        assert!(plays_source(Some(&info("input", "{old}")), &old));
        assert!(!plays_source(None, &second));
    }

    #[test]
    fn finds_the_choice_of_what_plays() {
        let (outputs, inputs) = ([dev("{spk}", "Speakers")], [dev("{mic}", "Microphone")]);
        let of = |kind: &str, name: &str| source_of_info(Some(&info(kind, name)), &outputs, &inputs);
        assert_eq!(of("loopback", ""), Some(Source::Loopback));
        assert_eq!(of("output", "Speakers"), Some(Source::Output("{spk}".into())));
        assert_eq!(of("input", "Microphone"), Some(Source::Input("{mic}".into())));
        assert_eq!(of("input", "Headset"), None);
        assert_eq!(source_of_info(None, &outputs, &inputs), None);
    }

    #[test]
    fn lists_the_devices_under_their_headings_and_one_chosen_that_is_gone() {
        let (outputs, inputs, none) = ([dev("{spk}", "Speakers")], [dev("{mic}", "Microphone")], DeviceNames::new());
        let options = source_options(&Source::Tone, &outputs, &inputs, &none);
        let values: Vec<Source> = options.iter().map(|o| o.value.clone()).collect();
        assert_eq!(
            values,
            [Source::Tone, Source::Loopback, Source::Output("{spk}".into()), Source::Input("{mic}".into())]
        );
        assert_ne!(options[2].group, options[3].group);
        let old = Source::Input("{old}".into());
        let names = DeviceNames::from([("{old}".to_owned(), "Headset".to_owned())]);
        let gone = source_options(&old, &outputs, &inputs, &names);
        let last = gone.last().expect("an entry for the device gone");
        assert_eq!((&last.value, &last.group), (&old, &options[3].group));
        // By the name it had, never by its id
        assert!(last.label.contains("Headset"));
        let unnamed = source_options(&old, &outputs, &inputs, &none);
        assert!(!unnamed.last().expect("an entry").label.contains("{old}"));
    }

    #[test]
    fn follows_the_windows_volume_of_output_devices_only() {
        let sources = [Source::Tone, Source::Loopback, Source::Output("{spk}".into()), Source::Input("{mic}".into())];
        assert_eq!(sources.map(|s| followable(&s)), [false, true, true, false]);
    }

    #[test]
    fn starts_a_broadcast_with_the_device_and_follow() {
        let c = Choices { source: Source::Output("{spk}".into()), follow: true, ..Choices::default() };
        let sent = source_params(&c.source, &[dev("{spk}", "Speakers")], &[], &DeviceNames::new());
        let Method::StreamStartBroadcast { source, source_id, source_name, volume, follow_volume, .. } =
            c.start_request(sent)
        else {
            panic!("not a broadcast")
        };
        assert_eq!(
            (source.as_str(), source_id.as_deref(), source_name.as_deref()),
            ("output", Some("{spk}"), Some("Speakers"))
        );
        assert_eq!((volume, follow_volume), (None, Some(true)));
    }

    fn adapter(instance_id: &str, address: Option<&str>, driver: DriverKind) -> AdapterInfo {
        AdapterInfo {
            instance_id: instance_id.into(),
            vid: 0x8087,
            pid: 0x0033,
            display_name: "Intel AX211 (0x8087:0x0033)".into(),
            device_name: String::new(),
            vendor: "Intel".into(),
            driver,
            driver_service: String::new(),
            realtek_chip: None,
            firmware_present: None,
            firmware_files: vec![],
            firmware_invalid: None,
            firmware_download: false,
            firmware_from_driver: false,
            firmware_driver: false,
            vendor_note: None,
            restart_needed: false,
            le_audio: None,
            address: address.map(str::to_owned),
            bt_version: None,
            windows_name: None,
        }
    }

    #[test]
    fn chooses_the_adapter_for_broadcasts_by_name_and_mac_address() {
        let list = [
            adapter("USB\\A", Some("00:1A:7D:DA:71:13"), DriverKind::Winusb),
            adapter("USB\\B", None, DriverKind::Winusb),
            adapter("USB\\C", Some("11:22:33:44:55:66"), DriverKind::Bluetooth),
        ];
        let (value, options) = auracast_adapter_choices(&list, "");
        assert_eq!(value, "");
        let labels: Vec<&str> = options.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(labels[0], t("ui.dev.auracast_adapter_current"));
        assert!(labels[1].ends_with(" · 00:1A:7D:DA:71:13"));
        assert!(labels[2].ends_with(" · 8087:0033"));
        assert_eq!(labels.len(), 3);
        // Whatever its case; a chosen one that is not there
        assert_eq!(auracast_adapter_choices(&list, "usb\\b").0, "USB\\B");
        let (value, options) = auracast_adapter_choices(&list, " USB\\Z ");
        assert_eq!(value, "USB\\Z");
        assert_eq!(options.last().map(|(_, l)| l.clone()), Some(t("ui.dev.auracast_adapter_missing")));
        // Offered with several adapters, or one chosen
        assert!(show_adapter(&list, ""));
        assert!(!show_adapter(&list[..1], ""));
        assert!(show_adapter(&list[..1], "USB\\A"));
    }
}

//! AirPods: the battery of each bud and of the case, noise control and where
//! the buds are, over the earbuds' own control protocol on classic L2CAP
//! (issue #595). Windows' own Bluetooth never sees earbuds connected to
//! BlueCaster's adapter, so apps that read them there cannot.
//!
//! Written from a fact sheet on the AirPods control protocol
//! (docs/clean-room.md, "Exception: headphone control messages").
//!
//! One L2CAP packet is one message: a 4-byte header (kind, 0, service 4,
//! 0), and for a command its opcode (2 bytes, little endian) and payload.
//! The host connects (kind 0), tells what it can (0x004D) and asks for
//! notifications (0x000F); the earbuds then report the battery (0x0004),
//! where the buds are (0x0006) and settings (0x0009) when they change.
//! Much of it is known from a few models only, so what does not parse is
//! skipped and counted, never fatal.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use bt_host::{BtHost, L2capCloseReason, L2capError, L2capEvent};
use ipc::{BudBattery, BudsStatus, EarState, NoiseMode};

use crate::service::ServiceEvent;

/// The control protocol's PSM
pub const PSM: u16 = 0x1001;

const KIND_CONNECT: u8 = 0x00;
const KIND_CONNECTED: u8 = 0x01;
const KIND_COMMAND: u8 = 0x04;
const SERVICE: u8 = 0x04;

const OP_BATTERY: u16 = 0x0004;
const OP_EAR: u16 = 0x0006;
const OP_SETTING: u16 = 0x0009;
const OP_NOTIFY: u16 = 0x000F;
const OP_DEVICE_INFO: u16 = 0x001D;
const OP_HOST_FEATURES: u16 = 0x004D;

/// Setting 0x0D: the listening mode (1 off, 2 noise cancellation,
/// 3 transparency, 4 adaptive)
const SETTING_LISTENING_MODE: u8 = 0x0D;

/// Connect: version 1.2, no features
const CONNECT: [u8; 16] = [KIND_CONNECT, 0, SERVICE, 0, 0x01, 0x00, 0x02, 0x00, 0, 0, 0, 0, 0, 0, 0, 0];
/// What the host can do: the value a computer of the earbuds' own vendor
/// sends (unknown bits; without it Adaptive may be refused)
const HOST_FEATURES: [u8; 8] = [0xFF, 0, 0, 0, 0, 0, 0, 0];
/// Every notification
const NOTIFY_ALL: [u8; 4] = [0xFF; 4];

// Battery components and states
const PART_SINGLE: u8 = 0x01;
const PART_RIGHT: u8 = 0x02;
const PART_LEFT: u8 = 0x04;
const PART_CASE: u8 = 0x08;
const POWER_CHARGING: u8 = 0x01;
const POWER_ABSENT: u8 = 0x04;

/// From the stream's start to opening the channel: earbuds may give up the
/// audio set-up when it opens during it (not confirmed)
const OPEN_DELAY: Duration = Duration::from_secs(2);
/// How long the connect answer is waited for before the rest is sent
/// anyway (not every host waits for it)
const CONNECT_WAIT: Duration = Duration::from_millis(500);
/// Between a failed open or a closed channel and the next open
const RETRY_GAP: Duration = Duration::from_secs(5);
/// Opens tried per stream (the earbuds' own open is taken at any time)
const MAX_OPENS: u32 = 3;
/// How long the earbuds have to confirm a listening mode
const ECHO_TIMEOUT: Duration = Duration::from_secs(3);

/// Earbuds of this protocol, by their name
pub fn is_airpods(device_name: &str) -> bool {
    device_name.to_lowercase().contains("airpods")
}

fn command(opcode: u16, payload: &[u8]) -> Vec<u8> {
    let mut p = vec![KIND_COMMAND, 0, SERVICE, 0];
    p.extend(opcode.to_le_bytes());
    p.extend(payload);
    p
}

fn noise_setting(mode: NoiseMode) -> Vec<u8> {
    command(OP_SETTING, &[SETTING_LISTENING_MODE, noise_value(mode), 0, 0, 0])
}

fn noise_value(mode: NoiseMode) -> u8 {
    match mode {
        NoiseMode::Off => 0x01,
        NoiseMode::NoiseCancellation => 0x02,
        NoiseMode::Transparency => 0x03,
        NoiseMode::Adaptive => 0x04,
    }
}

fn noise_mode(value: u8) -> Option<NoiseMode> {
    match value {
        0x01 => Some(NoiseMode::Off),
        0x02 => Some(NoiseMode::NoiseCancellation),
        0x03 => Some(NoiseMode::Transparency),
        0x04 => Some(NoiseMode::Adaptive),
        _ => None,
    }
}

fn ear_state(value: u8) -> Option<EarState> {
    match value {
        0x00 => Some(EarState::InEar),
        0x01 => Some(EarState::OutOfEar),
        // 3: seen as a closed case, not confirmed
        0x02 | 0x03 => Some(EarState::InCase),
        _ => None,
    }
}

/// One battery entry: component, level, state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Cell {
    part: u8,
    level: u8,
    state: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Packet {
    Connected {
        status: u16,
        minor: u16,
    },
    Battery(Vec<Cell>),
    /// The bud that holds the link first
    Ear(Vec<EarState>),
    Setting {
        id: u8,
        value: u8,
    },
    /// Its model number (the name before it is the user's: not kept)
    DeviceInfo {
        model: Option<String>,
    },
    Other(u16),
}

fn u16_at(p: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*p.get(i)?, *p.get(i + 1)?]))
}

fn parse(p: &[u8]) -> Option<Packet> {
    let [kind, _, service, _, rest @ ..] = p else { return None };
    if *service != SERVICE {
        return None;
    }
    match *kind {
        KIND_CONNECTED => Some(Packet::Connected { status: u16_at(rest, 0)?, minor: u16_at(rest, 4).unwrap_or(0) }),
        KIND_COMMAND => {
            let opcode = u16_at(rest, 0)?;
            let payload = &rest[2..];
            Some(match opcode {
                OP_BATTERY => Packet::Battery(battery_cells(payload)),
                OP_EAR => Packet::Ear(payload.iter().take(2).filter_map(|&v| ear_state(v)).collect()),
                OP_SETTING => Packet::Setting { id: *payload.first()?, value: *payload.get(1)? },
                OP_DEVICE_INFO => Packet::DeviceInfo { model: model_number(payload) },
                other => Packet::Other(other),
            })
        }
        _ => None,
    }
}

/// A count, then that many entries of 5 bytes (component, 1, level, state,
/// 1); an entry cut short ends the list
fn battery_cells(payload: &[u8]) -> Vec<Cell> {
    let Some((&count, entries)) = payload.split_first() else { return Vec::new() };
    entries
        .as_chunks::<5>()
        .0
        .iter()
        .take(usize::from(count))
        .map(|e| Cell { part: e[0], level: e[2], state: e[3] })
        .collect()
}

/// After 5 bytes of unknown meaning, NUL-terminated strings: the name, then
/// the model number
fn model_number(payload: &[u8]) -> Option<String> {
    let strings = payload.get(5..)?;
    let model = strings.split(|&b| b == 0).nth(1)?;
    let model = std::str::from_utf8(model).ok()?.trim();
    // A model number is short and plain (A2084); anything else is not one
    (!model.is_empty() && model.len() <= 16 && model.bytes().all(|b| b.is_ascii_alphanumeric()))
        .then(|| model.to_owned())
}

/// The listening modes a model offers, as far as it is known
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Modes {
    /// None known (yet): no noise control unless the earbuds report a mode
    Unknown,
    /// Off, noise cancellation, transparency
    Basic,
    /// And Adaptive
    WithAdaptive,
}

impl Modes {
    /// By the name the user sees, until the earbuds tell their model
    fn by_name(name: &str) -> Modes {
        let name = name.to_lowercase();
        if name.contains("airpods pro") {
            Modes::WithAdaptive
        } else if name.contains("airpods max") {
            Modes::Basic
        } else {
            Modes::Unknown
        }
    }

    /// By the model number of the device info: only the ones known
    fn by_model(model: &str) -> Option<Modes> {
        match model {
            // AirPods Pro (1st generation)
            "A2084" => Some(Modes::Basic),
            // AirPods Pro 2 (USB-C)
            "A3048" => Some(Modes::WithAdaptive),
            _ => None,
        }
    }

    fn list(self) -> Vec<NoiseMode> {
        match self {
            Modes::Unknown => Vec::new(),
            Modes::Basic => vec![NoiseMode::Off, NoiseMode::NoiseCancellation, NoiseMode::Transparency],
            Modes::WithAdaptive => {
                vec![NoiseMode::Off, NoiseMode::NoiseCancellation, NoiseMode::Transparency, NoiseMode::Adaptive]
            }
        }
    }
}

/// What the earbuds told, and what of it the app shows
struct Buds {
    status: BudsStatus,
    modes: Modes,
    /// Adaptive was asked for and another mode came back: not offered again
    no_adaptive: bool,
    /// A listening mode asked for, until the earbuds confirm one
    asked: Option<(NoiseMode, Instant)>,
    /// For the log, which counts what came
    counts: Counts,
}

#[derive(Debug, Default)]
struct Counts {
    packets: u32,
    battery: u32,
    ear: u32,
    modes: u32,
    other: u32,
    unparsed: u32,
}

impl Buds {
    fn new(name: &str) -> Buds {
        let modes = Modes::by_name(name);
        let mut b =
            Buds { status: BudsStatus::default(), modes, no_adaptive: false, asked: None, counts: Counts::default() };
        b.update_modes();
        b
    }

    fn update_modes(&mut self) {
        let mut list = self.modes.list();
        if self.no_adaptive {
            list.retain(|m| *m != NoiseMode::Adaptive);
        }
        self.status.noise_modes = list;
    }

    fn on_packet(&mut self, data: &[u8]) {
        self.counts.packets += 1;
        let Some(packet) = parse(data) else {
            self.counts.unparsed += 1;
            return;
        };
        match packet {
            Packet::Connected { status, minor } => {
                tracing::info!("AirPods: connected (status {status}, version 1.{minor})");
            }
            Packet::Battery(cells) => {
                self.counts.battery += 1;
                self.on_battery(&cells);
            }
            Packet::Ear(states) => {
                self.counts.ear += 1;
                self.status.ear = states;
            }
            Packet::Setting { id: SETTING_LISTENING_MODE, value } => {
                self.counts.modes += 1;
                match noise_mode(value) {
                    Some(mode) => self.on_mode(mode),
                    None => tracing::info!("AirPods: listening mode {value} not known"),
                }
            }
            Packet::Setting { .. } => self.counts.other += 1,
            Packet::DeviceInfo { model } => {
                let known = model.as_deref().and_then(Modes::by_model);
                tracing::info!(
                    "AirPods: model {} ({})",
                    model.as_deref().unwrap_or("not given"),
                    if known.is_some() { "known" } else { "not known, its modes as reported" }
                );
                if let Some(m) = known {
                    self.modes = m;
                    self.update_modes();
                }
            }
            Packet::Other(_) => self.counts.other += 1,
        }
    }

    fn on_battery(&mut self, cells: &[Cell]) {
        let mut single = None;
        for c in cells {
            let power = (c.state != POWER_ABSENT && c.level <= 100)
                .then_some(BudBattery { percent: c.level, charging: c.state == POWER_CHARGING });
            match c.part {
                PART_LEFT => self.status.left = power,
                PART_RIGHT => self.status.right = power,
                PART_CASE => self.status.case = power,
                PART_SINGLE => single = power,
                _ => {}
            }
        }
        // One battery (headphones): shown as both sides
        if let Some(s) = single {
            self.status.left = Some(s);
            self.status.right = Some(s);
        }
    }

    fn on_mode(&mut self, mode: NoiseMode) {
        if let Some((asked, _)) = self.asked.take()
            && asked != mode
        {
            tracing::info!("AirPods: asked for {asked:?}, they keep {mode:?}");
            if asked == NoiseMode::Adaptive {
                self.no_adaptive = true;
            }
        }
        // A mode reported: they have noise control, whatever the name said
        if self.modes == Modes::Unknown {
            self.modes = Modes::Basic;
        }
        if mode == NoiseMode::Adaptive {
            self.modes = Modes::WithAdaptive;
            self.no_adaptive = false;
        }
        self.update_modes();
        self.status.noise = Some(mode);
    }

    /// The deadline of a mode asked for passed
    fn check_asked(&mut self, now: Instant) {
        if let Some((mode, until)) = self.asked
            && now >= until
        {
            tracing::info!("AirPods: no answer to {mode:?}");
            self.asked = None;
        }
    }
}

enum Msg {
    Channel(u32, L2capEvent),
    Noise(NoiseMode),
    Stop,
}

/// The control channel's thread while a stream to AirPods runs. Dropping
/// it ends the thread, which closes the channel.
pub struct Session {
    tx: Sender<Msg>,
    status: Arc<std::sync::Mutex<Option<BudsStatus>>>,
}

impl Session {
    /// Starts the thread for `device`, the device being streamed to;
    /// `name` is its name (for the listening modes, never logged). What it
    /// learns goes to `events` as [`ServiceEvent::Buds`].
    pub fn start(host: Arc<BtHost>, device: [u8; 6], name: &str, events: Sender<ServiceEvent>) -> Option<Session> {
        let (tx, rx) = mpsc::channel();
        let status = Arc::new(std::sync::Mutex::new(None));
        let buds = Buds::new(name);
        let thread_tx = tx.clone();
        let thread_status = status.clone();
        let spawned = std::thread::Builder::new().name("airpods".into()).spawn(move || {
            run(&host, device, buds, &events, &rx, &thread_tx, &thread_status);
        });
        match spawned {
            Ok(_) => Some(Session { tx, status }),
            Err(e) => {
                tracing::warn!("AirPods: no thread for the control channel ({e})");
                None
            }
        }
    }

    /// Asks for a listening mode; false: not one the earbuds offer now
    pub fn set_noise(&self, mode: NoiseMode) -> bool {
        let offered = self
            .status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|s| s.noise_modes.contains(&mode));
        offered && self.tx.send(Msg::Noise(mode)).is_ok()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Stop);
    }
}

fn run(
    host: &BtHost,
    device: [u8; 6],
    mut buds: Buds,
    events: &Sender<ServiceEvent>,
    rx: &Receiver<Msg>,
    tx: &Sender<Msg>,
    shared: &std::sync::Mutex<Option<BudsStatus>>,
) {
    let sink = |tx: &Sender<Msg>| -> bt_host::L2capSink {
        let tx = tx.clone();
        Box::new(move |id, e| tx.send(Msg::Channel(id, e)).is_ok())
    };
    // Earbuds that open the channel themselves get it at any time
    let accept_tx = tx.clone();
    let listener = host
        .l2cap_listen(device, PSM, Box::new(move || Some(sink(&accept_tx))))
        .inspect_err(|e| tracing::info!("AirPods: their own open is not taken ({e:?})"))
        .ok();

    let mut channel: Option<u32> = None;
    // Connected, the rest not sent yet: until when the answer is waited for
    let mut connecting: Option<Instant> = None;
    let mut opens = 0u32;
    let mut next_open = Some(Instant::now() + OPEN_DELAY);
    let mut sent: Option<BudsStatus> = None;
    let publish = |buds: &Buds, sent: &mut Option<BudsStatus>, open: bool| {
        let now = open.then(|| buds.status.clone());
        if *sent != now {
            *shared.lock().unwrap_or_else(|e| e.into_inner()) = now.clone();
            let _ = events.send(ServiceEvent::Buds(now.clone()));
            *sent = now;
        }
    };

    loop {
        let now = Instant::now();
        buds.check_asked(now);
        if channel.is_none()
            && let Some(at) = next_open
            && now >= at
        {
            opens += 1;
            next_open = None;
            match host.l2cap_open(device, PSM, sink(tx)) {
                Ok(o) => {
                    tracing::info!("AirPods: control channel open");
                    channel = Some(o.id);
                    connecting = start(host, o.id);
                }
                Err(L2capError::NotConnected) => {
                    tracing::info!("AirPods: no connection for the control channel");
                    break;
                }
                Err(e) => {
                    tracing::info!("AirPods: control channel not opened ({e:?}, try {opens} of {MAX_OPENS})");
                    if opens < MAX_OPENS {
                        next_open = Some(Instant::now() + RETRY_GAP);
                    }
                }
            }
        }
        if let Some(until) = connecting
            && Instant::now() >= until
            && let Some(id) = channel
        {
            connecting = None;
            register(host, id);
        }
        let wake = [connecting, next_open.filter(|_| channel.is_none()), buds.asked.map(|(_, t)| t)]
            .into_iter()
            .flatten()
            .min();
        let wait = wake.map_or(Duration::from_secs(3600), |t| t.saturating_duration_since(Instant::now()));
        let msg = match rx.recv_timeout(wait) {
            Ok(m) => m,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        match msg {
            Msg::Stop => break,
            Msg::Noise(mode) => match channel {
                Some(id) if host.l2cap_send(id, noise_setting(mode)).wait().is_ok() => {
                    tracing::info!("AirPods: asking for {mode:?}");
                    buds.asked = Some((mode, Instant::now() + ECHO_TIMEOUT));
                }
                _ => tracing::info!("AirPods: {mode:?} not sent, no control channel"),
            },
            Msg::Channel(id, L2capEvent::Opened(_)) => {
                if channel.is_none() {
                    tracing::info!("AirPods: they opened the control channel");
                    channel = Some(id);
                    connecting = start(host, id);
                } else {
                    let _ = host.l2cap_close(id).wait();
                }
            }
            Msg::Channel(id, L2capEvent::Data(data)) if channel == Some(id) => {
                let was_connecting = connecting.is_some();
                buds.on_packet(&data);
                // The connect answer came: the rest at once
                if was_connecting && matches!(parse(&data), Some(Packet::Connected { .. })) {
                    connecting = None;
                    register(host, id);
                }
                publish(&buds, &mut sent, true);
            }
            Msg::Channel(id, L2capEvent::Closed(reason)) if channel == Some(id) => {
                channel = None;
                connecting = None;
                buds.asked = None;
                publish(&buds, &mut sent, false);
                tracing::info!("AirPods: control channel closed ({reason:?}), {:?}", buds.counts);
                if reason != L2capCloseReason::Remote {
                    break;
                }
                if opens < MAX_OPENS {
                    next_open = Some(Instant::now() + RETRY_GAP);
                }
            }
            // Of a channel closed already
            Msg::Channel(..) => {}
        }
    }
    if let Some(id) = channel {
        let _ = host.l2cap_close(id).wait();
    }
    if let Some(l) = listener {
        let _ = host.l2cap_unlisten(l).wait();
    }
    publish(&buds, &mut sent, false);
    tracing::info!("AirPods: control ended, {:?}", buds.counts);
}

/// Sends the connect; returns until when its answer is waited for
fn start(host: &BtHost, id: u32) -> Option<Instant> {
    if host.l2cap_send(id, CONNECT.to_vec()).wait().is_err() {
        tracing::info!("AirPods: connect not sent");
        return None;
    }
    Some(Instant::now() + CONNECT_WAIT)
}

/// What the host can do, then the notifications
fn register(host: &BtHost, id: u32) {
    let features = host.l2cap_send(id, command(OP_HOST_FEATURES, &HOST_FEATURES)).wait();
    let notify = host.l2cap_send(id, command(OP_NOTIFY, &NOTIFY_ALL)).wait();
    tracing::info!(
        "AirPods: notifications asked for (sent: features {}, notifications {})",
        features.is_ok(),
        notify.is_ok()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace().map(|b| u8::from_str_radix(b, 16).unwrap()).collect()
    }

    #[test]
    fn packets_sent() {
        assert_eq!(CONNECT.to_vec(), hex("00 00 04 00 01 00 02 00 00 00 00 00 00 00 00 00"));
        assert_eq!(command(OP_HOST_FEATURES, &HOST_FEATURES), hex("04 00 04 00 4D 00 FF 00 00 00 00 00 00 00"));
        assert_eq!(command(OP_NOTIFY, &NOTIFY_ALL), hex("04 00 04 00 0F 00 FF FF FF FF"));
        assert_eq!(noise_setting(NoiseMode::Transparency), hex("04 00 04 00 09 00 0D 03 00 00 00"));
        assert_eq!(noise_setting(NoiseMode::Adaptive), hex("04 00 04 00 09 00 0D 04 00 00 00"));
    }

    #[test]
    fn connect_answer() {
        let p = parse(&hex("01 00 04 00 00 00 01 00 02 00 05 00 49 4E 05 00 A5 4F")).unwrap();
        assert_eq!(p, Packet::Connected { status: 0, minor: 2 });
        // Shorter than known: still an answer
        assert_eq!(parse(&hex("01 00 04 00 00 00")), Some(Packet::Connected { status: 0, minor: 0 }));
    }

    #[test]
    fn battery_in_either_order() {
        let mut b = Buds::new("AirPods Pro");
        b.on_packet(&hex("04 00 04 00 04 00 03 02 01 50 02 01 04 01 4B 01 01 08 01 1E 02 01"));
        assert_eq!(b.status.right, Some(BudBattery { percent: 80, charging: false }));
        assert_eq!(b.status.left, Some(BudBattery { percent: 75, charging: true }));
        assert_eq!(b.status.case, Some(BudBattery { percent: 30, charging: false }));
        assert_eq!(ipc::buds_level(&b.status), Some(75));
        // Left first, the case gone
        b.on_packet(&hex("04 00 04 00 04 00 03 04 01 64 02 01 02 01 63 02 01 08 01 00 04 01"));
        assert_eq!(b.status.left.unwrap().percent, 100);
        assert_eq!(b.status.right.unwrap().percent, 99);
        assert_eq!(b.status.case, None);
        assert_eq!(ipc::buds_level(&b.status), Some(99));
    }

    #[test]
    fn battery_tolerates_odd_packets() {
        let mut b = Buds::new("AirPods");
        // The count says 3, two entries came; a level above 100 is no level
        b.on_packet(&hex("04 00 04 00 04 00 03 02 01 C8 02 01 04 01 32 02 01 08 01"));
        assert_eq!(b.status.right, None);
        assert_eq!(b.status.left.unwrap().percent, 50);
        assert_eq!(b.status.case, None);
        // One battery: both sides
        b.on_packet(&hex("04 00 04 00 04 00 01 01 01 2A 02 01"));
        assert_eq!(ipc::buds_level(&b.status), Some(42));
        assert_eq!(battery_cells(&[]), Vec::new());
    }

    #[test]
    fn ear_states() {
        let mut b = Buds::new("AirPods");
        b.on_packet(&hex("04 00 04 00 06 00 00 01"));
        assert_eq!(b.status.ear, vec![EarState::InEar, EarState::OutOfEar]);
        b.on_packet(&hex("04 00 04 00 06 00 02 03"));
        assert_eq!(b.status.ear, vec![EarState::InCase, EarState::InCase]);
    }

    #[test]
    fn modes_by_name_and_model() {
        assert_eq!(Buds::new("AirPods Pro").status.noise_modes.len(), 4);
        assert_eq!(Buds::new("AirPods Max").status.noise_modes.len(), 3);
        assert!(Buds::new("AirPods").status.noise_modes.is_empty());
        // The model number corrects the name
        let mut b = Buds::new("AirPods Pro");
        let info = [&hex("04 00 04 00 1D 00 01 B2 00 08 00")[..], b"My buds\0A2084\0Apple Inc.\0"].concat();
        assert_eq!(parse(&info), Some(Packet::DeviceInfo { model: Some("A2084".into()) }));
        b.on_packet(&info);
        assert!(!b.status.noise_modes.contains(&NoiseMode::Adaptive));
        // No model in it
        assert_eq!(model_number(&hex("01 02")), None);
    }

    #[test]
    fn a_reported_mode_shows_noise_control() {
        let mut b = Buds::new("AirPods");
        b.on_packet(&hex("04 00 04 00 09 00 0D 02 00 00 00"));
        assert_eq!(b.status.noise, Some(NoiseMode::NoiseCancellation));
        assert_eq!(b.status.noise_modes.len(), 3);
        b.on_packet(&hex("04 00 04 00 09 00 0D 04 00 00 00"));
        assert_eq!(b.status.noise, Some(NoiseMode::Adaptive));
        assert!(b.status.noise_modes.contains(&NoiseMode::Adaptive));
        // An unknown value changes nothing
        b.on_packet(&hex("04 00 04 00 09 00 0D 09 00 00 00"));
        assert_eq!(b.status.noise, Some(NoiseMode::Adaptive));
    }

    #[test]
    fn adaptive_refused_is_not_offered_again() {
        let mut b = Buds::new("AirPods Pro");
        b.asked = Some((NoiseMode::Adaptive, Instant::now() + ECHO_TIMEOUT));
        b.on_packet(&hex("04 00 04 00 09 00 0D 02 00 00 00"));
        assert_eq!(b.status.noise, Some(NoiseMode::NoiseCancellation));
        assert!(!b.status.noise_modes.contains(&NoiseMode::Adaptive));
        assert!(b.asked.is_none());
    }

    #[test]
    fn junk_is_counted_not_fatal() {
        let mut b = Buds::new("AirPods");
        for p in
            [hex(""), hex("04 00"), hex("04 00 05 00 04 00 01"), hex("09 00 04 00 01 00"), hex("04 00 04 00 09 00")]
        {
            b.on_packet(&p);
        }
        assert_eq!(b.counts.unparsed, 5);
        b.on_packet(&hex("04 00 04 00 2B 00 01 02"));
        assert_eq!(b.counts.other, 1);
    }

    #[test]
    fn airpods_names() {
        assert!(is_airpods("AirPods Pro"));
        assert!(is_airpods("Kim's AirPods Max"));
        assert!(!is_airpods("WH-1000XM4"));
    }
}

//! The Auracast assistant: while a broadcast runs, tells receivers with a
//! Broadcast Audio Scan Service (BASS) to play it, each its own BIS, and
//! takes it off them again when it stops.
//!
//! Each receiver is found and connected bare (SM and GATT only: no PACS /
//! ASCS, no VCP or CSIP, so it keeps its own volume), in turn with the
//! others (BTstack sets up one LE connection at a time). Once all are
//! connected, or have failed, all are sent Add Source together, so that
//! they start at about the same time. Then their Broadcast Receive States
//! are followed (and a Broadcast_Code given when one asks) until the stop.
//! A source left on a receiver from before (another assistant, an earlier
//! run) is taken off first, so that it plays only this broadcast.
//!
//! [`receivers`] is what each receiver does now, for the app; `out` (given
//! to [`Assistants::start`]) gets a line for each step, for the command
//! line and the log.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use bt_host::{BtHost, LeDeviceId};
use le_audio::bass;
use le_audio::broadcast::{BroadcastGroup, BroadcastLayout};
use le_audio::plan::ChannelSource;

/// How long a receiver is looked for
const FIND_TIMEOUT: Duration = Duration::from_secs(15);
/// How long its connection may take
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// How long the broadcast may take to come up
const BROADCAST_TIMEOUT: Duration = Duration::from_secs(45);
/// Broadcast Receive State PA_Sync_State: synchronized to the PA
const PA_SYNCHRONIZED: u8 = 2;
/// PA_Sync_State: failed to synchronize to the PA
const PA_SYNC_FAILED: u8 = 3;
/// BIG_Encryption: the Broadcast_Code given is wrong
const BIG_BAD_CODE: u8 = 3;

/// One receiver to tell
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    /// "XX:XX:XX:XX:XX:XX", the address it advertises with (its identity)
    pub address: String,
    pub name: String,
    /// 0-based, of the layout's broadcasts
    pub broadcast: usize,
    /// BIS indexes (1-based) in that broadcast's BIG; empty: the receiver picks
    pub bis: Vec<u8>,
    pub code: Option<String>,
    /// Close the link once the receiver plays (it keeps the broadcast; the
    /// source is not taken off at the stop): no LE connection beside the
    /// BIG while it runs
    pub leave: bool,
}

/// With [`Job::leave`]: how long a receiver plays all the BIS it was asked
/// before its link closes
const LEAVE_AFTER: Duration = Duration::from_secs(3);
/// With [`Job::leave`]: how long a receiver that reports other BIS than
/// those asked (some report BIS 1, 2 whatever is asked) plays before its
/// link closes
const LEAVE_AFTER_OTHER_BIS: Duration = Duration::from_secs(10);

/// A receiver that played and lost its BIS (the BIG set up again after slow
/// sending: some sync to the new one by themselves, some never do) while it
/// keeps the periodic advertising: told again after this long without BIS,
/// and again as long after each try
const RESYNC_AFTER: Duration = Duration::from_secs(3);
/// Tries per loss (see [`Resync`]); then it is left as it is
const RESYNC_TRIES: u32 = 3;

/// How a receiver that lost its BIS is told to play them again
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Resync {
    /// Modify Source with the BIS asked
    Modify,
    /// Modify Source with no BIS, then with the BIS asked
    Toggle,
    /// Remove Source and Add Source again
    ReAdd,
}

/// The try due for a receiver without BIS for `lost_for`, `tries` made for
/// this loss, the last `since_try` ago; None: nothing (yet)
fn resync_due(lost_for: Duration, tries: u32, since_try: Option<Duration>) -> Option<Resync> {
    if lost_for < RESYNC_AFTER || tries >= RESYNC_TRIES || since_try.is_some_and(|t| t < RESYNC_AFTER) {
        return None;
    }
    Some(match tries {
        0 => Resync::Modify,
        1 => Resync::Toggle,
        _ => Resync::ReAdd,
    })
}

/// An output of a layout by the name the API gives it: "FL" (also "L"),
/// "FR" ("R"), "RL", "RR", "SUBL" / "SUBR" (a 2.1 broadcast's subwoofer),
/// "SUB" (a mono subwoofer), "C" (a 3.1 broadcast's centre),
/// "M"
pub fn channel_source(name: &str) -> Option<ChannelSource> {
    Some(match name.trim().to_ascii_uppercase().as_str() {
        "FL" | "L" => ChannelSource::Left,
        "FR" | "R" => ChannelSource::Right,
        "RL" | "BL" => ChannelSource::RearLeft,
        "RR" | "BR" => ChannelSource::RearRight,
        "SUBL" => ChannelSource::SubLeft,
        "SUBR" => ChannelSource::SubRight,
        "SUB" | "LFE" => ChannelSource::SubMono,
        "C" | "FC" => ChannelSource::Center,
        "M" | "MONO" => ChannelSource::Mono,
        _ => return None,
    })
}

/// The job of a receiver that plays `channels` of `layout`: the broadcast
/// of the first of them that the layout has, and the BIS of all of them in
/// it. Err: why not (an unknown channel, none in the layout, channels in
/// two broadcasts).
pub fn job_for(
    layout: BroadcastLayout,
    address: &str,
    name: &str,
    channels: &[String],
    code: Option<String>,
) -> Result<Job, String> {
    let map = layout.bis_map();
    let mut broadcast = None;
    let mut bis = Vec::new();
    for c in channels {
        let source = channel_source(c).ok_or_else(|| format!("unknown channel \"{c}\""))?;
        let Some(&(g, b, _)) = map.iter().find(|(_, _, s)| *s == source) else {
            return Err(format!("layout {} has no {c}", layout.as_str()));
        };
        match broadcast {
            None => broadcast = Some(g),
            Some(first) if first != g => {
                return Err(format!("{} and {c} are in two broadcasts of layout {}", channels[0], layout.as_str()));
            }
            Some(_) => {}
        }
        if !bis.contains(&b) {
            bis.push(b);
        }
    }
    let Some(broadcast) = broadcast else { return Err("no channel".into()) };
    bis.sort_unstable();
    Ok(Job { address: address.trim().to_ascii_uppercase(), name: name.to_string(), broadcast, bis, code, leave: false })
}

/// What a receiver does now
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for its turn (another receiver is being connected)
    Waiting,
    Looking,
    Connecting,
    /// Connected, waiting for the others and the broadcast
    Ready,
    /// Add Source sent, not synchronized yet
    Added,
    /// Synchronized to the broadcast's periodic advertising, no BIS yet
    Synced,
    /// Synchronized to BIS
    Playing,
    /// Taken off at the stop
    Done,
    NotFound,
    NoBass,
    Failed,
}

impl Phase {
    /// The name in the API
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Waiting => "waiting",
            Phase::Looking => "looking",
            Phase::Connecting => "connecting",
            Phase::Ready => "ready",
            Phase::Added => "added",
            Phase::Synced => "synced",
            Phase::Playing => "playing",
            Phase::Done => "done",
            Phase::NotFound => "not_found",
            Phase::NoBass => "no_bass",
            Phase::Failed => "failed",
        }
    }
}

/// A receiver of the broadcast and what it does
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receiver {
    pub address: String,
    pub name: String,
    /// The BIS asked for (1-based, in its broadcast)
    pub asked: Vec<u8>,
    pub phase: Phase,
    /// The BIS it reports synchronized to
    pub bis: Vec<u8>,
    /// Why it failed, or what it reports besides (an error, a code asked for)
    pub detail: String,
    /// Its battery level (%), read once connected when it has a Battery
    /// Service
    pub battery: Option<u8>,
}

/// The receivers of the last [`Assistants::start`], and its number: the
/// assistants of a broadcast before it may still run (a stop does not wait
/// for one that looks for its receiver), and their receivers are no longer
/// these
static RECEIVERS: Mutex<(u64, Vec<Receiver>)> = Mutex::new((0, Vec::new()));

/// The receivers of the broadcast running (or of the last one)
pub fn receivers() -> Vec<Receiver> {
    RECEIVERS.lock().unwrap_or_else(|e| e.into_inner()).1.clone()
}

/// A receiver's place in [`RECEIVERS`]: the start it belongs to and its index
#[derive(Debug, Clone, Copy)]
struct At {
    run: u64,
    index: usize,
}

/// Changes the receiver at `at`, unless a newer start has replaced the list
fn update(at: At, f: impl FnOnce(&mut Receiver)) {
    let mut list = RECEIVERS.lock().unwrap_or_else(|e| e.into_inner());
    if list.0 == at.run
        && let Some(r) = list.1.get_mut(at.index)
    {
        f(r);
    }
}

fn set_phase(i: At, phase: Phase, detail: &str) {
    update(i, |r| {
        r.phase = phase;
        if !detail.is_empty() || matches!(phase, Phase::Ready | Phase::Added | Phase::Synced | Phase::Playing) {
            r.detail = detail.to_string();
        }
    });
}

fn set_bis(i: At, bis: Vec<u8>) {
    update(i, |r| r.bis = bis);
}

/// Where a step is told: a line each
pub type Out = Arc<dyn Fn(&str) + Send + Sync>;

/// The assistants of one broadcast
pub struct Assistants {
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl Assistants {
    /// One thread per job; `host` gives the host running now. Each thread
    /// waits for the host that broadcasts: the stream's start may replace
    /// the one running before (a broadcast on the Auracast adapter, a stack
    /// restarted after sleep or for changed settings), and a receiver looked
    /// for on that one is never found.
    pub fn start(
        host: impl Fn() -> Option<Arc<BtHost>> + Send + Sync + 'static,
        jobs: Vec<Job>,
        out: Out,
    ) -> Assistants {
        let stop = Arc::new(AtomicBool::new(false));
        let run = {
            let mut list = RECEIVERS.lock().unwrap_or_else(|e| e.into_inner());
            list.0 += 1;
            list.1 = jobs
                .iter()
                .map(|j| Receiver {
                    address: j.address.clone(),
                    name: j.name.clone(),
                    asked: j.bis.clone(),
                    phase: Phase::Waiting,
                    bis: Vec::new(),
                    detail: String::new(),
                    battery: None,
                })
                .collect();
            list.0
        };
        if jobs.is_empty() {
            return Assistants { stop, threads: Vec::new() };
        }
        let host = Arc::new(host);
        let turn = Arc::new(Mutex::new(()));
        let start = Arc::new(Barrier::new(jobs.len()));
        let threads = jobs
            .into_iter()
            .enumerate()
            .filter_map(|(index, job)| {
                let i = At { run, index };
                let (host, stop, turn, start, out) =
                    (host.clone(), stop.clone(), turn.clone(), start.clone(), out.clone());
                std::thread::Builder::new()
                    .name("le-assist".into())
                    .spawn(move || {
                        let stopped = || stop.load(Ordering::Acquire);
                        let until = Instant::now() + BROADCAST_TIMEOUT;
                        let host = loop {
                            if let Some(h) = host().filter(|h| !h.le_broadcast_groups().is_empty()) {
                                break Some(h);
                            }
                            if stopped() || Instant::now() > until {
                                break None;
                            }
                            std::thread::sleep(Duration::from_millis(100));
                        };
                        match host {
                            Some(h) => assist(&h, i, &job, &stopped, &turn, &start, &*out),
                            None => {
                                if !stopped() {
                                    out(&format!("Assistant {}: the broadcast did not start", job.address));
                                    set_phase(i, Phase::Failed, "the broadcast did not start");
                                }
                                start.wait();
                            }
                        }
                    })
                    .ok()
            })
            .collect();
        Assistants { stop, threads }
    }

    /// Tells each assistant to take the source off its receiver and go
    pub fn signal_stop(&self) {
        self.stop.store(true, Ordering::Release);
    }

    /// [`Assistants::signal_stop`] and waits for them, at most `limit` (one
    /// still looking for its receiver or connecting ends with that, later)
    pub fn stop_within(self, limit: Duration) {
        let (tx, rx) = mpsc::channel();
        let joiner = std::thread::Builder::new().name("le-assist-stop".into()).spawn(move || {
            self.stop();
            let _ = tx.send(());
        });
        if joiner.is_ok() {
            let _ = rx.recv_timeout(limit);
        }
    }

    /// [`Assistants::signal_stop`] and waits for them
    pub fn stop(self) {
        self.signal_stop();
        for t in self.threads {
            let _ = t.join();
        }
    }
}

/// A receiver connected: its link, its Broadcast Audio Scan Control Point,
/// and the notifications of its Broadcast Receive States
struct Link {
    id: LeDeviceId,
    cp: u16,
    watches: Vec<(u16, mpsc::Receiver<Vec<u8>>)>,
}

/// A Broadcast Receive State told; the state when it has a source
fn show_state(out: &dyn Fn(&str), who: &str, handle: u16, v: &[u8]) -> Option<bass::ReceiveState> {
    match bass::parse_receive_state(v) {
        Ok(None) => {
            out(&format!("Receiver {who} 0x{handle:04X}: empty"));
            None
        }
        Ok(Some(s)) => {
            out(&format!("Receiver {who} 0x{handle:04X}: {}", s.describe()));
            Some(s)
        }
        Err(e) => {
            out(&format!("Receiver {who} 0x{handle:04X}: {e}"));
            None
        }
    }
}

/// Finds and connects the receiver, watches its Broadcast Receive States
/// and takes a source left from before off it. `turn`: held throughout.
fn connect(
    host: &BtHost,
    i: At,
    who: &str,
    stopped: &dyn Fn() -> bool,
    turn: &Mutex<()>,
    out: &dyn Fn(&str),
) -> Option<Link> {
    use le_audio::defs::{UUID_CHR_BASS_CONTROL_POINT, UUID_CHR_BROADCAST_RECEIVE_STATE, UUID_SERVICE_BASS};
    // Battery Service, Battery Level
    const UUID_SERVICE_BATTERY: u16 = 0x180F;
    const UUID_CHR_BATTERY_LEVEL: u16 = 0x2A19;

    let Some(parsed) = bt_host::parse_addr(who) else {
        out(&format!("Assistant {who}: invalid address"));
        set_phase(i, Phase::Failed, "invalid address");
        return None;
    };
    let _held = turn.lock().unwrap_or_else(|e| e.into_inner());
    if stopped() {
        return None;
    }
    out(&format!("Assistant {who}: looking for it..."));
    set_phase(i, Phase::Looking, "");
    let Some(found) = host.le_find(parsed, FIND_TIMEOUT) else {
        out(&format!("Assistant {who}: not seen advertising in {} s", FIND_TIMEOUT.as_secs()));
        set_phase(i, Phase::NotFound, "");
        return None;
    };
    set_phase(i, Phase::Connecting, "");
    let id = match host.le_connect_bare(found.addr, &found.name, CONNECT_TIMEOUT) {
        Ok(id) => id,
        Err(e) => {
            out(&format!("Assistant {who}: cannot connect: {e}"));
            set_phase(i, Phase::Failed, &e);
            return None;
        }
    };
    let services = host.le_services(id);
    // Its battery level, when it tells it (shown with its state)
    if let Some(level) = services
        .iter()
        .filter(|s| s.uuid16 == UUID_SERVICE_BATTERY)
        .flat_map(|s| &s.chars)
        .find(|c| c.uuid16 == UUID_CHR_BATTERY_LEVEL)
        .and_then(|c| host.le_read(id, c.value_handle).ok())
        .and_then(|v| v.first().copied())
        .filter(|l| *l <= 100)
    {
        out(&format!("Assistant {who}: battery {level}%"));
        update(i, |r| r.battery = Some(level));
    }
    let Some(svc) = services.iter().find(|s| s.uuid16 == UUID_SERVICE_BASS) else {
        out(&format!("Assistant {who}: no BASS"));
        set_phase(i, Phase::NoBass, "");
        host.le_disconnect(id);
        return None;
    };
    let cp = svc.chars.iter().find(|c| c.uuid16 == UUID_CHR_BASS_CONTROL_POINT).map(|c| c.value_handle);
    let states: Vec<u16> =
        svc.chars.iter().filter(|c| c.uuid16 == UUID_CHR_BROADCAST_RECEIVE_STATE).map(|c| c.value_handle).collect();
    let Some(cp) = cp else {
        out(&format!("Assistant {who}: no Broadcast Audio Scan Control Point"));
        set_phase(i, Phase::NoBass, "no control point");
        host.le_disconnect(id);
        return None;
    };
    let mut watches = Vec::new();
    for &h in &states {
        match host.le_watch(id, h) {
            Ok(rx) => watches.push((h, rx)),
            Err(e) => out(&format!("Assistant {who}: notifications of 0x{h:04X} not on (0x{e:02X})")),
        }
        if let Ok(v) = host.le_read(id, h)
            && let Some(old) = show_state(out, who, h, &v)
        {
            out(&format!("Assistant {who}: taking source {} off the receiver first", old.source_id));
            take_off(host, id, cp, old.source_id, old.subgroups.len(), who, out);
        }
    }
    let link = Link { id, cp, watches };
    link.show_changes(who, out);
    out(&format!("Assistant {who}: ready"));
    set_phase(i, Phase::Ready, "");
    Some(link)
}

/// Modify Source (PA off, no BIS) and Remove Source
fn take_off(host: &BtHost, id: LeDeviceId, cp: u16, source_id: u8, subgroups: usize, who: &str, out: &dyn Fn(&str)) {
    let off = vec![0; subgroups];
    let modify = bass::modify_source(source_id, bass::PA_SYNC_OFF, bass::PA_INTERVAL_UNKNOWN, &off);
    if let Err(e) = host.le_write(id, cp, modify) {
        out(&format!("Assistant {who}: Modify Source refused (0x{e:02X})"));
    }
    std::thread::sleep(Duration::from_millis(500));
    if let Err(e) = host.le_write(id, cp, bass::remove_source(source_id)) {
        out(&format!("Assistant {who}: Remove Source refused (0x{e:02X})"));
    }
    std::thread::sleep(Duration::from_millis(300));
}

impl Link {
    fn show_changes(&self, who: &str, out: &dyn Fn(&str)) {
        for (h, rx) in &self.watches {
            while let Ok(v) = rx.try_recv() {
                show_state(out, who, *h, &v);
            }
        }
    }
}

/// The source to add: waits for the broadcast's BIGs; Err (why) when the
/// job's broadcast or BIS are not there
fn source_of(host: &BtHost, job: &Job, stopped: &dyn Fn() -> bool) -> Result<(BroadcastGroup, bass::Source), String> {
    let until = Instant::now() + BROADCAST_TIMEOUT;
    let groups = loop {
        let g = host.le_broadcast_groups();
        if !g.is_empty() {
            break g;
        }
        if stopped() || Instant::now() > until {
            return Err("the broadcast did not start".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    let Some(g) = groups.into_iter().nth(job.broadcast) else {
        return Err(format!("the layout has fewer than {} broadcast(s)", job.broadcast + 1));
    };
    let public = host.local_address().and_then(|a| bt_host::parse_addr(&a)).unwrap_or([0; 6]);
    let (addr, random) = g.advertiser_address(public);
    let wanted: u32 = job.bis.iter().filter(|&&b| b > 0).fold(0, |m, &b| m | 1 << (b - 1));
    let bis_sync: Vec<u32> = g
        .subgroup_bis()
        .iter()
        .map(|&m| if job.bis.is_empty() { bass::BIS_NO_PREFERENCE } else { m & wanted })
        .collect();
    if !job.bis.is_empty() && bis_sync.iter().all(|&m| m == 0) {
        return Err(format!("\"{}\" has BIS 1-{} only", g.name, g.bis.len()));
    }
    let source = bass::Source {
        addr_type: u8::from(random),
        addr,
        adv_sid: g.sid,
        broadcast_id: g.broadcast_id,
        pa_sync: bass::PA_SYNC_NO_PAST,
        pa_interval: bass::PA_INTERVAL_UNKNOWN,
        bis_sync,
    };
    Ok((g, source))
}

/// A saved LE Audio device an assistant can tell
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub address: String,
    pub name: String,
    /// BASS among the GATT services kept at its last connection; None:
    /// none kept (never connected)
    pub bass: Option<bool>,
}

/// The saved LE Audio devices (le_audio_devices.txt), each with whether it
/// has BASS as far as known: those with first, then not known, then
/// without; by name within each
pub fn candidates(config_dir: &std::path::Path) -> Vec<Candidate> {
    let cache = crate::paths::pairing_dir(config_dir).join(bt_host::le_gatt_cache::FILE_NAME);
    const BASS: &str = "0000184f-0000-1000-8000-00805f9b34fb";
    let mut list: Vec<Candidate> = crate::devices::load_le(config_dir)
        .into_iter()
        .map(|d| {
            let bass = bt_host::le_gatt_cache::service_uuids(&cache, &d.address).map(|u| u.iter().any(|s| s == BASS));
            Candidate { address: d.address, name: d.name, bass }
        })
        .collect();
    let rank = |b: Option<bool>| match b {
        Some(true) => 0,
        None => 1,
        Some(false) => 2,
    };
    list.sort_by(|a, b| {
        rank(a.bass).cmp(&rank(b.bass)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    list
}

/// The BIS (1-based) a receive state reports synchronized to, over its
/// subgroups
fn synced_bis(s: &bass::ReceiveState) -> Vec<u8> {
    let mut v: Vec<u8> = s
        .subgroups
        .iter()
        .filter(|g| g.bis_sync != bass::BIS_NO_PREFERENCE)
        .flat_map(|g| (0..31u8).filter(move |b| g.bis_sync & (1 << b) != 0).map(|b| b + 1))
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// The receiver reports every BIS it was asked (`asked` empty: it picks,
/// so any)
fn plays_all_asked(asked: &[u8], synced: &[u8]) -> bool {
    !synced.is_empty() && asked.iter().all(|b| synced.contains(b))
}

/// One receiver from connection to stop (see the module's description)
fn assist(
    host: &BtHost,
    i: At,
    job: &Job,
    stopped: &dyn Fn() -> bool,
    turn: &Mutex<()>,
    start: &Barrier,
    out: &dyn Fn(&str),
) {
    let who = job.address.as_str();
    let link = connect(host, i, who, stopped, turn, out);
    let plan = link.as_ref().map(|_| source_of(host, job, stopped));
    start.wait();
    let Some(link) = link else { return };
    let (g, source) = match plan {
        Some(Ok(p)) if !stopped() => p,
        other => {
            // Not a failure when the stop came first
            if let Some(Err(e)) = other
                && !stopped()
            {
                out(&format!("Assistant {who}: {e}"));
                set_phase(i, Phase::Failed, &e);
            }
            host.le_disconnect(link.id);
            return;
        }
    };
    let (id, cp) = (link.id, link.cp);
    link.show_changes(who, out);
    out(&format!(
        "Assistant {who}: Add Source \"{}\" (Broadcast_ID 0x{:06X}, SID {}, {} address, BIS_Sync {:08X?})",
        g.name,
        g.broadcast_id,
        g.sid,
        if source.addr_type != 0 { "random" } else { "public" },
        source.bis_sync
    ));
    set_phase(i, Phase::Added, "");
    if let Err(e) = host.le_write(id, cp, bass::add_source(&source)) {
        out(&format!("Assistant {who}: Add Source refused (0x{e:02X})"));
        set_phase(i, Phase::Failed, &format!("Add Source refused (0x{e:02X})"));
    }
    let mut source_id = None;
    let mut code_sent = false;
    let mut seen_groups = false;
    let mut playing_since: Option<Instant> = None;
    // Playing every BIS asked (not only the first of a stereo pair)
    let mut all_asked = false;
    // [`Job::leave`]: the link is closed, the receiver plays on
    let mut left = false;
    // It played: losing the BIS later (PA kept) is told again ([`Resync`])
    let mut played = false;
    let mut lost_since: Option<Instant> = None;
    let mut resyncs = 0u32;
    let mut last_resync: Option<Instant> = None;
    while !stopped() {
        let mut idle = true;
        for (h, rx) in &link.watches {
            while let Ok(v) = rx.try_recv() {
                idle = false;
                let Some(s) = show_state(out, who, *h, &v) else { continue };
                if s.broadcast_id != g.broadcast_id {
                    continue;
                }
                source_id = Some(s.source_id);
                let bis = synced_bis(&s);
                let (phase, detail) = if !bis.is_empty() {
                    (Phase::Playing, "")
                } else if s.big_encryption == BIG_BAD_CODE {
                    (Phase::Failed, "wrong Broadcast_Code")
                } else if s.pa_sync == PA_SYNCHRONIZED {
                    (Phase::Synced, "")
                } else if s.pa_sync == PA_SYNC_FAILED {
                    (Phase::Failed, "cannot synchronize to the broadcast")
                } else {
                    (Phase::Added, "")
                };
                if phase != Phase::Playing {
                    playing_since = None;
                } else if playing_since.is_none() {
                    playing_since = Some(Instant::now());
                }
                if phase == Phase::Playing {
                    if resyncs > 0 {
                        out(&format!("Assistant {who}: it plays again"));
                    }
                    played = true;
                    (lost_since, resyncs, last_resync) = (None, 0, None);
                } else if played && phase == Phase::Synced {
                    lost_since.get_or_insert_with(Instant::now);
                } else {
                    lost_since = None;
                }
                all_asked = plays_all_asked(&job.bis, &bis);
                set_bis(i, bis);
                set_phase(i, phase, detail);
                if s.big_encryption == 1 && !code_sent {
                    match &job.code {
                        Some(code) => {
                            code_sent = true;
                            out(&format!("Assistant {who}: Set Broadcast_Code"));
                            if let Err(e) =
                                host.le_write(id, cp, bass::set_broadcast_code(s.source_id, code.as_bytes()))
                            {
                                out(&format!("Assistant {who}: Set Broadcast_Code refused (0x{e:02X})"));
                            }
                        }
                        None => set_phase(i, phase, "asks for a Broadcast_Code"),
                    }
                }
            }
        }
        let wait = if all_asked { LEAVE_AFTER } else { LEAVE_AFTER_OTHER_BIS };
        if job.leave && !left && playing_since.is_some_and(|t| t.elapsed() >= wait) {
            out(&format!("Assistant {who}: it plays; closing the link (the receiver keeps the broadcast)"));
            set_phase(i, Phase::Playing, "link closed");
            host.le_disconnect(id);
            left = true;
        }
        if !left
            && let (Some(lost), Some(sid)) = (lost_since, source_id)
            && let Some(how) = resync_due(lost.elapsed(), resyncs, last_resync.map(|t| t.elapsed()))
        {
            resyncs += 1;
            last_resync = Some(Instant::now());
            out(&format!(
                "Assistant {who}: no BIS for {} ms (PA kept): {how:?} ({resyncs} of {RESYNC_TRIES})",
                lost.elapsed().as_millis()
            ));
            let modify = |bis: &[u32]| {
                let m = bass::modify_source(sid, source.pa_sync, source.pa_interval, bis);
                if let Err(e) = host.le_write(id, cp, m) {
                    out(&format!("Assistant {who}: Modify Source refused (0x{e:02X})"));
                }
            };
            match how {
                Resync::Modify => modify(&source.bis_sync),
                Resync::Toggle => {
                    modify(&vec![0; source.bis_sync.len()]);
                    std::thread::sleep(Duration::from_millis(500));
                    modify(&source.bis_sync);
                }
                Resync::ReAdd => {
                    if let Err(e) = host.le_write(id, cp, bass::remove_source(sid)) {
                        out(&format!("Assistant {who}: Remove Source refused (0x{e:02X})"));
                    }
                    std::thread::sleep(Duration::from_millis(500));
                    if let Err(e) = host.le_write(id, cp, bass::add_source(&source)) {
                        out(&format!("Assistant {who}: Add Source refused (0x{e:02X})"));
                    }
                    // A new source: it asks for the Broadcast_Code again
                    code_sent = false;
                }
            }
        }
        // The broadcast gone (stopped elsewhere): nothing left to play
        let up = !host.le_broadcast_groups().is_empty();
        if up {
            seen_groups = true;
        } else if seen_groups {
            break;
        }
        if idle {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    if left {
        // No link to take the source off through: the receiver drops the
        // broadcast by itself once it is gone
        set_bis(i, Vec::new());
        set_phase(i, Phase::Done, "link closed");
        return;
    }
    if let Some(sid) = source_id {
        out(&format!("Assistant {who}: taking the source off the receiver"));
        take_off(host, id, cp, sid, source.bis_sync.len(), who, out);
        link.show_changes(who, out);
    }
    set_bis(i, Vec::new());
    set_phase(i, Phase::Done, "");
    host.le_disconnect(id);
}

#[cfg(test)]
mod tests {
    use le_audio::broadcast::{PairMode, SingleMode};

    use super::*;

    fn ch(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_job_takes_the_bis_of_its_channels() {
        let single = BroadcastLayout::Single(SingleMode::RearMono);
        let j = job_for(single, "aa:bb:cc:dd:ee:ff", "Speaker", &ch(&["RL"]), None).unwrap();
        assert_eq!((j.address.as_str(), j.broadcast, j.bis.clone()), ("AA:BB:CC:DD:EE:FF", 0, vec![3]));
        let j = job_for(single, "A", "", &ch(&["FR", "FL"]), None).unwrap();
        assert_eq!(j.bis, [1, 2]);
        // Left and right in two broadcasts: the second one's BIS 1
        let j = job_for(BroadcastLayout::Split, "A", "", &ch(&["R"]), None).unwrap();
        assert_eq!((j.broadcast, j.bis.clone()), (1, vec![1]));
        assert!(job_for(BroadcastLayout::Split, "A", "", &ch(&["L", "R"]), None).is_err());
        let pairs = BroadcastLayout::Matrix { front: PairMode::Stereo, rear: PairMode::Stereo };
        let j = job_for(pairs, "A", "", &ch(&["RL", "RR"]), None).unwrap();
        assert_eq!((j.broadcast, j.bis.clone()), (1, vec![1, 2]));
        assert!(job_for(BroadcastLayout::Stereo, "A", "", &ch(&["RL"]), None).is_err());
        assert!(job_for(BroadcastLayout::Stereo, "A", "", &ch(&["X"]), None).is_err());
        assert_eq!(job_for(BroadcastLayout::Mono, "A", "", &ch(&["M"]), None).unwrap().bis, [1]);
    }

    #[test]
    fn a_receiver_plays_all_it_was_asked_once_each_bis_is_reported() {
        // A stereo receiver that has only its first BIS yet
        assert!(!plays_all_asked(&[3, 4], &[3]));
        assert!(plays_all_asked(&[3, 4], &[3, 4]));
        // One that reports BIS 1, 2 whatever is asked
        assert!(!plays_all_asked(&[3, 4], &[1, 2]));
        // Left to pick: any BIS, but one
        assert!(plays_all_asked(&[], &[1]));
        assert!(!plays_all_asked(&[], &[]));
    }

    #[test]
    fn a_receiver_that_lost_its_bis_is_told_again_three_ways() {
        let s = Duration::from_secs;
        // Not before RESYNC_AFTER without BIS
        assert_eq!(resync_due(s(2), 0, None), None);
        assert_eq!(resync_due(s(3), 0, None), Some(Resync::Modify));
        // Each try waits as long after the last
        assert_eq!(resync_due(s(4), 1, Some(s(1))), None);
        assert_eq!(resync_due(s(6), 1, Some(s(3))), Some(Resync::Toggle));
        assert_eq!(resync_due(s(9), 2, Some(s(3))), Some(Resync::ReAdd));
        // Then it is left as it is
        assert_eq!(resync_due(s(60), 3, Some(s(30))), None);
    }

    #[test]
    fn the_bis_a_receiver_reports() {
        let s = bass::ReceiveState {
            source_id: 1,
            addr_type: 0,
            addr: [0; 6],
            adv_sid: 1,
            broadcast_id: 1,
            pa_sync: PA_SYNCHRONIZED,
            big_encryption: 0,
            bad_code: None,
            subgroups: vec![
                bass::ReceiveSubgroup { bis_sync: 0b0001, metadata: Vec::new() },
                bass::ReceiveSubgroup { bis_sync: 0b0100, metadata: Vec::new() },
            ],
        };
        assert_eq!(synced_bis(&s), [1, 3]);
    }
}

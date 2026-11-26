//! Broadcast source (Auracast, developer mode): what BlueCaster advertises and
//! how the BIG is set up, as plain data.
//!
//! A broadcast is one BIG of one BIS per channel (stereo: Front Left and
//! Front Right, which a pair of earbuds picks from; mono: one BIS without a
//! location), announced by
//! - extended advertising ([`adv_data`]): Broadcast Audio Announcement
//!   (service data 0x1852, Broadcast_ID), Public Broadcast Announcement
//!   (service data 0x1856, PBP features: encrypted, standard / high
//!   quality) and the Broadcast Name (AD type 0x30), and
//! - periodic advertising ([`base_ad`]): the Basic Audio Announcement
//!   (service data 0x1851) with the BASE: presentation delay, one subgroup
//!   (LC3, sampling frequency, frame duration, octets per frame; Media in
//!   the metadata) and the BIS with their channel allocation.
//!
//! The configurations are BAP's broadcast "high reliability" QoS settings
//! (BAP 1.0.1 Table 6.4, written from memory: RTN 4, max transport latency
//! 60 ms for 16 / 24 / 32 kHz and 65 ms for 48 kHz, 10 ms frames, unframed,
//! presentation delay 40 ms). PBP's standard quality is 16_2 / 24_2, high
//! quality 48 kHz.
//!
//! The stream reuses the unicast [`StreamPlan`] (one "CIS" entry per BIS, no
//! device), so that the encode thread and the ISO sender serve both.

use crate::defs::{
    CFG_CHANNEL_ALLOCATION, CFG_FRAME_DURATION, CFG_OCTETS_PER_FRAME, CFG_SAMPLING_FREQUENCY, CODING_FORMAT_LC3,
    CONTEXT_MEDIA, LOCATION_BACK_LEFT, LOCATION_BACK_RIGHT, LOCATION_FRONT_LEFT, LOCATION_FRONT_RIGHT, LOCATION_MONO,
    META_STREAMING_CONTEXTS, PHY_2M, TARGET_LATENCY_HIGH_RELIABILITY, sf_hz_to_code,
};
use crate::ltv;
use crate::plan::{ChannelPlan, ChannelSource, CisPlan, StreamPlan};
use crate::presets::StreamPreset;

/// Basic Audio Announcement Service (the BASE, periodic advertising)
pub const UUID_BASIC_AUDIO_ANNOUNCEMENT: u16 = 0x1851;
/// Broadcast Audio Announcement Service (Broadcast_ID)
pub const UUID_BROADCAST_AUDIO_ANNOUNCEMENT: u16 = 0x1852;
/// Public Broadcast Announcement (PBP)
pub const UUID_PUBLIC_BROADCAST_ANNOUNCEMENT: u16 = 0x1856;
/// AD type Broadcast Name
pub const AD_BROADCAST_NAME: u8 = 0x30;
/// AD type Service Data - 16-bit UUID
const AD_SERVICE_DATA_UUID16: u8 = 0x16;

/// PBP features: the broadcast is encrypted
pub const PBP_ENCRYPTED: u8 = 0x01;
/// PBP features: Standard Quality Public Broadcast Audio (16_2 / 24_2)
pub const PBP_STANDARD_QUALITY: u8 = 0x02;
/// PBP features: High Quality Public Broadcast Audio (48 kHz)
pub const PBP_HIGH_QUALITY: u8 = 0x04;

/// The default configuration ("high quality")
pub const DEFAULT_PRESET: &str = "48_4_2";
/// PBP standard quality
pub const STANDARD_PRESET: &str = "24_2_2";
/// Between the two (32 kHz, no PBP quality): the most four BIS in one BIG
/// on the BT600 take without losing SDUs (48 kHz: about 60 % lost)
pub const MID_PRESET: &str = "32_2_2";

/// Broadcast Name: 4 to 32 octets of UTF-8 (PBP)
pub const NAME_MIN: usize = 4;
pub const NAME_MAX: usize = 32;
/// Broadcast_Code given as a string: 4 to 16 octets of UTF-8
pub const CODE_MIN: usize = 4;
pub const CODE_MAX: usize = 16;

/// The BIG handle of the broadcast
pub const BIG_HANDLE: u8 = 0;
/// Advertising SID of the broadcast's advertising set
pub const ADV_SID: u8 = 1;
/// Extended advertising interval, 0.625 ms units (100-150 ms)
pub const ADV_INTERVAL: (u16, u16) = (0x00A0, 0x00F0);
/// Periodic advertising interval, 1.25 ms units (100-150 ms)
pub const PA_INTERVAL: (u16, u16) = (0x0050, 0x0078);

fn make(name: &str, hz: u32, octets: u16, latency_ms: u16) -> StreamPreset {
    StreamPreset {
        name: name.into(),
        sampling_hz: hz,
        frame_us: 10000,
        octets,
        target_latency: TARGET_LATENCY_HIGH_RELIABILITY,
        framing: 0,
        rtn: 4,
        max_transport_latency_ms: latency_ms,
        presentation_delay_us: 40000,
    }
}

/// The broadcast configurations by name (BAP 1.0.1 Table 6.4, high
/// reliability, 10 ms frames)
pub fn presets() -> Vec<StreamPreset> {
    vec![
        make("16_2_2", 16000, 40, 60),
        make("24_2_2", 24000, 60, 60),
        make("32_2_2", 32000, 80, 60),
        make("48_2_2", 48000, 100, 65),
        make("48_4_2", 48000, 120, 65),
        make("48_6_2", 48000, 155, 65),
    ]
}

pub fn find_preset(name: &str) -> Option<StreamPreset> {
    presets().into_iter().find(|p| p.name == name)
}

/// What the user chose for a broadcast
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BroadcastConfig {
    /// Broadcast Name, [`NAME_MIN`]..=[`NAME_MAX`] octets ([`check_name`])
    pub name: String,
    /// Broadcast_Code ([`broadcast_code`]); None: open
    pub code: Option<[u8; 16]>,
    pub preset: StreamPreset,
    /// A matrix layout's rear (RL RR) at a configuration of its own (same
    /// frame duration); None: `preset`
    pub rear_preset: Option<StreamPreset>,
    pub layout: BroadcastLayout,
    /// 24 bits, random per broadcast (the first broadcast's; the others
    /// count up from it)
    pub broadcast_id: u32,
}

/// A BIS of a layout: its location and where its audio comes from
type LayoutBis = (u32, ChannelSource);

/// A broadcast of a layout
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LayoutBroadcast {
    /// Name suffix (None: the name as it is)
    suffix: Option<&'static str>,
    /// Its number of BIS
    bis: usize,
    /// BIS per subgroup of its BASE (each labelled by Program_Info); 0:
    /// one subgroup for all
    subgroup_size: usize,
}

/// A broadcast of `bis` BIS, a subgroup per BIS (`per_bis`) or one for all
const fn broadcast(suffix: Option<&'static str>, bis: usize, per_bis: bool) -> LayoutBroadcast {
    LayoutBroadcast { suffix, bis, subgroup_size: if per_bis { 1 } else { 0 } }
}

/// How the four outputs of [`BroadcastLayout::Single`] are laid out in its
/// one BIG (for trying receivers: which they take, and how)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SingleMode {
    /// One subgroup: FL FR, the rear without a location (mono, which a
    /// speaker of any location takes). Four mono receivers, or a stereo
    /// one on the front.
    #[default]
    RearMono,
    /// One subgroup: FL FR BL BR (a receiver whose sink locations are FL
    /// FR, as a HomeBuddy's, takes no BL / BR BIS)
    Located,
    /// Two subgroups of FL FR, front then rear: a stereo receiver each
    Pairs,
    /// Four subgroups of one BIS, each labelled, without a location
    PerBis,
}

impl SingleMode {
    pub const ALL: [SingleMode; 4] = [SingleMode::RearMono, SingleMode::Located, SingleMode::Pairs, SingleMode::PerBis];

    /// The layout's name: "single" + this
    fn suffix(self) -> &'static str {
        match self {
            SingleMode::RearMono => "",
            SingleMode::Located => "_located",
            SingleMode::Pairs => "_pairs",
            SingleMode::PerBis => "_per_bis",
        }
    }
}

/// How a pair of matrix outputs (front FL FR, rear RL RR) is sent
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PairMode {
    /// One stereo broadcast ("<name> Front" / "<name> Rear"), a left and a
    /// right BIS
    #[default]
    Stereo,
    /// Each output a mono broadcast of its own ("<name> FL" / "FR" / "RL"
    /// / "RR"): two BIGs
    Separate,
    /// One broadcast of two subgroups ("<name> Front" / "<name> Rear"),
    /// one mono BIS each, labelled by Program_Info ("Front left", ...), for
    /// a receiver that lets one pick a subgroup
    Subgroups,
    /// Not sent
    Off,
}

impl PairMode {
    pub const ALL: [PairMode; 4] = [PairMode::Stereo, PairMode::Separate, PairMode::Subgroups, PairMode::Off];

    pub fn as_str(self) -> &'static str {
        match self {
            PairMode::Stereo => "stereo",
            PairMode::Separate => "separate",
            PairMode::Subgroups => "subgroups",
            PairMode::Off => "off",
        }
    }

    pub fn parse(s: &str) -> Option<PairMode> {
        PairMode::ALL.into_iter().find(|m| m.as_str() == s)
    }

    /// The BIGs (broadcasts) it takes
    pub fn bigs(self) -> usize {
        match self {
            PairMode::Stereo | PairMode::Subgroups => 1,
            PairMode::Separate => 2,
            PairMode::Off => 0,
        }
    }

    /// Its BIS and broadcasts: `left` / `right` the sources, `pair` the
    /// name suffix of one broadcast, `names` those of two
    fn spec(
        self,
        (left, right): (ChannelSource, ChannelSource),
        pair: &'static str,
        names: (&'static str, &'static str),
        bis: &mut Vec<LayoutBis>,
        broadcasts: &mut Vec<LayoutBroadcast>,
    ) {
        const FL: u32 = LOCATION_FRONT_LEFT;
        const FR: u32 = LOCATION_FRONT_RIGHT;
        const MONO: u32 = LOCATION_MONO;
        match self {
            PairMode::Stereo => {
                bis.extend([(FL, left), (FR, right)]);
                broadcasts.push(broadcast(Some(pair), 2, false));
            }
            PairMode::Separate => {
                bis.extend([(MONO, left), (MONO, right)]);
                broadcasts.extend([broadcast(Some(names.0), 1, false), broadcast(Some(names.1), 1, false)]);
            }
            PairMode::Subgroups => {
                bis.extend([(MONO, left), (MONO, right)]);
                broadcasts.push(broadcast(Some(pair), 2, true));
            }
            PairMode::Off => {}
        }
    }
}

/// The channels of a broadcast and how they are split over broadcasts
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BroadcastLayout {
    /// One broadcast of two BIS, Front Left / Right
    #[default]
    Stereo,
    /// One broadcast of one mono BIS (the downmix)
    Mono,
    /// Left and right as two broadcasts of their own ("<name> L" / "<name>
    /// R", one mono BIS each, a BIG each), for a pair of speakers that each
    /// tune to one broadcast
    Split,
    /// The matrix mixer's four outputs: the front (FL FR) and the rear (RL
    /// RR) each sent as its [`PairMode`] says (not both off). The BT600
    /// runs two BIGs at most ([`BroadcastLayout::bigs`]).
    Matrix { front: PairMode, rear: PairMode },
    /// The matrix mixer's four outputs as one broadcast (one BIG, one
    /// subgroup) of four BIS: Front Left / Right, then the rear without a
    /// location. For receivers an Auracast assistant tells their BIS (BASS
    /// BIS_Sync); a stereo receiver left to itself takes the front by its
    /// locations. Receivers pick BIS by location too: a HomeBuddy told BIS
    /// 1 played every Front Left BIS, and one told BIS 3 located Back Left
    /// took none (its sink locations are FL FR), so the rear BIS carry no
    /// Audio_Channel_Allocation (mono, which a speaker of any location
    /// takes).
    /// One BIG: the BT600 loses SDUs once two BIGs' events meet. The
    /// other ways to lay them out are [`SingleMode`]'s.
    Single(SingleMode),
}

impl BroadcastLayout {
    /// Each speaker a mono broadcast of its own (four BIGs)
    pub const QUAD: BroadcastLayout = BroadcastLayout::Matrix { front: PairMode::Separate, rear: PairMode::Separate };
    /// A stereo front, two mono rears (three BIGs)
    pub const FRONT_REAR: BroadcastLayout =
        BroadcastLayout::Matrix { front: PairMode::Stereo, rear: PairMode::Separate };
    /// Front and rear as two stereo broadcasts (two BIGs)
    pub const PAIRS: BroadcastLayout = BroadcastLayout::Matrix { front: PairMode::Stereo, rear: PairMode::Stereo };
    /// A stereo front, the rear in subgroups (two BIGs)
    pub const REAR_SUBGROUPS: BroadcastLayout =
        BroadcastLayout::Matrix { front: PairMode::Stereo, rear: PairMode::Subgroups };
    /// Front and rear in subgroups (two BIGs)
    pub const SUBGROUPS: BroadcastLayout =
        BroadcastLayout::Matrix { front: PairMode::Subgroups, rear: PairMode::Subgroups };

    /// The names of earlier versions for matrix layouts (command line)
    const ALIASES: [(&'static str, BroadcastLayout); 5] = [
        ("quad", BroadcastLayout::QUAD),
        ("front_rear", BroadcastLayout::FRONT_REAR),
        ("pairs", BroadcastLayout::PAIRS),
        ("rear_subgroups", BroadcastLayout::REAR_SUBGROUPS),
        ("subgroups", BroadcastLayout::SUBGROUPS),
    ];

    /// The forms [`BroadcastLayout::parse`] takes, for an error message
    pub const NAMES: &'static str = "stereo, mono, split, single, single_located, single_pairs, single_per_bis, \
                                      matrix_<front>_<rear> (each stereo, separate, subgroups or off), quad, \
                                      front_rear, pairs, rear_subgroups, subgroups";

    /// The name in the API and on the command line
    pub fn as_str(self) -> String {
        match self {
            BroadcastLayout::Stereo => "stereo".into(),
            BroadcastLayout::Mono => "mono".into(),
            BroadcastLayout::Split => "split".into(),
            BroadcastLayout::Single(m) => format!("single{}", m.suffix()),
            BroadcastLayout::Matrix { front, rear } => format!("matrix_{}_{}", front.as_str(), rear.as_str()),
        }
    }

    /// From [`BroadcastLayout::as_str`] or an older name ('-' taken for
    /// '_'), any case
    pub fn parse(s: &str) -> Option<BroadcastLayout> {
        let s = s.trim().to_ascii_lowercase().replace('-', "_");
        match s.as_str() {
            "stereo" => return Some(BroadcastLayout::Stereo),
            "mono" => return Some(BroadcastLayout::Mono),
            "split" => return Some(BroadcastLayout::Split),
            _ => {}
        }
        if let Some(m) = SingleMode::ALL.iter().find(|m| s == format!("single{}", m.suffix())) {
            return Some(BroadcastLayout::Single(*m));
        }
        if let Some((_, l)) = BroadcastLayout::ALIASES.iter().find(|(n, _)| *n == s) {
            return Some(*l);
        }
        let (front, rear) = s.strip_prefix("matrix_")?.split_once('_')?;
        let (front, rear) = (PairMode::parse(front)?, PairMode::parse(rear)?);
        (front != PairMode::Off || rear != PairMode::Off).then_some(BroadcastLayout::Matrix { front, rear })
    }

    /// Fed by the matrix mixer (else the fixed stereo / mono downmix)
    pub fn is_matrix(self) -> bool {
        matches!(self, BroadcastLayout::Matrix { .. } | BroadcastLayout::Single(_))
    }

    /// Whose rear may run at a configuration of its own (a broadcast of
    /// its own): a [`BroadcastLayout::Matrix`]
    fn rear_apart(self) -> bool {
        matches!(self, BroadcastLayout::Matrix { .. })
    }

    /// The BIS in plan order: (location, source) each, and its broadcasts
    fn spec(self) -> (Vec<LayoutBis>, Vec<LayoutBroadcast>) {
        use ChannelSource::*;
        const FL: u32 = LOCATION_FRONT_LEFT;
        const FR: u32 = LOCATION_FRONT_RIGHT;
        const MONO: u32 = LOCATION_MONO;
        match self {
            BroadcastLayout::Stereo => (vec![(FL, Left), (FR, Right)], vec![broadcast(None, 2, false)]),
            BroadcastLayout::Mono => (vec![(MONO, Mono)], vec![broadcast(None, 1, false)]),
            BroadcastLayout::Split => (
                vec![(MONO, Left), (MONO, Right)],
                vec![broadcast(Some("L"), 1, false), broadcast(Some("R"), 1, false)],
            ),
            BroadcastLayout::Single(m) => {
                let (fl, fr, bl, br, size) = match m {
                    SingleMode::RearMono => (FL, FR, MONO, MONO, 0),
                    SingleMode::Located => (FL, FR, LOCATION_BACK_LEFT, LOCATION_BACK_RIGHT, 0),
                    SingleMode::Pairs => (FL, FR, FL, FR, 2),
                    SingleMode::PerBis => (MONO, MONO, MONO, MONO, 1),
                };
                (
                    vec![(fl, Left), (fr, Right), (bl, RearLeft), (br, RearRight)],
                    vec![LayoutBroadcast { suffix: None, bis: 4, subgroup_size: size }],
                )
            }
            BroadcastLayout::Matrix { front, rear } => {
                let (mut bis, mut broadcasts) = (Vec::new(), Vec::new());
                front.spec((Left, Right), "Front", ("FL", "FR"), &mut bis, &mut broadcasts);
                rear.spec((RearLeft, RearRight), "Rear", ("RL", "RR"), &mut bis, &mut broadcasts);
                (bis, broadcasts)
            }
        }
    }

    /// BIS over all its broadcasts
    pub fn bis_count(self) -> usize {
        self.spec().0.len()
    }

    /// Broadcasts (BIGs, advertising sets)
    pub fn broadcasts(self) -> usize {
        self.spec().1.len()
    }

    /// Each BIS: its broadcast (0-based, as [`groups`] lists them), its
    /// index in that broadcast's BIG (1-based, as BIS_Sync counts) and the
    /// output it carries; what an Auracast assistant tells a receiver
    pub fn bis_map(self) -> Vec<(usize, u8, ChannelSource)> {
        let (bis, broadcasts) = self.spec();
        let mut sources = bis.into_iter().map(|(_, s)| s);
        let mut out = Vec::new();
        for (g, b) in broadcasts.iter().enumerate() {
            for i in 0..b.bis {
                out.extend(sources.next().map(|s| (g, i as u8 + 1, s)));
            }
        }
        out
    }
}

/// Retransmissions of each BIS payload when more than two BIS share the
/// air (the broadcast presets' 4 leave no room for four BIS and their
/// advertising in a 10 ms interval)
pub const RTN_MANY_BIS: u8 = 2;

/// One broadcast of a configuration: an advertising set and a BIG on it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BroadcastGroup {
    /// BIG handle, also btstack-sys' broadcast slot
    pub big_handle: u8,
    /// Advertising SID
    pub sid: u8,
    /// Broadcast Name as advertised
    pub name: String,
    pub broadcast_id: u32,
    /// Its BIS: these entries of the plan's `cis`, in BIS order
    pub bis: std::ops::Range<usize>,
    /// BIS per subgroup of its BASE (each labelled by Program_Info); 0:
    /// one subgroup for all
    pub subgroup_size: usize,
}

/// The broadcasts of a configuration, in order: BIG 0, 1, ..., SID 1, 2,
/// ..., each with its name and Broadcast_ID and its BIS of the plan
pub fn groups(config: &BroadcastConfig) -> Vec<BroadcastGroup> {
    let mut first = 0;
    config
        .layout
        .spec()
        .1
        .iter()
        .enumerate()
        .map(|(i, &LayoutBroadcast { suffix, bis: n, subgroup_size })| {
            let g = BroadcastGroup {
                big_handle: BIG_HANDLE + i as u8,
                sid: ADV_SID + i as u8,
                name: suffix.map_or_else(|| config.name.clone(), |s| suffixed_name(&config.name, s)),
                // Counting up from the first, after 0xFFFFFE (the last valid
                // ID) 1
                broadcast_id: config.broadcast_id.wrapping_add(i as u32).wrapping_sub(1) % 0x00FF_FFFE + 1,
                bis: first..first + n,
                subgroup_size,
            };
            first += n;
            g
        })
        .collect()
}

impl BroadcastGroup {
    /// The address its advertising set uses, as btstack-sys' shim sets it
    /// (`bluecaster_broadcast_advertise`): the public one for BIG 0, for
    /// every other a static random address of its own derived from it.
    /// (address most significant byte first, random)
    pub fn advertiser_address(&self, public: [u8; 6]) -> ([u8; 6], bool) {
        let null = public == [0; 6];
        if !null && self.big_handle == 0 {
            return (public, false);
        }
        let mut a = if null { [0xC2, 0xA2, 0xD0, 0x00, 0x00, 0x01] } else { public };
        a[0] |= 0xC0;
        a[5] ^= 0x5Au8.wrapping_add(self.big_handle);
        (a, true)
    }

    /// The BIS of each subgroup of its BASE, as BIS_Sync bit masks (bit n:
    /// BIS index n + 1): one subgroup with all, or one per
    /// [`BroadcastGroup::subgroup_size`] BIS
    pub fn subgroup_bis(&self) -> Vec<u32> {
        let n = self.bis.len().min(31);
        let size = if self.subgroup_size == 0 { n.max(1) } else { self.subgroup_size };
        (0..n).step_by(size).map(|first| (first..(first + size).min(n)).fold(0, |m, i| m | 1 << i)).collect()
    }
}

/// `name` + " " + `suffix` within [`NAME_MAX`] octets (the name cut at a
/// character boundary)
fn suffixed_name(name: &str, suffix: &str) -> String {
    let room = NAME_MAX - suffix.len() - 1;
    let mut base = name.to_string();
    while base.len() > room {
        base.pop();
    }
    format!("{} {suffix}", base.trim_end())
}

/// The part of `plan` one broadcast sends (its BIS, numbered from 1 within
/// its BIG), for its BASE
pub fn group_plan(plan: &StreamPlan, group: &BroadcastGroup) -> StreamPlan {
    let mut p = plan.clone();
    p.cis = plan.cis[group.bis.clone()].to_vec();
    // A broadcast runs at one configuration: its BIS's own, if they have one
    if let Some(own) = p.cis.first().and_then(|b| b.preset.clone()) {
        p.preset = own;
    }
    for (i, bis) in p.cis.iter_mut().enumerate() {
        bis.cis_id = i as u8 + 1;
        bis.preset = None;
    }
    p.cig_id = group.big_handle;
    p
}

/// The Broadcast Name as advertised: trimmed, at most [`NAME_MAX`] octets
/// (cut at a character boundary). Err when shorter than [`NAME_MIN`].
pub fn check_name(name: &str) -> Result<String, String> {
    let mut s = name.trim().to_string();
    while s.len() > NAME_MAX {
        s.pop();
    }
    if s.len() < NAME_MIN {
        return Err(format!("the broadcast name needs {NAME_MIN} to {NAME_MAX} bytes"));
    }
    Ok(s)
}

/// The Broadcast_Code of a code string: its UTF-8 octets, first character
/// first, zero padded to 16 (as the HCI parameter LE Create BIG takes).
/// Err when not [`CODE_MIN`]..=[`CODE_MAX`] octets.
pub fn broadcast_code(code: &str) -> Result<[u8; 16], String> {
    let b = code.as_bytes();
    if !(CODE_MIN..=CODE_MAX).contains(&b.len()) {
        return Err(format!("the broadcast code needs {CODE_MIN} to {CODE_MAX} bytes"));
    }
    let mut out = [0u8; 16];
    out[..b.len()].copy_from_slice(b);
    Ok(out)
}

/// A Broadcast_ID from any random value: 24 bits, never 0 or 0xFFFFFF
pub fn broadcast_id(random: u64) -> u32 {
    let id = (random ^ (random >> 24) ^ (random >> 48)) as u32 & 0x00FF_FFFF;
    match id {
        0 => 1,
        0x00FF_FFFF => 0x00FF_FFFE,
        id => id,
    }
}

/// PBP features of the configuration (of its first broadcast)
pub fn pbp_features(config: &BroadcastConfig) -> u8 {
    features_at(config, config.preset.sampling_hz)
}

/// The configuration one broadcast runs at: a matrix's rear its own
pub fn group_preset<'a>(config: &'a BroadcastConfig, group: &BroadcastGroup) -> &'a StreamPreset {
    let (channels, _) = config.layout.spec();
    let rear = channels.get(group.bis.clone()).is_some_and(|c| c.iter().any(|&(_, s)| s.is_rear()));
    match &config.rear_preset {
        Some(r) if rear && config.layout.rear_apart() => r,
        _ => &config.preset,
    }
}

fn features_at(config: &BroadcastConfig, sampling_hz: u32) -> u8 {
    let mut f = 0;
    if config.code.is_some() {
        f |= PBP_ENCRYPTED;
    }
    match sampling_hz {
        16000 | 24000 => f |= PBP_STANDARD_QUALITY,
        48000 => f |= PBP_HIGH_QUALITY,
        _ => {}
    }
    f
}

fn put_ad(out: &mut Vec<u8>, ty: u8, value: &[u8]) {
    out.push(value.len() as u8 + 1);
    out.push(ty);
    out.extend_from_slice(value);
}

/// The extended advertising data of the configuration's (first) broadcast
pub fn adv_data(config: &BroadcastConfig) -> Vec<u8> {
    let first = groups(config).swap_remove(0);
    group_adv_data(config, &first)
}

/// The extended advertising data of one broadcast: Broadcast Audio
/// Announcement, Public Broadcast Announcement (no metadata), Broadcast Name
pub fn group_adv_data(config: &BroadcastConfig, group: &BroadcastGroup) -> Vec<u8> {
    let mut out = Vec::new();
    let id = group.broadcast_id.to_le_bytes();
    let mut v = UUID_BROADCAST_AUDIO_ANNOUNCEMENT.to_le_bytes().to_vec();
    v.extend_from_slice(&id[..3]);
    put_ad(&mut out, AD_SERVICE_DATA_UUID16, &v);
    let mut v = UUID_PUBLIC_BROADCAST_ANNOUNCEMENT.to_le_bytes().to_vec();
    // Standard or high quality by the broadcast's own rate
    v.extend_from_slice(&[features_at(config, group_preset(config, group).sampling_hz), 0]);
    put_ad(&mut out, AD_SERVICE_DATA_UUID16, &v);
    put_ad(&mut out, AD_BROADCAST_NAME, group.name.as_bytes());
    out
}

/// The stream plan of a broadcast: one entry per BIS over all its
/// broadcasts, in [`groups`] order (BIS index = `cis_id`, from 1 within
/// each broadcast). Stereo: Front Left / Right from the left / right audio;
/// mono: one BIS of the downmix; a BIS alone in its broadcast has no
/// location (a speaker plays it whole). More than two BIS: RTN at most
/// [`RTN_MANY_BIS`].
pub fn plan(config: &BroadcastConfig) -> StreamPlan {
    let bis = |index: u8, allocation: u32, source: ChannelSource| CisPlan {
        device_id: 0,
        ase_id: 0,
        cis_id: index,
        acl: 0xFFFF,
        cis_handle: 0xFFFF,
        channels: vec![ChannelPlan { allocation, source }],
        source_ase: None,
        preset: None,
    };
    let (channels, _) = config.layout.spec();
    let many = channels.len() > 2;
    let held = |p: &StreamPreset| {
        let mut p = p.clone();
        if many {
            p.rtn = p.rtn.min(RTN_MANY_BIS);
        }
        p
    };
    // The rear's own configuration, when it differs (a matrix layout)
    let rear = config.rear_preset.as_ref().filter(|r| **r != config.preset && config.layout.rear_apart()).map(held);
    let mut cis = Vec::with_capacity(channels.len());
    for g in groups(config) {
        for (i, &(allocation, source)) in channels[g.bis].iter().enumerate() {
            let mut b = bis(i as u8 + 1, allocation, source);
            if source.is_rear() {
                b.preset = rear.clone();
            }
            cis.push(b);
        }
    }
    let preset = held(&config.preset);
    StreamPlan {
        preset,
        cig_id: BIG_HANDLE,
        phy: PHY_2M,
        requested_contexts: CONTEXT_MEDIA,
        contexts: CONTEXT_MEDIA,
        // One delay for every broadcast, so front and rear play together
        presentation_delay_us: config
            .preset
            .presentation_delay_us
            .max(config.rear_preset.as_ref().map_or(0, |r| r.presentation_delay_us)),
        cis,
        source: None,
        source_note: String::new(),
    }
}

/// One line for the log and the UI, e.g. "LC3 48 kHz 10 ms 120 octets (96
/// kbps/ch), 2 ch over 2 BIS, 48_4_2"
pub fn describe(plan: &StreamPlan) -> String {
    // A matrix whose rear runs at another rate: each configuration of its
    // own, in plan order ("... 2 ch over 2 BIS, 48_4_2 + ... 24_2_2")
    let mut parts: Vec<(&StreamPreset, usize, usize)> = Vec::new();
    for c in plan.sink_cis() {
        let p = plan.preset_of(c);
        match parts.iter_mut().find(|(q, _, _)| q.name == p.name) {
            Some((_, ch, bis)) => {
                *ch += c.channels.len();
                *bis += 1;
            }
            None => parts.push((p, c.channels.len(), 1)),
        }
    }
    if parts.len() <= 1 {
        return plan.describe().replace(" CIS, ", " BIS, ");
    }
    parts
        .iter()
        .map(|(p, ch, bis)| {
            format!(
                "LC3 {} kHz {} ms {} octets ({} kbps/ch), {ch} ch over {bis} BIS, {}",
                p.sampling_hz / 1000,
                if p.frame_us == 7500 { "7.5" } else { "10" },
                p.octets,
                p.kbps_per_channel(),
                p.name
            )
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

/// The BASE (BAP 1.0.1 3.7.2.2): presentation delay, one subgroup of LC3
/// with the plan's codec configuration and Media as Streaming Audio
/// Contexts, then each BIS with its channel allocation (none for mono)
pub fn base(plan: &StreamPlan) -> Vec<u8> {
    base_with(plan, 0)
}

/// Metadata type Program_Info (UTF-8, a subgroup's label)
pub const META_PROGRAM_INFO: u8 = 0x03;

/// A subgroup's label: the speaker its BIS carries, or (`pair`) the pair
fn program_info(source: ChannelSource, pair: bool) -> &'static str {
    if pair {
        return match source {
            ChannelSource::RearLeft | ChannelSource::RearRight => "Rear",
            _ => "Front",
        };
    }
    match source {
        ChannelSource::Left => "Front left",
        ChannelSource::Right => "Front right",
        ChannelSource::RearLeft => "Rear left",
        ChannelSource::RearRight => "Rear right",
        ChannelSource::Mono => "Mono",
    }
}

/// The BASE, with all BIS in one subgroup (`subgroup_size` 0) or in
/// subgroups of `subgroup_size` BIS, each labelled by Program_Info
pub fn base_with(plan: &StreamPlan, subgroup_size: usize) -> Vec<u8> {
    let p = &plan.preset;
    let mut out = Vec::new();
    out.extend_from_slice(&plan.presentation_delay_us.to_le_bytes()[..3]);
    let mut config = Vec::new();
    ltv::put(&mut config, CFG_SAMPLING_FREQUENCY, &[sf_hz_to_code(p.sampling_hz)]);
    ltv::put(&mut config, CFG_FRAME_DURATION, &[if p.frame_us == 7500 { 0x00 } else { 0x01 }]);
    ltv::put(&mut config, CFG_OCTETS_PER_FRAME, &p.octets.to_le_bytes());
    let subgroups: Vec<&[CisPlan]> =
        if subgroup_size > 0 { plan.cis.chunks(subgroup_size).collect() } else { vec![&plan.cis[..]] };
    out.push(subgroups.len() as u8);
    for bises in subgroups {
        out.push(bises.len() as u8);
        out.extend_from_slice(&[CODING_FORMAT_LC3, 0, 0, 0, 0]);
        out.push(config.len() as u8);
        out.extend_from_slice(&config);
        let mut metadata = Vec::new();
        ltv::put(&mut metadata, META_STREAMING_CONTEXTS, &plan.contexts.to_le_bytes());
        if subgroup_size > 0 {
            let source = bises[0].channels.first().map_or(ChannelSource::Mono, |c| c.source);
            ltv::put(&mut metadata, META_PROGRAM_INFO, program_info(source, subgroup_size > 1).as_bytes());
        }
        out.push(metadata.len() as u8);
        out.extend_from_slice(&metadata);
        for bis in bises {
            out.push(bis.cis_id);
            let mut config = Vec::new();
            let allocation = bis.allocation();
            if allocation != 0 {
                ltv::put(&mut config, CFG_CHANNEL_ALLOCATION, &allocation.to_le_bytes());
            }
            out.push(config.len() as u8);
            out.extend_from_slice(&config);
        }
    }
    out
}

/// The periodic advertising data of one broadcast of `plan`: its BASE
pub fn group_base_ad(plan: &StreamPlan, group: &BroadcastGroup) -> Vec<u8> {
    let mut v = UUID_BASIC_AUDIO_ANNOUNCEMENT.to_le_bytes().to_vec();
    v.extend_from_slice(&base_with(&group_plan(plan, group), group.subgroup_size));
    let mut out = Vec::new();
    put_ad(&mut out, AD_SERVICE_DATA_UUID16, &v);
    out
}

/// The periodic advertising data: the BASE as Basic Audio Announcement
/// service data
pub fn base_ad(plan: &StreamPlan) -> Vec<u8> {
    let mut v = UUID_BASIC_AUDIO_ANNOUNCEMENT.to_le_bytes().to_vec();
    v.extend_from_slice(&base(plan));
    let mut out = Vec::new();
    put_ad(&mut out, AD_SERVICE_DATA_UUID16, &v);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(stereo: bool, preset: &str, code: Option<&str>) -> BroadcastConfig {
        BroadcastConfig {
            name: "BlueCaster test".into(),
            code: code.map(|c| broadcast_code(c).unwrap()),
            preset: find_preset(preset).unwrap(),
            rear_preset: None,
            layout: if stereo { BroadcastLayout::Stereo } else { BroadcastLayout::Mono },
            broadcast_id: 0x123456,
        }
    }

    #[test]
    fn layouts_by_name() {
        let mut all = vec![BroadcastLayout::Stereo, BroadcastLayout::Mono, BroadcastLayout::Split];
        all.extend(SingleMode::ALL.map(BroadcastLayout::Single));
        for front in PairMode::ALL {
            for rear in PairMode::ALL {
                if (front, rear) != (PairMode::Off, PairMode::Off) {
                    all.push(BroadcastLayout::Matrix { front, rear });
                }
            }
        }
        for l in &all {
            assert_eq!(BroadcastLayout::parse(&l.as_str()), Some(*l));
        }
        assert_eq!(BroadcastLayout::QUAD.as_str(), "matrix_separate_separate");
        assert_eq!(BroadcastLayout::parse(" Front-Rear "), Some(BroadcastLayout::FRONT_REAR));
        assert_eq!(
            BroadcastLayout::parse("Matrix-Stereo-Off"),
            Some(BroadcastLayout::Matrix { front: PairMode::Stereo, rear: PairMode::Off })
        );
        for bad in ["5.1", "matrix_off_off", "matrix_stereo", "matrix_stereo_both"] {
            assert_eq!(BroadcastLayout::parse(bad), None, "{bad}");
        }
        let counts: Vec<_> = [
            BroadcastLayout::Stereo,
            BroadcastLayout::Mono,
            BroadcastLayout::Split,
            BroadcastLayout::Single(SingleMode::RearMono),
            BroadcastLayout::QUAD,
            BroadcastLayout::FRONT_REAR,
            BroadcastLayout::PAIRS,
            BroadcastLayout::REAR_SUBGROUPS,
            BroadcastLayout::SUBGROUPS,
        ]
        .iter()
        .map(|l| (l.bis_count(), l.broadcasts()))
        .collect();
        assert_eq!(counts, [(2, 1), (1, 1), (2, 2), (4, 1), (4, 4), (4, 3), (4, 2), (4, 2), (4, 2)]);
        // BIGs: as the pair modes add up
        for l in &all {
            if let BroadcastLayout::Matrix { front, rear } = l {
                assert_eq!(l.broadcasts(), front.bigs() + rear.bigs(), "{l:?}");
            }
        }
    }

    #[test]
    fn the_rear_may_run_at_a_rate_of_its_own() {
        let mut c = config(true, "48_4_2", None);
        c.layout = BroadcastLayout::PAIRS;
        c.rear_preset = find_preset("24_2_2");
        let p = plan(&c);
        let rates: Vec<_> = p.cis.iter().map(|b| (p.preset_of(b).sampling_hz, p.preset_of(b).rtn)).collect();
        assert_eq!(rates, [(48000, RTN_MANY_BIS), (48000, RTN_MANY_BIS), (24000, RTN_MANY_BIS), (24000, RTN_MANY_BIS)]);
        let g = groups(&c);
        let front = group_plan(&p, &g[0]);
        let rear = group_plan(&p, &g[1]);
        assert_eq!((front.preset.sampling_hz, rear.preset.sampling_hz), (48000, 24000));
        assert!(rear.cis.iter().all(|b| b.preset.is_none()));
        assert_eq!(rear.presentation_delay_us, front.presentation_delay_us);
        // The rear's BASE says 24 kHz (Sampling_Frequency code 5), the front's 48 kHz (8)
        let sf = |plan: &StreamPlan| {
            let b = base(plan);
            let at =
                b.windows(3).position(|w| w == [2, CFG_SAMPLING_FREQUENCY, sf_hz_to_code(plan.preset.sampling_hz)]);
            at.is_some()
        };
        assert!(sf(&front) && sf(&rear));
        assert_eq!(
            describe(&p),
            "LC3 48 kHz 10 ms 120 octets (96 kbps/ch), 2 ch over 2 BIS, 48_4_2 + \
             LC3 24 kHz 10 ms 60 octets (48 kbps/ch), 2 ch over 2 BIS, 24_2_2"
        );
        // Each announces its own quality (PBP): front high, rear standard
        let pbp = |g: &BroadcastGroup| {
            let adv = group_adv_data(&c, g);
            let u = UUID_PUBLIC_BROADCAST_ANNOUNCEMENT.to_le_bytes();
            let at = adv.windows(2).position(|w| w == u).unwrap();
            adv[at + 2]
        };
        assert_eq!((pbp(&g[0]), pbp(&g[1])), (PBP_HIGH_QUALITY, PBP_STANDARD_QUALITY));
        // Not a matrix, or the same preset: no BIS of its own
        c.rear_preset = find_preset("48_4_2");
        assert!(plan(&c).cis.iter().all(|b| b.preset.is_none()));
        c.layout = BroadcastLayout::Stereo;
        c.rear_preset = find_preset("24_2_2");
        assert!(plan(&c).cis.iter().all(|b| b.preset.is_none()));
    }

    #[test]
    fn a_matrix_may_send_one_pair_only() {
        let mut c = config(true, "24_2_2", None);
        c.layout = BroadcastLayout::Matrix { front: PairMode::Off, rear: PairMode::Separate };
        let p = plan(&c);
        use ChannelSource::*;
        let sources: Vec<_> = p.cis.iter().map(|b| (b.allocation(), b.channels[0].source)).collect();
        assert_eq!(sources, [(0, RearLeft), (0, RearRight)]);
        assert_eq!(p.preset.rtn, find_preset("24_2_2").unwrap().rtn);
        let names: Vec<_> = groups(&c).into_iter().map(|g| (g.name, g.bis)).collect();
        assert_eq!(names, [("BlueCaster test RL".to_string(), 0..1), ("BlueCaster test RR".to_string(), 1..2)]);
        c.layout = BroadcastLayout::Matrix { front: PairMode::Subgroups, rear: PairMode::Off };
        let g = groups(&c);
        assert_eq!((g.len(), g[0].name.as_str(), g[0].subgroup_size), (1, "BlueCaster test Front", 1));
    }

    #[test]
    fn quad_is_four_mono_broadcasts_of_the_speakers() {
        let mut c = config(true, "24_2_2", None);
        c.layout = BroadcastLayout::QUAD;
        let p = plan(&c);
        use ChannelSource::*;
        let sources: Vec<_> = p.cis.iter().map(|b| (b.cis_id, b.allocation(), b.channels[0].source)).collect();
        assert_eq!(sources, [(1, 0, Left), (1, 0, Right), (1, 0, RearLeft), (1, 0, RearRight)]);
        assert_eq!(p.preset.rtn, RTN_MANY_BIS);
        let g = groups(&c);
        let names: Vec<_> = g.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["BlueCaster test FL", "BlueCaster test FR", "BlueCaster test RL", "BlueCaster test RR"]);
        let ids: Vec<_> = g.iter().map(|g| (g.big_handle, g.sid, g.broadcast_id, g.bis.clone())).collect();
        assert_eq!(
            ids,
            [(0, 1, 0x123456, 0..1), (1, 2, 0x123457, 1..2), (2, 3, 0x123458, 2..3), (3, 4, 0x123459, 3..4)]
        );
    }

    #[test]
    fn the_bis_an_assistant_tells_of_each_output() {
        use ChannelSource::*;
        assert_eq!(BroadcastLayout::Stereo.bis_map(), [(0, 1, Left), (0, 2, Right)]);
        assert_eq!(BroadcastLayout::Split.bis_map(), [(0, 1, Left), (1, 1, Right)]);
        assert_eq!(BroadcastLayout::Mono.bis_map(), [(0, 1, Mono)]);
        let four = [(0, 1, Left), (0, 2, Right), (0, 3, RearLeft), (0, 4, RearRight)];
        for m in SingleMode::ALL {
            assert_eq!(BroadcastLayout::Single(m).bis_map(), four, "{m:?}");
        }
        assert_eq!(
            BroadcastLayout::FRONT_REAR.bis_map(),
            [(0, 1, Left), (0, 2, Right), (1, 1, RearLeft), (2, 1, RearRight)]
        );
    }

    #[test]
    fn single_pairs_are_two_subgroups_of_front_left_and_right() {
        let mut c = config(true, "32_2_2", None);
        c.layout = BroadcastLayout::Single(SingleMode::Pairs);
        let p = plan(&c);
        let locations: Vec<_> = p.cis.iter().map(|b| b.allocation()).collect();
        let (fl, fr) = (LOCATION_FRONT_LEFT, LOCATION_FRONT_RIGHT);
        assert_eq!(locations, [fl, fr, fl, fr]);
        let g = groups(&c);
        assert_eq!(g[0].subgroup_bis(), vec![0b0011, 0b1100]);
        let base = base_with(&group_plan(&p, &g[0]), g[0].subgroup_size);
        assert_eq!(base[3], 2);
        let found = |s: &str| base.windows(s.len()).filter(|w| *w == s.as_bytes()).count();
        assert_eq!((found("Front"), found("Rear")), (1, 1));
        // Located: one subgroup, the rear Back Left / Right; per BIS: four
        // subgroups, none located
        c.layout = BroadcastLayout::Single(SingleMode::Located);
        let locations: Vec<_> = plan(&c).cis.iter().map(|b| b.allocation()).collect();
        assert_eq!(locations, [fl, fr, LOCATION_BACK_LEFT, LOCATION_BACK_RIGHT]);
        assert_eq!(groups(&c)[0].subgroup_bis(), vec![0b1111]);
        c.layout = BroadcastLayout::Single(SingleMode::PerBis);
        assert!(plan(&c).cis.iter().all(|b| b.allocation() == 0));
        assert_eq!(groups(&c)[0].subgroup_bis(), vec![1, 2, 4, 8]);
    }

    #[test]
    fn single_is_one_broadcast_of_the_four_outputs() {
        let mut c = config(true, "48_4_2", None);
        c.layout = BroadcastLayout::Single(SingleMode::RearMono);
        // One configuration for its one BIG: a rear preset is not used
        c.rear_preset = find_preset("24_2_2");
        let p = plan(&c);
        use ChannelSource::*;
        let sources: Vec<_> = p.cis.iter().map(|b| (b.cis_id, b.allocation(), b.channels[0].source)).collect();
        assert_eq!(
            sources,
            [
                (1, LOCATION_FRONT_LEFT, Left),
                (2, LOCATION_FRONT_RIGHT, Right),
                (3, LOCATION_MONO, RearLeft),
                (4, LOCATION_MONO, RearRight)
            ]
        );
        assert!(p.cis.iter().all(|b| b.preset.is_none()));
        assert_eq!((p.preset.sampling_hz, p.preset.rtn), (48000, RTN_MANY_BIS));
        let g = groups(&c);
        assert_eq!(g.len(), 1);
        assert_eq!(
            (g[0].name.as_str(), g[0].bis.clone(), g[0].subgroup_bis()),
            ("BlueCaster test", 0..4, vec![0b1111])
        );
        assert_eq!(group_preset(&c, &g[0]).name, "48_4_2");
        // The BASE: one subgroup of four BIS
        let base = group_base_ad(&p, &g[0]);
        let b = &base[4..];
        assert_eq!((b[3], b[4]), (1, 4));
    }

    #[test]
    fn pairs_are_two_stereo_broadcasts_of_front_and_rear() {
        let mut c = config(true, "24_2_2", None);
        c.layout = BroadcastLayout::PAIRS;
        let p = plan(&c);
        use ChannelSource::*;
        let sources: Vec<_> = p.cis.iter().map(|b| (b.cis_id, b.allocation(), b.channels[0].source)).collect();
        assert_eq!(
            sources,
            [
                (1, LOCATION_FRONT_LEFT, Left),
                (2, LOCATION_FRONT_RIGHT, Right),
                (1, LOCATION_FRONT_LEFT, RearLeft),
                (2, LOCATION_FRONT_RIGHT, RearRight)
            ]
        );
        assert_eq!(p.preset.rtn, RTN_MANY_BIS);
        let g = groups(&c);
        let names: Vec<_> = g.iter().map(|g| (g.big_handle, g.name.as_str(), g.bis.clone())).collect();
        assert_eq!(names, [(0, "BlueCaster test Front", 0..2), (1, "BlueCaster test Rear", 2..4)]);
    }

    #[test]
    fn rear_subgroups_put_each_rear_bis_in_a_subgroup_of_its_own() {
        let mut c = config(true, "24_2_2", None);
        c.layout = BroadcastLayout::REAR_SUBGROUPS;
        let p = plan(&c);
        let g = groups(&c);
        let names: Vec<_> = g.iter().map(|g| (g.name.as_str(), g.bis.clone(), g.subgroup_size)).collect();
        assert_eq!(names, [("BlueCaster test Front", 0..2, 0), ("BlueCaster test Rear", 2..4, 1)]);
        // The front: one subgroup of two BIS, as before
        let front = base_with(&group_plan(&p, &g[0]), g[0].subgroup_size);
        assert_eq!(front, base(&group_plan(&p, &g[0])));
        assert_eq!(front[3..5], [1, 2]);
        // The rear: two subgroups of one BIS (index 1 and 2), each labelled
        let rear = base_with(&group_plan(&p, &g[1]), 1);
        assert_eq!(rear[3], 2);
        let label = |s: &str| {
            let mut v = vec![s.len() as u8 + 1, META_PROGRAM_INFO];
            v.extend_from_slice(s.as_bytes());
            v
        };
        let find = |needle: &[u8]| rear.windows(needle.len()).position(|w| w == needle);
        let (l, r) = (find(&label("Rear left")).unwrap(), find(&label("Rear right")).unwrap());
        assert!(l < r);
        // Subgroup: BIS count 1 ... metadata, then BIS index, no config
        let after_l = l + label("Rear left").len();
        assert_eq!(rear[after_l..after_l + 2], [1, 0]);
        assert_eq!(rear[rear.len() - 2..], [2, 0]);
        // As service data
        let ad = group_base_ad(&p, &g[1]);
        assert_eq!(&ad[2..4], &UUID_BASIC_AUDIO_ANNOUNCEMENT.to_le_bytes());
        assert!(ad.ends_with(&rear));
        // Both in subgroups: the front too, its BIS mono
        c.layout = BroadcastLayout::SUBGROUPS;
        let p = plan(&c);
        let g = groups(&c);
        assert!(g.iter().all(|g| g.subgroup_size == 1));
        assert!(p.cis.iter().all(|b| b.allocation() == 0));
        let front = base_with(&group_plan(&p, &g[0]), 1);
        assert_eq!(front[3], 2);
        let found = |b: &[u8], s: &str| b.windows(s.len()).any(|w| w == s.as_bytes());
        assert!(found(&front, "Front left") && found(&front, "Front right") && !found(&front, "Rear"));
    }

    #[test]
    fn front_rear_is_a_stereo_front_and_two_mono_rears() {
        let mut c = config(true, "48_4_2", None);
        c.layout = BroadcastLayout::FRONT_REAR;
        let p = plan(&c);
        use ChannelSource::*;
        let sources: Vec<_> = p.cis.iter().map(|b| (b.cis_id, b.allocation(), b.channels[0].source)).collect();
        assert_eq!(
            sources,
            [(1, LOCATION_FRONT_LEFT, Left), (2, LOCATION_FRONT_RIGHT, Right), (1, 0, RearLeft), (1, 0, RearRight)]
        );
        let g = groups(&c);
        let names: Vec<_> = g.iter().map(|g| (g.name.as_str(), g.bis.clone())).collect();
        assert_eq!(
            names,
            [("BlueCaster test Front", 0..2), ("BlueCaster test RL", 2..3), ("BlueCaster test RR", 3..4)]
        );
        // The front's BASE: one subgroup of two BIS
        let base = base(&group_plan(&p, &g[0]));
        assert_eq!(base[3..5], [1, 2]);
        // Two BIS or fewer keep the preset's RTN
        c.layout = BroadcastLayout::Split;
        assert_eq!(plan(&c).preset.rtn, 4);
    }

    #[test]
    fn split_left_and_right_into_broadcasts_of_their_own() {
        let mut c = config(true, "24_2_2", None);
        c.layout = BroadcastLayout::Split;
        let p = plan(&c);
        let sources: Vec<_> = p.cis.iter().map(|b| (b.cis_id, b.allocation(), b.channels[0].source)).collect();
        assert_eq!(sources, [(1, 0, ChannelSource::Left), (1, 0, ChannelSource::Right)]);
        let g = groups(&c);
        assert_eq!(g.len(), 2);
        assert_eq!(
            (g[0].big_handle, g[0].sid, g[0].name.as_str(), g[0].bis.clone()),
            (0, 1, "BlueCaster test L", 0..1)
        );
        assert_eq!(
            (g[1].big_handle, g[1].sid, g[1].name.as_str(), g[1].bis.clone()),
            (1, 2, "BlueCaster test R", 1..2)
        );
        assert_eq!((g[0].broadcast_id, g[1].broadcast_id), (0x123456, 0x123457));
        // Each announces its own name and ID, and a BASE of one mono BIS
        let adv = group_adv_data(&c, &g[1]);
        assert_eq!(&adv[4..7], &[0x57, 0x34, 0x12]);
        assert!(adv.ends_with(b"BlueCaster test R"));
        let base = base(&group_plan(&p, &g[1]));
        assert_eq!(base[3..5], [1, 1]); // one subgroup of one BIS
        assert_eq!(base[base.len() - 2..], [1, 0]); // BIS 1, no allocation
        // A long name keeps room for the suffix
        c.name = "x".repeat(NAME_MAX);
        assert!(groups(&c).iter().all(|g| g.name.len() == NAME_MAX && g.name.ends_with(['L', 'R'])));
        // The IDs differ, also at the top of the range
        for (id, right) in [(0x00FF_FFFD, 0x00FF_FFFE), (0x00FF_FFFE, 1)] {
            c.broadcast_id = id;
            let g = groups(&c);
            assert_eq!((g[0].broadcast_id, g[1].broadcast_id), (id, right));
        }
        // Mono or plain stereo: one broadcast as before
        c.layout = BroadcastLayout::Stereo;
        assert_eq!(groups(&c).len(), 1);
        assert_eq!(groups(&c)[0].bis, 0..2);
    }

    #[test]
    fn names_and_codes() {
        assert_eq!(check_name("  Living room  ").unwrap(), "Living room");
        assert!(check_name("abc").is_err());
        // Cut to 32 octets at a character boundary (3 octets each)
        let long = "あ".repeat(20);
        let cut = check_name(&long).unwrap();
        assert_eq!(cut.len(), 30);
        assert!(long.starts_with(&cut));

        let c = broadcast_code("1234abcd").unwrap();
        assert_eq!(&c[..8], b"1234abcd");
        assert!(c[8..].iter().all(|&b| b == 0));
        assert!(broadcast_code("123").is_err());
        assert!(broadcast_code("12345678901234567").is_err());
        assert!(broadcast_code("1234567890123456").is_ok());

        assert_eq!(broadcast_id(0), 1);
        assert!(broadcast_id(u64::MAX) <= 0xFFFFFE);
        assert_eq!(broadcast_id(0x42), 0x42);
    }

    #[test]
    fn announcements() {
        let c = config(true, DEFAULT_PRESET, None);
        let adv = adv_data(&c);
        assert_eq!(
            adv[..14],
            [
                6,
                0x16,
                0x52,
                0x18,
                0x56,
                0x34,
                0x12, // Broadcast_ID
                5,
                0x16,
                0x56,
                0x18,
                PBP_HIGH_QUALITY,
                0, // PBP, no metadata
                16
            ]
        );
        assert_eq!(adv[14], AD_BROADCAST_NAME);
        assert_eq!(&adv[15..], b"BlueCaster test");

        let c = config(false, STANDARD_PRESET, Some("1234"));
        assert_eq!(adv_data(&c)[11], PBP_ENCRYPTED | PBP_STANDARD_QUALITY);
        assert_eq!(pbp_features(&config(true, "32_2_2", None)), 0);
    }

    #[test]
    fn base_of_stereo_and_mono() {
        let plan = plan(&config(true, DEFAULT_PRESET, None));
        assert_eq!(describe(&plan), "LC3 48 kHz 10 ms 120 octets (96 kbps/ch), 2 ch over 2 BIS, 48_4_2");
        assert_eq!(plan.cis.iter().map(|b| b.cis_id).collect::<Vec<_>>(), [1, 2]);
        let base = base(&plan);
        #[rustfmt::skip]
        assert_eq!(base, [
            0x40, 0x9C, 0x00,             // 40 ms
            1,                            // subgroups
            2, 0x06, 0, 0, 0, 0,          // BIS, LC3
            10, 2, 1, 8, 2, 2, 1, 3, 4, 120, 0,
            4, 3, 2, 4, 0,                // Streaming Audio Contexts: Media
            1, 6, 5, 3, 1, 0, 0, 0,       // BIS 1: Front Left
            2, 6, 5, 3, 2, 0, 0, 0,       // BIS 2: Front Right
        ]);
        let ad = base_ad(&plan);
        assert_eq!(ad[0] as usize, ad.len() - 1);
        assert_eq!(ad[1..4], [0x16, 0x51, 0x18]);
        assert_eq!(&ad[4..], &base[..]);

        let mono = super::plan(&config(false, "16_2_2", None));
        assert_eq!(mono.cis.len(), 1);
        assert_eq!(mono.cis[0].channels[0].source, ChannelSource::Mono);
        let base = super::base(&mono);
        assert_eq!(base[4], 1);
        assert_eq!(base[base.len() - 2..], [1, 0]); // BIS 1, no allocation
    }

    #[test]
    fn the_advertiser_address_and_subgroups_of_a_broadcast() {
        let g = |big_handle, n, subgroup_size| BroadcastGroup {
            big_handle,
            sid: 1 + big_handle,
            name: String::new(),
            broadcast_id: 1,
            bis: 0..n,
            subgroup_size,
        };
        let public = [0x00, 0x1A, 0x7D, 0xDA, 0x71, 0x13];
        assert_eq!(g(0, 2, 0).advertiser_address(public), (public, false));
        assert_eq!(g(1, 1, 0).advertiser_address(public), ([0xC0, 0x1A, 0x7D, 0xDA, 0x71, 0x13 ^ 0x5B], true));
        assert_eq!(g(0, 1, 0).advertiser_address([0; 6]), ([0xC2, 0xA2, 0xD0, 0, 0, 0x01 ^ 0x5A], true));
        assert_eq!(g(0, 2, 0).subgroup_bis(), vec![0b11]);
        assert_eq!(g(0, 2, 1).subgroup_bis(), vec![0b01, 0b10]);
        assert_eq!(g(0, 4, 2).subgroup_bis(), vec![0b0011, 0b1100]);
    }

    #[test]
    fn presets_fit_one_iso_packet() {
        for p in presets() {
            assert_eq!(p.frame_us, 10000);
            assert_eq!(p.rtn, 4);
            assert!(p.octets <= 155);
        }
        assert!(find_preset(DEFAULT_PRESET).is_some());
        assert!(find_preset(STANDARD_PRESET).is_some());
        assert!(find_preset("48_4_1").is_none());
    }
}

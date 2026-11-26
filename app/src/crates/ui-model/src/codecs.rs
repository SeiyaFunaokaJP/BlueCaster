//! The codecs a profile can ask for and what each allows, as v1's profile
//! dialog (wx_profile_dialog.cpp on the develop branch, update_codec_dependent): quality
//! modes with their bitrates (since issue #86 High / Medium / Low for LDAC
//! and LHDC V5, LHDC V5's Medium-High too (issue #203), High / Low Latency for aptX Adaptive, only High, the highest
//! bit rate the device takes, for the others; core: Quality::for_codec),
//! ABR only for LDAC / LHDC V5 (and Auto, which may
//! pick them; v2 adds SBC / SBC XQ's adaptive bitpool), 24-bit and 88.2 / 96 kHz only for LDAC and LHDC V5, 192 kHz and
//! "mhq" only for LHDC V5, no quality choice for the aptX family (but aptX
//! Adaptive) or under LHDC V5 ABR, 48 kHz only for Opus and Opus 05, 48 / 96
//! kHz for aptX Adaptive, whose "ll" quality is its low latency mode (on
//! the Codecs screen a card of its own, aptX Adaptive LL: issue #86).
//! aptX HD and aptX Adaptive always carry 24-bit (no choice).
//!
//! Codec ids are `&'static str`: "auto", "ldac", "aptxhd", "aptxll", "aptx",
//! "aptxad", "aptxadll", "lhdcv5", "opus", "aac", "sbcxq", "sbc",
//! "faststream", "opus05".

use ipc::{DevOverrides, Profile, Settings};

use crate::sbc::SbcCaps;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QualityMode {
    pub id: &'static str,
    /// Translation key of the name
    pub label: &'static str,
    pub kbps: Option<&'static str>,
}

/// 1..5 for the comparison bars
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bars {
    pub quality: u8,
    pub latency: u8,
    pub stability: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodecSpec {
    pub id: &'static str,
    /// Translation key of the name
    pub label: &'static str,
    /// 1..5 for the comparison bars (Auto has none: it depends on the device)
    pub bars: Option<Bars>,
    /// Quality modes; empty: the codec has a fixed bitrate
    pub qualities: &'static [QualityMode],
    pub abr: bool,
    /// 24-bit and 88.2 / 96 kHz
    pub hi_res: bool,
    /// The only capture rates it takes (0 = auto), when not the usual ones
    pub rates: Option<&'static [u32]>,
    /// Its one bit rate at 44.1 / 48 kHz when it has no qualities (core:
    /// Encoder::bitrate_kbps)
    pub fixed_kbps: Option<[u32; 2]>,
    /// The bit depth it always carries when there is no choice (16 if not set)
    pub bits: Option<u32>,
    /// Variable bit rate (where the headphones list it), on the "abr" setting
    pub vbr: bool,
    /// The SBC bitpool and frame setup (blocks, subbands, allocation) can be chosen
    pub bitpool: bool,
    /// Shown and used only in developer mode (core: CodecKind::developer_only):
    /// outside it, a device's is left out as if it did not offer it
    pub developer: bool,
}

/// SBC's largest bitpool (the local SEP's)
pub const BITPOOL_MAX: u32 = 53;
/// SBC XQ is dual channel: its largest bitpool per channel (44.1 kHz; 43 at 48 kHz)
pub const BITPOOL_MAX_XQ: u32 = 47;
pub const BITPOOL_MIN: u32 = 2;

const HQ: &str = "quality.high";
const MEDIUM_HIGH: &str = "quality.medium_high";
const MEDIUM: &str = "quality.medium";
const LOW: &str = "quality.low";
const LL: &str = "quality.low_latency";

const fn q(id: &'static str, label: &'static str, kbps: Option<&'static str>) -> QualityMode {
    QualityMode { id, label, kbps }
}

const fn bars(quality: u8, latency: u8, stability: u8) -> Option<Bars> {
    Some(Bars { quality, latency, stability })
}

const BASE: CodecSpec = CodecSpec {
    id: "",
    label: "",
    bars: None,
    qualities: &[],
    abr: false,
    hi_res: false,
    rates: None,
    fixed_kbps: None,
    bits: None,
    vbr: false,
    bitpool: false,
    developer: false,
};

pub const CODECS: &[CodecSpec] = &[
    CodecSpec {
        id: "auto",
        label: "codec.auto",
        // Auto-select takes the best options (issue #86)
        qualities: &[q("hq", HQ, None)],
        abr: true,
        ..BASE
    },
    CodecSpec {
        id: "ldac",
        label: "codec.ldac",
        bars: bars(5, 2, 2),
        qualities: &[q("hq", HQ, Some("990")), q("sq", MEDIUM, Some("660")), q("mq", LOW, Some("330"))],
        abr: true,
        hi_res: true,
        ..BASE
    },
    CodecSpec {
        id: "lhdcv5",
        label: "codec.lhdc_v5",
        bars: bars(5, 2, 2),
        // Android's four fixed levels (the sink may cap the maximum). Medium-
        // High was left out in issue #86 and is back since issue #203
        qualities: &[
            q("hq", HQ, Some("1000")),
            q("mhq", MEDIUM_HIGH, Some("900")),
            q("sq", MEDIUM, Some("500")),
            q("mq", LOW, Some("400")),
        ],
        abr: true,
        hi_res: true,
        // No 88.2 kHz: the device lists 44.1, 48, 96 and 192 kHz only (issue #86)
        rates: Some(&[0, 44100, 48000, 96000, 192000]),
        ..BASE
    },
    CodecSpec {
        id: "aptxhd",
        label: "codec.aptx_hd",
        bars: bars(4, 3, 3),
        fixed_kbps: Some([529, 576]),
        bits: Some(24),
        ..BASE
    },
    CodecSpec { id: "aptx", label: "codec.aptx", bars: bars(3, 3, 4), fixed_kbps: Some([352, 384]), ..BASE },
    CodecSpec { id: "aptxll", label: "codec.aptx_ll", bars: bars(2, 5, 4), fixed_kbps: Some([352, 384]), ..BASE },
    // aptX Adaptive: High with an R1 sink
    // (an R2 sink: 427 -> 340), 48 kHz; ABR steps down from High; Low
    // Latency has a fixed bit rate. 96 kHz when chosen, to a device that
    // lists it (an R2 one), not in Low Latency (48 kHz otherwise; 646
    // kbps). Always 24-bit.
    CodecSpec {
        id: "aptxad",
        label: "codec.aptx_adaptive",
        bars: bars(4, 3, 4),
        qualities: &[q("hq", HQ, Some("427")), q("ll", LL, Some("265"))],
        abr: true,
        rates: Some(&[0, 48000, 96000]),
        bits: Some(24),
        ..BASE
    },
    CodecSpec {
        id: "opus",
        label: "codec.opus",
        bars: bars(4, 3, 4),
        // Android's A2DP Opus: 256 kbps stereo (mono: half), 48 kHz, 20 ms frames
        qualities: &[q("hq", HQ, Some("256"))],
        rates: Some(&[0, 48000]),
        ..BASE
    },
    CodecSpec {
        id: "aac",
        label: "codec.aac",
        bars: bars(3, 2, 4),
        qualities: &[q("hq", HQ, Some("256"))],
        vbr: true,
        ..BASE
    },
    // Dual channel, high bitpool (PipeWire's SBC-XQ); only on sinks whose
    // SBC allows it
    CodecSpec {
        id: "sbcxq",
        label: "codec.sbc_xq",
        bars: bars(3, 3, 3),
        // High: the highest bitpool the device takes (or the one set)
        qualities: &[q("hq", HQ, Some("~552"))],
        // Adaptive bitpool, down to Mobile's (core: SBC ABR only when the
        // profile asks for SBC / SBC XQ itself)
        abr: true,
        bitpool: true,
        ..BASE
    },
    CodecSpec {
        id: "sbc",
        label: "codec.sbc",
        bars: bars(2, 3, 5),
        qualities: &[q("hq", HQ, Some("~345"))],
        abr: true,
        bitpool: true,
        ..BASE
    },
    // CSR FastStream: fixed SBC (bitpool 29, ~200 kbps), music only; never
    // picked by Auto
    CodecSpec {
        id: "faststream",
        label: "codec.faststream",
        bars: bars(1, 4, 4),
        fixed_kbps: Some([198, 216]),
        ..BASE
    },
    // PipeWire's own Opus ("Opus 05"; never picked by Auto):
    // Android Opus' levels, stereo, 48 kHz, 10 ms frames when offered
    CodecSpec {
        id: "opus05",
        label: "codec.opus05",
        bars: bars(4, 3, 3),
        qualities: &[q("hq", HQ, Some("256"))],
        rates: Some(&[0, 48000]),
        developer: true,
        ..BASE
    },
];

/// aptX Adaptive's low latency mode as a card of its own (issue #86), the
/// codec cards' only one that is not a codec: chosen, the profile asks for
/// aptX Adaptive with quality "ll" (what earlier versions saved for it
/// loads as this card). 48 kHz, 24-bit, its one bit rate; no ABR.
pub const APTXAD_LL: CodecSpec = CodecSpec {
    id: "aptxadll",
    label: "codec.aptx_adaptive_ll",
    bars: bars(3, 5, 3),
    fixed_kbps: Some([265, 265]),
    rates: Some(&[0, 48000]),
    bits: Some(24),
    ..BASE
};

pub fn codec_spec(id: &str) -> &'static CodecSpec {
    if id == APTXAD_LL.id {
        return &APTXAD_LL;
    }
    CODECS.iter().find(|c| c.id == id).unwrap_or(&CODECS[0])
}

/// The codec a card is (what the device lists and the core streams):
/// aptX Adaptive LL's is aptX Adaptive
pub fn codec_key(card: &str) -> &str {
    if card == APTXAD_LL.id { "aptxad" } else { card }
}

/// The card a profile's codec shows on: aptX Adaptive in Low Latency is
/// aptX Adaptive LL's
pub fn card_of<'a>(codec: &'a str, quality: &str) -> &'a str {
    if codec == "aptxad" && quality == "ll" { APTXAD_LL.id } else { codec }
}

/// What the codec settings are about, in a profile ([`Profile`]) or in a
/// codec's "..." dialog ([`CodecChoice`])
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecChoice {
    pub codec: String,
    pub quality: String,
    pub abr: bool,
    pub sample_rate: u32,
    pub bit_depth: u32,
    pub sbc_bitpool: u8,
    pub sbcxq_bitpool: u8,
    pub sbc_min_bitpool: u8,
    pub sbcxq_min_bitpool: u8,
    pub sbc_blocks: u8,
    pub sbc_subbands: u8,
    pub sbc_allocation: String,
    pub sbc_channel_mode: String,
    pub unlisted_options: bool,
    pub lhdcv5_low_latency: bool,
}

/// Something with the codec settings ([`CodecChoice`]'s fields), so the
/// helpers work on a profile and on a dialog's values alike
pub trait CodecFields: Clone {
    fn codec_choice(&self) -> CodecChoice;
    fn set_codec_choice(&mut self, c: CodecChoice);
}

impl CodecFields for CodecChoice {
    fn codec_choice(&self) -> CodecChoice {
        self.clone()
    }
    fn set_codec_choice(&mut self, c: CodecChoice) {
        *self = c;
    }
}

impl From<&Profile> for CodecChoice {
    fn from(p: &Profile) -> Self {
        CodecChoice {
            codec: p.codec.clone(),
            quality: p.quality.clone(),
            abr: p.abr,
            sample_rate: p.sample_rate,
            bit_depth: p.bit_depth,
            sbc_bitpool: p.sbc_bitpool,
            sbcxq_bitpool: p.sbcxq_bitpool,
            sbc_min_bitpool: p.sbc_min_bitpool,
            sbcxq_min_bitpool: p.sbcxq_min_bitpool,
            sbc_blocks: p.sbc_blocks,
            sbc_subbands: p.sbc_subbands,
            sbc_allocation: p.sbc_allocation.clone(),
            sbc_channel_mode: p.sbc_channel_mode.clone(),
            unlisted_options: p.unlisted_options,
            lhdcv5_low_latency: p.lhdcv5_low_latency,
        }
    }
}

impl CodecFields for Profile {
    fn codec_choice(&self) -> CodecChoice {
        CodecChoice::from(self)
    }
    fn set_codec_choice(&mut self, c: CodecChoice) {
        self.codec = c.codec;
        self.quality = c.quality;
        self.abr = c.abr;
        self.sample_rate = c.sample_rate;
        self.bit_depth = c.bit_depth;
        self.sbc_bitpool = c.sbc_bitpool;
        self.sbcxq_bitpool = c.sbcxq_bitpool;
        self.sbc_min_bitpool = c.sbc_min_bitpool;
        self.sbcxq_min_bitpool = c.sbcxq_min_bitpool;
        self.sbc_blocks = c.sbc_blocks;
        self.sbc_subbands = c.sbc_subbands;
        self.sbc_allocation = c.sbc_allocation;
        self.sbc_channel_mode = c.sbc_channel_mode;
        self.unlisted_options = c.unlisted_options;
        self.lhdcv5_low_latency = c.lhdcv5_low_latency;
    }
}

/// A profile's codec and quality for a card: aptX Adaptive LL's are aptX
/// Adaptive and Low Latency, aptX Adaptive's own never Low Latency
pub fn card_codec<T: CodecFields>(p: &T) -> T {
    let mut c = p.codec_choice();
    if c.codec == APTXAD_LL.id {
        c.codec = "aptxad".into();
        c.quality = "ll".into();
    } else if c.codec == "aptxad" && c.quality == "ll" {
        c.quality = "hq".into();
    } else {
        return p.clone();
    }
    let mut out = p.clone();
    out.set_codec_choice(c);
    out
}

/// Translation key of a codec name as the core reports it ("LDAC", "aptX HD") or a profile id
pub fn codec_label_key(name: &str) -> &'static str {
    let n: String =
        name.to_lowercase().chars().filter(|&c| !(crate::format::js_space(c) || c == '_' || c == '-')).collect();
    match n.as_str() {
        "auto" => "codec.auto",
        "ldac" => "codec.ldac",
        "aptx" => "codec.aptx",
        "aptxhd" => "codec.aptx_hd",
        "aptxll" | "aptxlowlatency" => "codec.aptx_ll",
        "aptxad" | "aptxadaptive" => "codec.aptx_adaptive",
        "aac" => "codec.aac",
        "sbc" => "codec.sbc",
        "sbcxq" => "codec.sbc_xq",
        "faststream" => "codec.faststream",
        "lhdcv5" | "lhdc" => "codec.lhdc_v5",
        "opus" => "codec.opus",
        "opus05" => "codec.opus05",
        // LE Audio (StreamInfo.le)
        "lc3" => "ui.codec.lc3",
        _ => "",
    }
}

/// Whether a device offers a codec, from the codec keys the core learnt when
/// it last connected (Device.codecs: "ldac", "aac", ...; empty: not known
/// until the first connection). Auto takes what the device offers. One it may
/// offer only once chosen (`on_request`, Device.codecs_on_request) is not known
/// until then (issue #30).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecSupport {
    Offered,
    NotOffered,
    Unknown,
}

pub fn codec_support<S: AsRef<str>, R: AsRef<str>>(codec: &str, known: &[S], on_request: &[R]) -> CodecSupport {
    if known.is_empty() {
        return CodecSupport::Unknown;
    }
    if codec == "auto" {
        return CodecSupport::Offered;
    }
    let id = codec_key(codec).to_lowercase();
    if known.iter().any(|k| k.as_ref().to_lowercase() == id) {
        return CodecSupport::Offered;
    }
    if on_request.iter().any(|k| k.as_ref() == id) { CodecSupport::Unknown } else { CodecSupport::NotOffered }
}

/// What the developer-mode overrides go by (Pick<Settings,
/// "developer_mode" | "dev_overrides">)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DevMode {
    pub developer_mode: bool,
    pub dev_overrides: DevOverrides,
}

impl From<&Settings> for DevMode {
    fn from(s: &Settings) -> Self {
        DevMode { developer_mode: s.developer_mode, dev_overrides: s.dev_overrides }
    }
}

/// Developer mode's override: each codec's "..." settings apply under
/// Auto-select too, and open without turning it off
pub fn auto_keeps_codec_settings(s: DevMode) -> bool {
    s.developer_mode && s.dev_overrides.auto_codec_settings
}

/// Developer mode's override: every codec runs at its quality's bit rate,
/// whatever maximum the device lists (core: DevOverrides::ignore_sink_max)
pub fn ignores_sink_max(s: DevMode) -> bool {
    s.developer_mode && s.dev_overrides.ignore_sink_max
}

/// What a device's endpoint of a codec lists (Device.codec_caps)
pub type CodecCaps = ipc::CodecDeviceCaps;

/// What the codec settings go by: without the device's maximum bit rates
/// while developer mode's override ignores them, so every quality is offered
pub fn caps_for_settings(caps: &[CodecCaps], s: DevMode) -> Vec<CodecCaps> {
    if !ignores_sink_max(s) {
        return caps.to_vec();
    }
    caps.iter().map(|c| CodecCaps { max_bitrate: 0, ..c.clone() }).collect()
}

/// The SBC setup the codec settings go by: no device maximum bitpool while
/// the override ignores it (the device's minimum still applies, as in the
/// core)
pub fn sbc_caps_for_settings(caps: Option<&SbcCaps>, s: DevMode) -> Option<SbcCaps> {
    let caps = caps?;
    if caps.max_bitpool < 2 || !ignores_sink_max(s) {
        return Some(caps.clone());
    }
    Some(SbcCaps { max_bitpool: 255, ..caps.clone() })
}

/// Auto's choice, highest sound quality first (core: CodecKind::AUTO_PRIORITY,
/// issue #86). FastStream and Opus 05 are used only when chosen.
pub const AUTO_ORDER: [&str; 10] =
    ["lhdcv5", "ldac", "aptxad", "opus", "aptxhd", "aptx", "aptxll", "aac", "sbcxq", "sbc"];

/// The codec keys of `keys` that Auto and the fallback take (those of
/// AUTO_ORDER) in the order given, then any missing in their default order
/// (core: AutoOrder::from_keys). Empty: AUTO_ORDER, highest sound quality
/// first.
pub fn auto_order<S: AsRef<str>>(keys: &[S]) -> Vec<&'static str> {
    let mut order: Vec<&'static str> = Vec::new();
    let given = keys.iter().map(|k| k.as_ref().to_lowercase());
    for k in given.chain(AUTO_ORDER.iter().map(|k| (*k).to_owned())) {
        if let Some(c) = AUTO_ORDER.iter().find(|c| **c == k)
            && !order.contains(c)
        {
            order.push(c);
        }
    }
    order
}

/// The default order of a device's codec cards: AUTO_ORDER, with Opus 05
/// beside Opus and FastStream last (Auto and the fallback never take those
/// two)
pub const CARD_ORDER: [&str; 13] = [
    "lhdcv5",
    "ldac",
    "aptxad",
    "aptxadll",
    "opus",
    "opus05",
    "aptxhd",
    "aptx",
    "aptxll",
    "aac",
    "sbcxq",
    "sbc",
    "faststream",
];

/// A device's codec cards in its order (the profile's codec_order, issue #86
/// round 8: dragged with Auto-select off): the codecs given (each once,
/// unknown keys left out), then any missing in CARD_ORDER's order (as the
/// core's AutoOrder::from_keys). Empty: CARD_ORDER.
pub fn card_order<S: AsRef<str>>(keys: &[S]) -> Vec<&'static str> {
    let mut order: Vec<&'static str> = Vec::new();
    for k in keys.iter().map(|k| k.as_ref().to_lowercase()) {
        if let Some(c) = CARD_ORDER.iter().find(|c| **c == k)
            && !order.contains(c)
        {
            order.push(c);
        }
    }
    // aptX Adaptive LL, new in an order saved before it: beside aptX Adaptive
    if !order.contains(&"aptxadll")
        && let Some(i) = order.iter().position(|c| *c == "aptxad")
    {
        order.insert(i + 1, "aptxadll");
    }
    for k in CARD_ORDER {
        if !order.contains(&k) {
            order.push(k);
        }
    }
    order
}

/// The cards' order is the default one
pub fn is_default_order<S: AsRef<str>>(keys: &[S]) -> bool {
    card_order(keys).iter().zip(CARD_ORDER.iter()).all(|(c, d)| c == d)
}

/// `list` with the item at `from` moved to `to`
pub fn moved<T: Clone>(list: &[T], from: usize, to: usize) -> Vec<T> {
    let mut next = list.to_vec();
    if from < next.len() {
        let item = next.remove(from);
        next.insert(to.min(next.len()), item);
    }
    next
}

/// Shown and used only in developer mode (CodecSpec.developer)
pub fn developer_only(codec: &str) -> bool {
    codec_spec(codec).developer
}

/// The codecs of `list` the app offers: those of developer mode only with it on
pub fn offered_codecs<T: AsRef<str> + Clone>(list: &[T], developer: bool) -> Vec<T> {
    list.iter().filter(|c| developer || !developer_only(c.as_ref())).cloned().collect()
}

/// Offered by the device
fn usable<S: AsRef<str>>(codec: &str, known: &[S]) -> bool {
    codec_support::<S, &str>(codec, known, &[]) == CodecSupport::Offered
}

/// The codec Auto picks among those a device offers (`known`, as
/// Device.codecs): the first in `order` (the device's, AUTO_ORDER unless
/// given: pass `&AUTO_ORDER`; never FastStream nor Opus 05); None when not known
pub fn auto_pick<S: AsRef<str>, O: AsRef<str>>(known: &[S], order: &[O]) -> Option<&'static str> {
    auto_order(order).into_iter().find(|c| usable(c, known))
}

/// The codec a connection uses for `codec` chosen on a device that offers
/// `known` (core: caps::choose_in with the fallback every profile has): the
/// codec itself when offered, else the first in the device's `order`
/// (AUTO_ORDER unless given: pass `&AUTO_ORDER`) it offers, as Auto picks (issue #86 round 8:
/// not the next one below the codec chosen). None when the device's codecs
/// are not known yet.
pub fn effective_codec<S: AsRef<str>, O: AsRef<str>>(codec: &str, known: &[S], order: &[O]) -> Option<String> {
    if known.is_empty() {
        return None;
    }
    if codec != "auto" && usable(codec, known) {
        return Some(codec.to_owned());
    }
    auto_pick(known, order).map(str::to_owned)
}

pub fn sample_rates(codec: &str) -> Vec<u32> {
    let spec = codec_spec(codec);
    match spec.rates {
        Some(r) => r.to_vec(),
        None if spec.hi_res => vec![0, 44100, 48000, 88200, 96000],
        None => vec![0, 44100, 48000],
    }
}

/// What a codec's settings go by with its card's "Show only supported
/// options" off (issue #86): everything the codec has, as when the device's
/// caps are not known (core: RemoteCaps::unlisted). aptX Adaptive's 96 kHz
/// stays the device's: it needs an R2 bitstream
pub fn unlisted_caps<'a>(codec: &str, caps: Option<&'a CodecCaps>) -> Option<&'a CodecCaps> {
    if codec_key(codec) == "aptxad" { caps } else { None }
}

/// The caps of `codec` among a device's (None: not known)
pub fn caps_of<'a>(caps: &'a [CodecCaps], codec: &str) -> Option<&'a CodecCaps> {
    let key = codec_key(codec).to_lowercase();
    caps.iter().find(|c| c.codec.to_lowercase() == key)
}

/// Whether the device lists a sample rate (anything goes while not known).
/// aptX Adaptive's 96 kHz only when
/// the device lists it: few do (an R2 one with a 96 kHz bit; issue #86, a
/// Bose QC Ultra has none), and the core streams 48 kHz to the others
pub fn rate_offered(codec: &str, rate: u32, caps: Option<&CodecCaps>) -> bool {
    if rate == 0 {
        return true;
    }
    if codec == "aptxad" && rate == 96000 {
        return caps.is_some_and(|c| c.sample_rates.contains(&rate));
    }
    match caps {
        Some(c) if !c.sample_rates.is_empty() => c.sample_rates.contains(&rate),
        _ => true,
    }
}

/// The one value a setting can take when there is nothing to choose (Auto
/// could only pick it): shown as a fixed value, without Auto. None when
/// there is a choice, or when the value kept is one the device did not list
/// (the list stays, to move off it)
pub fn only_choice<T: PartialEq + Clone>(offered: &[T], current: &T, auto: &T) -> Option<T> {
    let values: Vec<&T> = offered.iter().filter(|x| *x != auto).collect();
    (values.len() == 1 && (current == auto || current == values[0])).then(|| values[0].clone())
}

/// The sample rates the dialog offers (issue #86): the codec's, as far as
/// the device lists them
pub fn offered_sample_rates(codec: &str, caps: Option<&CodecCaps>) -> Vec<u32> {
    sample_rates(codec).into_iter().filter(|&r| rate_offered(codec, r, caps)).collect()
}

/// The sample rate Auto streams `codec` at, shown beside "Auto" (issue
/// #86), as the core picks it (stream.rs): 48 kHz, else what the device
/// lists, 44.1 kHz first; SBC, SBC XQ and Opus always 48 kHz
pub fn auto_sample_rate(codec: &str, caps: Option<&CodecCaps>) -> u32 {
    if ["sbc", "sbcxq", "opus", "opus05"].contains(&codec) {
        return 48000;
    }
    let listed: Vec<u32> = offered_sample_rates(codec, caps).into_iter().filter(|&r| r > 0).collect();
    if listed.is_empty() || listed.contains(&48000) {
        return 48000;
    }
    if listed.contains(&44100) { 44100 } else { listed[0] }
}

/// The bit depth Auto streams `codec` with (stream.rs): 24-bit for LDAC,
/// aptX HD and aptX Adaptive, LHDC V5 when the device lists it, else 16
pub fn auto_bit_depth(codec: &str, caps: Option<&CodecCaps>) -> u32 {
    if codec == "ldac" || codec == "aptxhd" || codec_key(codec) == "aptxad" {
        return 24;
    }
    if codec == "lhdcv5" {
        let listed = caps.map(|c| c.bit_depths.as_slice()).unwrap_or_default();
        return if listed.is_empty() || listed.contains(&24) { 24 } else { 16 };
    }
    16
}

/// Of `listed` (lowest first), the rate nearest the PC's `source` rate, as
/// the core picks it (caps.rs nearest_rate): itself, else its family (44.1
/// kHz multiples or not) below, then above; else the other family above,
/// then below
pub fn nearest_rate(listed: &[u32], source: u32) -> u32 {
    let family = |r: u32| r.is_multiple_of(11025);
    let same: Vec<u32> = listed.iter().copied().filter(|&r| family(r) == family(source)).collect();
    let other: Vec<u32> = listed.iter().copied().filter(|&r| family(r) != family(source)).collect();
    let below = |rs: &[u32]| rs.iter().copied().rfind(|&r| r <= source);
    let above = |rs: &[u32]| rs.iter().copied().find(|&r| r >= source);
    below(&same).or_else(|| above(&same)).or_else(|| above(&other)).or_else(|| below(&other)).unwrap_or(source)
}

/// The sample rate Auto streams `codec` at while it matches the PC's
/// output (developer mode), as the core picks it (stream.rs)
pub fn matched_sample_rate(codec: &str, caps: Option<&CodecCaps>, source: u32) -> u32 {
    let listed: Vec<u32> = offered_sample_rates(codec, caps).into_iter().filter(|&r| r > 0).collect();
    nearest_rate(&listed, source)
}

/// The bit depth Auto streams `codec` with while it matches the PC's
/// output (developer mode): 16-bit from a 16-bit output, else Auto's
pub fn matched_bit_depth(codec: &str, caps: Option<&CodecCaps>, source: u32) -> u32 {
    if source <= 16 && codec_key(codec) != "aptxad" { 16 } else { auto_bit_depth(codec, caps) }
}

/// The bit depths the dialog offers: the codec's, as far as the device
/// lists them (LHDC V5 says)
pub fn offered_bit_depths(codec: &str, caps: Option<&CodecCaps>) -> Vec<u32> {
    let listed = caps.map(|c| c.bit_depths.as_slice()).unwrap_or_default();
    bit_depths(codec).into_iter().filter(|b| *b == 0 || listed.is_empty() || listed.contains(b)).collect()
}

/// The profile's ABR flag means AAC's VBR under AAC, the adaptive bitpool
/// under SBC and under SBC XQ (each its own: issue #86, SBC's turned XQ's on),
/// and LDAC's / LHDC V5's ABR elsewhere: switching between those leaves it
/// off rather than carrying it over (an LDAC profile with ABR moved to SBC
/// does not start adapting its bitpool unasked).
pub fn abr_carries(from: &str, to: &str) -> bool {
    let family = |c: &str| if matches!(c, "aac" | "sbc" | "sbcxq") { c.to_owned() } else { "abr".to_owned() };
    family(from) == family(to)
}

/// A codec's starting values from another codec's (`p.codec`): what does
/// not carry over (ABR across families, SBC's frame setup to SBC XQ and
/// back) is left at the default, then the values are fitted to the codec
pub fn carry_to_codec<T: CodecFields>(p: &T, to: &str) -> T {
    let mut c = p.codec_choice();
    let same = c.codec == to;
    c.abr = abr_carries(&c.codec, to) && c.abr;
    c.codec = to.to_owned();
    // Each codec's own: its card's "Show only supported options"
    if !same {
        c.unlisted_options = false;
        c.sbc_blocks = 0;
        c.sbc_subbands = 0;
        c.sbc_allocation.clear();
    }
    let mut out = p.clone();
    out.set_codec_choice(c);
    fit_to_codec(&out)
}

/// LHDC V5 low latency can be asked for (issue #543): the core asks only a
/// device that lists it, so the switch is left out for one known not to
pub fn low_latency_offered(codec: &str, caps: Option<&CodecCaps>) -> bool {
    codec == "lhdcv5" && caps.is_none_or(|c| c.lhdcv5_low_latency)
}

/// LHDC V5 ABR runs at 400 kbps or below whatever the quality (as on
/// Android): the quality choice is greyed out then
pub fn quality_ignored(codec: &str, abr: bool) -> bool {
    codec == "lhdcv5" && abr
}

/// LDAC's rates at 44.1 / 88.2 kHz, LHDC V5's Standard at 44.1 kHz (88.2
/// kHz runs as 96 kHz there): v1's profile list
fn rate_44k(codec: &str, quality: &str) -> Option<&'static str> {
    match (codec, quality) {
        ("ldac", "hq") => Some("909"),
        ("ldac", "sq") => Some("606"),
        ("ldac", "mq") => Some("303"),
        ("lhdcv5", "sq") => Some("480"),
        _ => None,
    }
}

/// aptX Adaptive at 96 kHz (an R2 device's 96 kHz levels; Low Latency runs
/// at 48 kHz)
fn rate_96k(codec: &str, quality: &str) -> Option<&'static str> {
    match (codec, quality) {
        ("aptxad", "hq") => Some("646"),
        _ => None,
    }
}

/// A quality's bit rate at the profile's sample rate (0 = auto), at most
/// the device's largest (`max_bitrate` in bps, 0 = not known: AAC's, LHDC
/// V5's; the core caps it the same way, issue #86)
pub fn quality_kbps(codec: &str, quality: &str, sample_rate: u32, max_bitrate: u32) -> Option<String> {
    let spec = codec_spec(codec);
    let q = spec.qualities.iter().find(|x| x.id == quality).or(spec.qualities.first())?;
    let is441 = if codec == "lhdcv5" { sample_rate == 44100 } else { sample_rate == 44100 || sample_rate == 88200 };
    let table = if is441 {
        rate_44k(spec.id, q.id)
    } else if sample_rate == 96000 {
        rate_96k(spec.id, q.id)
    } else {
        None
    };
    let kbps = table.or(q.kbps)?;
    let cap = max_bitrate / 1000;
    match kbps.parse::<u64>() {
        Ok(n) if cap > 0 && kbps.bytes().all(|b| b.is_ascii_digit()) && n > u64::from(cap) => Some(cap.to_string()),
        _ => Some(kbps.to_owned()),
    }
}

/// "384 kbps": a codec's one bit rate at a sample rate of 44.1 or 48 kHz, for
/// one without qualities (issue #86)
pub fn fixed_kbps_text(codec: &str, sample_rate: u32) -> String {
    match codec_spec(codec).fixed_kbps {
        Some(k) => format!("{} kbps", k[if sample_rate == 44100 { 0 } else { 1 }]),
        None => String::new(),
    }
}

/// A quality the dialog offers, with its bit rate there
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferedQuality {
    pub id: &'static str,
    /// Translation key of the name
    pub label: &'static str,
    pub kbps: Option<String>,
}

/// The qualities the dialog offers for a codec on a device whose largest
/// bit rate is `max_bitrate` (bps, 0 = not known), each with its bit rate
/// there: one the device caps to the same bit rate as a higher one is left
/// out (LHDC V5 at most 400 kbps: High only), unless it is `current` ("" for none)
pub fn offered_qualities(codec: &str, sample_rate: u32, max_bitrate: u32, current: &str) -> Vec<OfferedQuality> {
    let mut out: Vec<OfferedQuality> = Vec::new();
    for q in codec_spec(codec).qualities {
        // Low Latency is aptX Adaptive LL's card (issue #86)
        if codec == "aptxad" && q.id == "ll" {
            continue;
        }
        let kbps = quality_kbps(codec, q.id, sample_rate, max_bitrate);
        if q.id != current && kbps.is_some() && out.iter().any(|o| o.kbps == kbps) {
            continue;
        }
        out.push(OfferedQuality { id: q.id, label: q.label, kbps });
    }
    out
}

/// The quality of a profile as the profile list shows it (v1's
/// rebuild_profile_list): "High (990 kbps)", "ABR (≤400 kbps)" under LHDC V5
/// ABR; None when the codec has no quality choice (aptX family, FastStream,
/// LE Audio). `label` translates the quality's key (normally `i18n::t`).
pub fn quality_tag(p: &Profile, label: impl Fn(&str) -> String) -> Option<String> {
    if p.le_audio {
        return None;
    }
    let spec = codec_spec(&p.codec);
    let q = spec.qualities.iter().find(|x| x.id == p.quality).or(spec.qualities.first())?;
    if quality_ignored(&p.codec, p.abr) {
        return Some("ABR (≤400 kbps)".into());
    }
    Some(match quality_kbps(&p.codec, q.id, p.sample_rate, 0) {
        Some(kbps) => format!("{} ({kbps} kbps)", label(q.label)),
        None => label(q.label),
    })
}

/// Why a stream's bit rate is below the profile's, appended to the codec
/// line: ABR (LHDC V5 stays at 400 kbps or below), or the device's maximum
/// (StreamInfo.abr, StreamInfo.bitrate_capped)
pub fn bitrate_note_key(abr: bool, bitrate_capped: bool) -> Option<&'static str> {
    if abr {
        Some("quality.codec_abr")
    } else if bitrate_capped {
        Some("quality.codec_capped")
    } else {
        None
    }
}

/// 0 = auto. Auto may pick LDAC / LHDC V5, so it offers 16 / 24 too (the
/// core carries 24-bit only when the codec it picks has it)
pub fn bit_depths(codec: &str) -> Vec<u32> {
    let spec = codec_spec(codec);
    if spec.hi_res || spec.id == "auto" { vec![0, 16, 24] } else { vec![0] }
}

/// The bit depth a codec without a choice carries (aptX HD and aptX
/// Adaptive 24, the others 16)
pub fn fixed_bit_depth(codec: &str) -> u32 {
    codec_spec(codec).bits.unwrap_or(16)
}

/// Values the codec cannot use go back to the defaults (v1 did the same when the codec changed)
pub fn fit_to_codec<T: CodecFields>(p: &T) -> T {
    let mut c = p.codec_choice();
    let spec = codec_spec(&c.codec);
    if !(spec.qualities.is_empty() || spec.qualities.iter().any(|m| m.id == c.quality)) {
        c.quality = "hq".into();
    }
    // The frame setup is SBC's and SBC XQ's; the channel mode SBC's only (XQ
    // is dual channel, and plain SBC has no dual channel: issue #86)
    let sbc = c.codec == "sbc" || c.codec == "sbcxq";
    let is_sbc = c.codec == "sbc";
    let is_xq = c.codec == "sbcxq";
    c.abr = (spec.abr || spec.vbr) && c.abr;
    if !sample_rates(&c.codec).contains(&c.sample_rate) {
        c.sample_rate = 0;
    }
    if !bit_depths(&c.codec).contains(&c.bit_depth) {
        c.bit_depth = 0;
    }
    if !is_sbc {
        c.sbc_bitpool = 0;
        c.sbc_min_bitpool = 0;
    }
    if !is_xq {
        c.sbcxq_bitpool = 0;
        c.sbcxq_min_bitpool = 0;
    }
    if !sbc {
        c.sbc_blocks = 0;
        c.sbc_subbands = 0;
        c.sbc_allocation.clear();
    }
    if !(is_sbc && c.sbc_channel_mode != "dual_channel") {
        c.sbc_channel_mode.clear();
    }
    c.lhdcv5_low_latency = c.codec == "lhdcv5" && c.lhdcv5_low_latency;
    let mut out = p.clone();
    out.set_codec_choice(c);
    out
}

// Max media packet size (crates/bt-host/src/media.rs, MEDIA_PAYLOAD_LIMIT_MIN / MAX / DEFAULT)
pub const PACKET_MIN: u16 = 679;
pub const PACKET_MAX: u16 = 1679;
pub const PACKET_DEFAULT: u16 = 1023;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::english_text;
    use crate::sbc::mock::*;
    use ipc::Msg;

    const NONE: &[&str] = &[];

    fn caps(codec: &str, sample_rates: &[u32], bit_depths: &[u32], max_bitrate: u32) -> CodecCaps {
        CodecCaps {
            codec: codec.into(),
            sample_rates: sample_rates.to_vec(),
            bit_depths: bit_depths.to_vec(),
            max_bitrate,
            lhdcv5_low_latency: false,
        }
    }

    /// A profile as the tests change it: `{ ...sampleProfile(), codec: "lhdcv5", quality: "mhq", abr: true, sample_rate: 96000, bit_depth: 24 }`
    fn p() -> Profile {
        Profile {
            codec: "lhdcv5".into(),
            quality: "mhq".into(),
            abr: true,
            sample_rate: 96000,
            bit_depth: 24,
            ..sample_profile()
        }
    }

    fn with(codec: &str) -> Profile {
        Profile { codec: codec.into(), ..p() }
    }

    // ---- profiles ----

    // Issue #543: LHDC V5's low latency is its own and is asked for only
    // from a device that lists it (or one whose caps are not known yet)
    #[test]
    fn keeps_lhdc_v5_low_latency_to_lhdc_v5_and_the_devices_that_list_it() {
        let lp = Profile { codec: "lhdcv5".into(), lhdcv5_low_latency: true, ..sample_profile() };
        assert!(fit_to_codec(&lp).lhdcv5_low_latency);
        assert!(!fit_to_codec(&Profile { codec: "ldac".into(), ..lp }).lhdcv5_low_latency);
        let c = caps("lhdcv5", &[48000], &[16, 24], 0);
        assert!(low_latency_offered("lhdcv5", None));
        assert!(low_latency_offered("lhdcv5", Some(&CodecCaps { lhdcv5_low_latency: true, ..c.clone() })));
        assert!(!low_latency_offered("lhdcv5", Some(&CodecCaps { lhdcv5_low_latency: false, ..c })));
        assert!(!low_latency_offered("ldac", None));
    }

    #[test]
    fn drops_what_the_new_codec_cannot_do() {
        let f = fit_to_codec(&with("ldac"));
        assert_eq!((f.quality.as_str(), f.abr, f.sample_rate, f.bit_depth), ("hq", true, 96000, 24));
        // AAC takes the flag as VBR (the editor clears it when the codec changes to or from AAC)
        let f = fit_to_codec(&with("aac"));
        assert_eq!((f.abr, f.sample_rate, f.bit_depth), (true, 0, 0));
        // SBC takes it as its adaptive bitpool (the editor clears it when the
        // codec changes to or from SBC / SBC XQ: abrCarries)
        let f = fit_to_codec(&Profile { sbc_bitpool: 40, ..with("sbc") });
        assert_eq!((f.abr, f.sbc_bitpool), (true, 40));
        assert!(!fit_to_codec(&with("faststream")).abr);
        assert!(abr_carries("ldac", "lhdcv5"));
        assert!(abr_carries("auto", "ldac"));
        // SBC's adaptive bitpool is not SBC XQ's (issue #86)
        assert!(!abr_carries("sbc", "sbcxq"));
        assert!(!abr_carries("sbcxq", "sbc"));
        assert!(abr_carries("sbc", "sbc"));
        let sbc_abr = Profile {
            codec: "sbc".into(),
            abr: true,
            sbc_bitpool: 40,
            sbcxq_bitpool: 0,
            sbc_blocks: 8,
            sbc_subbands: 4,
            sbc_allocation: "snr".into(),
            sbc_channel_mode: "stereo".into(),
            ..p()
        };
        let f = carry_to_codec(&sbc_abr, "sbcxq");
        assert_eq!(
            (
                f.codec.as_str(),
                f.abr,
                f.sbc_bitpool,
                f.sbc_blocks,
                f.sbc_subbands,
                f.sbc_allocation.as_str(),
                f.sbc_channel_mode.as_str()
            ),
            ("sbcxq", false, 0, 0, 0, "", "")
        );
        let f = carry_to_codec(&sbc_abr, "sbc");
        assert_eq!(
            (
                f.abr,
                f.sbc_bitpool,
                f.sbc_blocks,
                f.sbc_subbands,
                f.sbc_allocation.as_str(),
                f.sbc_channel_mode.as_str()
            ),
            (true, 40, 8, 4, "snr", "stereo")
        );
        assert!(carry_to_codec(&Profile { codec: "ldac".into(), abr: true, ..p() }, "lhdcv5").abr);
        assert!(!abr_carries("ldac", "sbc"));
        assert!(!abr_carries("sbcxq", "auto"));
        assert!(!abr_carries("aac", "sbc"));
        assert!(!abr_carries("aac", "ldac"));
        assert_eq!(fit_to_codec(&Profile { sbc_bitpool: 40, ..with("ldac") }).sbc_bitpool, 0);
        // SBC and SBC XQ keep their own bitpool
        let f = fit_to_codec(&Profile { sbc_bitpool: 53, sbcxq_bitpool: 40, ..with("sbcxq") });
        assert_eq!((f.sbc_bitpool, f.sbcxq_bitpool), (0, 40));
        // ... and their own minimum (issue #86)
        let f = fit_to_codec(&Profile { sbc_min_bitpool: 20, sbcxq_min_bitpool: 30, ..with("sbcxq") });
        assert_eq!((f.sbc_min_bitpool, f.sbcxq_min_bitpool), (0, 30));
        assert_eq!(fit_to_codec(&Profile { sbc_min_bitpool: 20, ..with("aac") }).sbc_min_bitpool, 0);
        // The frame setup is both's; the channel mode SBC's only
        let frame = |codec: &str| Profile {
            sbc_blocks: 8,
            sbc_subbands: 4,
            sbc_allocation: "snr".into(),
            sbc_channel_mode: "stereo".into(),
            ..with(codec)
        };
        let setup = |f: &Profile| (f.sbc_blocks, f.sbc_subbands, f.sbc_allocation.clone(), f.sbc_channel_mode.clone());
        assert_eq!(setup(&fit_to_codec(&frame("sbc"))), (8, 4, "snr".into(), "stereo".into()));
        assert_eq!(setup(&fit_to_codec(&frame("sbcxq"))), (8, 4, "snr".into(), String::new()));
        assert_eq!(setup(&fit_to_codec(&frame("aac"))), (0, 0, String::new(), String::new()));
        let f = fit_to_codec(&with("auto"));
        assert_eq!((f.abr, f.sample_rate, f.bit_depth), (true, 0, 24));
        let f = fit_to_codec(&with("aptx"));
        assert_eq!((f.quality.as_str(), f.abr), ("mhq", false));
        let f = fit_to_codec(&with("opus"));
        assert_eq!((f.quality.as_str(), f.abr, f.sample_rate, f.bit_depth), ("hq", false, 0, 0));
        assert_eq!(fit_to_codec(&Profile { sample_rate: 48000, ..with("opus") }).sample_rate, 48000);
        assert_eq!(fit_to_codec(&Profile { sample_rate: 44100, ..with("opus") }).sample_rate, 0);
        assert_eq!(sample_rates("opus"), [0, 48000]);
        let f = fit_to_codec(&Profile { sample_rate: 44100, ..with("opus05") });
        assert_eq!((f.quality.as_str(), f.abr, f.sample_rate, f.bit_depth), ("hq", false, 0, 0));
        assert_eq!(sample_rates("opus05"), [0, 48000]);
        // aptX Adaptive: 48 / 96 kHz, ABR, and its low latency mode as a quality
        let f = fit_to_codec(&Profile { sample_rate: 44100, ..with("aptxad") });
        assert_eq!((f.quality.as_str(), f.abr, f.sample_rate, f.bit_depth), ("hq", true, 0, 0));
        assert_eq!(fit_to_codec(&Profile { quality: "ll".into(), ..with("aptxad") }).quality, "ll");
        assert_eq!(fit_to_codec(&Profile { quality: "ll".into(), ..with("ldac") }).quality, "hq");
        assert_eq!(sample_rates("aptxad"), [0, 48000, 96000]);
        assert_eq!(fit_to_codec(&Profile { sample_rate: 96000, ..with("aptxad") }).sample_rate, 96000);
        // Always 24-bit, no choice (aptX HD too); the others without a choice 16
        assert_eq!(bit_depths("aptxad"), [0]);
        assert_eq!(
            [fixed_bit_depth("aptxad"), fixed_bit_depth("aptxhd"), fixed_bit_depth("aptx"), fixed_bit_depth("opus")],
            [24, 24, 16, 16]
        );
    }

    #[test]
    fn offers_192_khz_for_lhdc_v5_only_and_no_quality_under_its_abr() {
        assert_eq!(sample_rates("lhdcv5"), [0, 44100, 48000, 96000, 192000]);
        assert!(!sample_rates("ldac").contains(&192000));
        let lp = Profile { codec: "lhdcv5".into(), sample_rate: 192000, ..sample_profile() };
        assert_eq!(fit_to_codec(&lp).sample_rate, 192000);
        assert_eq!(fit_to_codec(&Profile { codec: "ldac".into(), ..lp }).sample_rate, 0);
        // LHDC V5 ABR stays at 400 kbps or below whatever the quality (as on Android)
        assert!(quality_ignored("lhdcv5", true));
        assert!(!quality_ignored("lhdcv5", false));
        assert!(!quality_ignored("ldac", true));
    }

    #[test]
    fn offers_high_medium_low_for_ldac_and_lhdc_v5_high_ll_for_aptx_adaptive_high_elsewhere() {
        let ids = |codec: &str| {
            CODECS.iter().find(|c| c.id == codec).unwrap().qualities.iter().map(|q| q.id).collect::<Vec<_>>()
        };
        assert_eq!(ids("ldac"), ["hq", "sq", "mq"]);
        assert_eq!(ids("lhdcv5"), ["hq", "mhq", "sq", "mq"]);
        assert_eq!(ids("aptxad"), ["hq", "ll"]);
        // Low Latency has its own card, aptX Adaptive LL: not in aptX Adaptive's (issue #86)
        assert_eq!(offered_qualities("aptxad", 0, 0, "").iter().map(|q| q.id).collect::<Vec<_>>(), ["hq"]);
        let ll = codec_spec("aptxadll");
        assert_eq!((ll.label, ll.qualities.len(), ll.abr, ll.bits), ("codec.aptx_adaptive_ll", 0, false, Some(24)));
        assert_eq!(fixed_kbps_text("aptxadll", 48000), "265 kbps");
        assert_eq!(sample_rates("aptxadll"), [0, 48000]);
        assert!(!CODECS.iter().any(|c| c.id == "aptxadll"));
        for c in ["auto", "aac", "opus", "opus05", "sbc", "sbcxq"] {
            assert_eq!(ids(c), ["hq"]);
        }
        for c in ["aptx", "aptxhd", "aptxll", "faststream"] {
            assert!(ids(c).is_empty());
        }
    }

    #[test]
    fn offers_only_what_the_device_lists() {
        let list = [caps("lhdcv5", &[44100, 48000], &[16], 0), caps("aac", &[44100, 48000], &[], 0x020000)];
        let lhdc = caps_of(&list, "LHDCV5");
        assert_eq!(offered_sample_rates("lhdcv5", lhdc), [0, 44100, 48000]);
        assert_eq!(offered_bit_depths("lhdcv5", lhdc), [0, 16]);
        // Not known: everything; 88.2 kHz runs as 96 kHz on LHDC V5
        assert_eq!(offered_sample_rates("lhdcv5", None), sample_rates("lhdcv5"));
        // No 88.2 kHz standing in for 96 kHz (issue #86: Moto buds+)
        let c = CodecCaps { sample_rates: vec![48000, 96000], ..lhdc.unwrap().clone() };
        assert_eq!(offered_sample_rates("lhdcv5", Some(&c)), [0, 48000, 96000]);
        // AAC's High at the device's largest bit rate
        assert_eq!(quality_kbps("aac", "hq", 0, caps_of(&list, "aac").unwrap().max_bitrate).as_deref(), Some("131"));
        assert_eq!(quality_kbps("aac", "hq", 0, 320000).as_deref(), Some("256"));
        // LHDC V5 at most 500 kbps: High is 500, Medium the same (left out)
        assert_eq!(fixed_kbps_text("aptx", 44100), "352 kbps");
        assert_eq!(fixed_kbps_text("aptxll", 48000), "384 kbps");
        assert_eq!(fixed_kbps_text("faststream", 48000), "216 kbps");
        let pairs = |v: Vec<OfferedQuality>| v.into_iter().map(|q| (q.id, q.kbps.unwrap())).collect::<Vec<_>>();
        assert_eq!(
            pairs(offered_qualities("lhdcv5", 48000, 0, "")),
            [("hq", "1000".into()), ("mhq", "900".into()), ("sq", "500".into()), ("mq", "400".into())]
        );
        assert_eq!(pairs(offered_qualities("lhdcv5", 48000, 500000, "")), [("hq", "500".into()), ("mq", "400".into())]);
        assert_eq!(
            offered_qualities("lhdcv5", 48000, 500000, "sq").iter().map(|q| q.id).collect::<Vec<_>>(),
            ["hq", "sq", "mq"]
        );
        // Developer mode's override: every codec's maximum goes (issue #237:
        // LHDC V5 400 kbps listed), the SBC bitpool's too but not its minimum
        let listed = [
            caps("lhdcv5", &[48000], &[24], 400000),
            caps("aac", &[48000], &[], 262144),
            caps("ldac", &[48000], &[], 0),
        ];
        let dev = |on: bool, developer_mode: bool| DevMode {
            developer_mode,
            dev_overrides: DevOverrides { ignore_sink_max: on, ..Default::default() },
        };
        assert_eq!(caps_for_settings(&listed, dev(false, true)), listed);
        assert_eq!(caps_for_settings(&listed, dev(true, false)), listed);
        assert_eq!(
            caps_for_settings(&listed, dev(true, true)).iter().map(|c| c.max_bitrate).collect::<Vec<_>>(),
            [0, 0, 0]
        );
        let sbc = SbcCaps { min_bitpool: 10, max_bitpool: 39, ..xm5_sbc() };
        assert_eq!(sbc_caps_for_settings(Some(&sbc), dev(false, true)), Some(sbc.clone()));
        assert_eq!(sbc_caps_for_settings(None, dev(true, true)), None);
        assert_eq!(sbc_caps_for_settings(Some(&sbc), dev(true, true)), Some(SbcCaps { max_bitpool: 255, ..sbc }));
        // aptX Adaptive's 96 kHz only when the device lists it (issue #86: a
        // Bose QC Ultra, 0x51, does not; an EarFun Air Pro 4+ does), also
        // while not known
        let aptxad = |rates: &[u32]| caps("aptxad", rates, &[], 0);
        assert_eq!(offered_sample_rates("aptxad", Some(&aptxad(&[48000]))), [0, 48000]);
        assert_eq!(offered_sample_rates("aptxad", Some(&aptxad(&[48000, 96000]))), [0, 48000, 96000]);
        assert_eq!(offered_sample_rates("aptxad", None), [0, 48000]);
        assert_eq!(offered_sample_rates("aptxad", Some(&aptxad(&[]))), [0, 48000]);
        // One value to take: fixed, no Auto to choose beside it
        assert_eq!(only_choice(&offered_sample_rates("aptxad", Some(&aptxad(&[48000]))), &0, &0), Some(48000));
        assert_eq!(only_choice(&offered_sample_rates("aptxad", Some(&aptxad(&[48000]))), &48000, &0), Some(48000));
        assert_eq!(only_choice(&offered_sample_rates("aptxad", Some(&aptxad(&[48000, 96000]))), &0, &0), None);
        // A kept value the device did not list keeps the list, to move off it
        assert_eq!(only_choice(&[0, 48000], &96000, &0), None);
        assert_eq!(only_choice(&["", "loudness"], &"", &""), Some("loudness"));
    }

    #[test]
    fn says_why_the_bit_rate_is_below_the_profiles() {
        assert_eq!(bitrate_note_key(true, false), Some("quality.codec_abr"));
        assert_eq!(bitrate_note_key(false, true), Some("quality.codec_capped"));
        assert_eq!(bitrate_note_key(true, true), Some("quality.codec_abr"));
        assert_eq!(bitrate_note_key(false, false), None);
    }

    #[test]
    fn names_codecs_as_the_core_reports_them() {
        assert_eq!(codec_label_key("aptX HD"), "codec.aptx_hd");
        assert_eq!(codec_label_key("LHDC V5"), "codec.lhdc_v5");
        assert_eq!(codec_label_key("Opus"), "codec.opus");
        assert_eq!(codec_label_key("Opus 05"), "codec.opus05");
        assert_eq!(codec_label_key("aptX Adaptive"), "codec.aptx_adaptive");
        assert_eq!(codec_label_key("SBC XQ"), "codec.sbc_xq");
        assert_eq!(codec_label_key("FastStream"), "codec.faststream");
        assert_eq!(codec_label_key("Ldac"), "codec.ldac");
        // LE Audio streams LC3
        assert_eq!(codec_label_key("LC3"), "ui.codec.lc3");
    }

    // ---- profile list ----

    #[test]
    fn shows_the_quality_and_its_bit_rate_as_v1_did() {
        // English whatever language another test switched to
        let en = |key: &str| english_text(&Msg { key: key.into(), args: vec![] });
        let tag = |codec: &str, quality: &str, sample_rate: u32, abr: bool, le_audio: bool| {
            let pr = Profile {
                codec: codec.into(),
                quality: quality.into(),
                sample_rate,
                abr,
                le_audio,
                ..sample_profile()
            };
            quality_tag(&pr, en)
        };
        let t = |codec: &str, quality: &str, rate: u32| tag(codec, quality, rate, false, false);
        assert_eq!(t("ldac", "hq", 0).as_deref(), Some("High (990 kbps)"));
        assert_eq!(t("ldac", "sq", 44100).as_deref(), Some("Medium (606 kbps)"));
        assert_eq!(t("ldac", "mq", 88200).as_deref(), Some("Low (303 kbps)"));
        assert_eq!(t("lhdcv5", "sq", 44100).as_deref(), Some("Medium (480 kbps)"));
        assert_eq!(t("lhdcv5", "sq", 96000).as_deref(), Some("Medium (500 kbps)"));
        // LHDC V5's Medium-High is back (issue #203); a quality a codec does
        // not offer is High
        assert_eq!(t("lhdcv5", "mhq", 0).as_deref(), Some("Medium-High (900 kbps)"));
        assert_eq!(t("ldac", "mhq", 0).as_deref(), Some("High (990 kbps)"));
        assert_eq!(t("aptxad", "ll", 0).as_deref(), Some("Low Latency (265 kbps)"));
        assert_eq!(t("aptxad", "hq", 96000).as_deref(), Some("High (646 kbps)"));
        assert_eq!(t("aptxad", "mq", 96000).as_deref(), Some("High (646 kbps)"));
        assert_eq!(t("aptxad", "ll", 96000).as_deref(), Some("Low Latency (265 kbps)"));
        assert_eq!(tag("lhdcv5", "hq", 0, true, false).as_deref(), Some("ABR (≤400 kbps)"));
        assert_eq!(t("sbc", "sq", 0).as_deref(), Some("High (~345 kbps)"));
        assert_eq!(t("aac", "mq", 0).as_deref(), Some("High (256 kbps)"));
        assert_eq!(t("auto", "hq", 0).as_deref(), Some("High"));
        // No quality choice
        assert_eq!(t("aptxhd", "hq", 0), None);
        assert_eq!(t("faststream", "hq", 0), None);
        assert_eq!(tag("auto", "hq", 0, false, true), None);
    }

    // ---- codecs a device offers ----

    #[test]
    fn is_unknown_until_the_device_connected_once() {
        assert_eq!(codec_support("ldac", NONE, NONE), CodecSupport::Unknown);
        assert_eq!(codec_support("auto", NONE, NONE), CodecSupport::Unknown);
    }

    #[test]
    fn tells_offered_from_not_offered() {
        let known = ["sbc", "aac", "ldac"];
        assert_eq!(codec_support("ldac", &known, NONE), CodecSupport::Offered);
        assert_eq!(codec_support("aptxhd", &known, NONE), CodecSupport::NotOffered);
        assert_eq!(codec_support("auto", &known, NONE), CodecSupport::Offered);
        assert_eq!(codec_support("LDAC", &["ldac"], NONE), CodecSupport::Offered);
    }

    #[test]
    fn does_not_call_one_it_may_offer_once_chosen_not_offered() {
        assert_eq!(codec_support("ldac", &["sbc", "aac"], &["ldac"]), CodecSupport::Unknown);
        assert_eq!(codec_support("aptx", &["sbc", "aac"], &["ldac"]), CodecSupport::NotOffered);
        assert_eq!(codec_support("ldac", &["sbc", "ldac"], &["ldac"]), CodecSupport::Offered);
    }

    // ---- Auto's order and the fallback (issue #86) ----

    #[test]
    fn goes_by_sound_quality_as_the_core() {
        assert_eq!(AUTO_ORDER, ["lhdcv5", "ldac", "aptxad", "opus", "aptxhd", "aptx", "aptxll", "aac", "sbcxq", "sbc"]);
        assert_eq!(auto_pick(&["sbc", "aac", "aptx", "aptxad"], &AUTO_ORDER), Some("aptxad"));
        assert_eq!(auto_pick(&["sbc", "aac", "ldac", "lhdcv5"], &AUTO_ORDER), Some("lhdcv5"));
        assert_eq!(auto_pick(&["sbc", "faststream", "opus05"], &AUTO_ORDER), Some("sbc"));
        assert_eq!(auto_pick(NONE, &AUTO_ORDER), None);
    }

    fn eff(codec: &str, known: &[&str]) -> Option<String> {
        effective_codec(codec, known, &AUTO_ORDER)
    }

    #[test]
    fn falls_back_to_the_best_codec_the_device_offers() {
        let earfun = ["sbc", "aac", "aptx", "aptxad"];
        // The tester's EarFun: Opus chosen gives aptX Adaptive, not aptX below Opus
        assert_eq!(eff("opus", &earfun).as_deref(), Some("aptxad"));
        assert_eq!(eff("lhdcv5", &earfun).as_deref(), Some("aptxad"));
        assert_eq!(eff("aptxhd", &earfun).as_deref(), Some("aptxad"));
        assert_eq!(eff("aptxhd", &["sbc", "aac", "ldac"]).as_deref(), Some("ldac"));
        assert_eq!(eff("sbcxq", &["sbc", "aac"]).as_deref(), Some("aac"));
        // Offered: itself; Auto: its pick
        assert_eq!(eff("aac", &earfun).as_deref(), Some("aac"));
        assert_eq!(eff("auto", &earfun).as_deref(), Some("aptxad"));
        // Opus 05 and FastStream are never a fallback
        assert_eq!(eff("opus05", &["sbc", "opus"]).as_deref(), Some("opus"));
        assert_eq!(eff("faststream", &["sbc", "aac"]).as_deref(), Some("aac"));
        assert_eq!(eff("ldac", &["sbc", "faststream", "opus05"]).as_deref(), Some("sbc"));
        // Not known yet
        assert_eq!(eff("ldac", &[]), None);
    }

    #[test]
    fn offers_pipewires_opus_only_in_developer_mode() {
        assert_eq!(CODECS.iter().filter(|c| developer_only(c.id)).map(|c| c.id).collect::<Vec<_>>(), ["opus05"]);
        assert_eq!(offered_codecs(&CARD_ORDER, true), CARD_ORDER);
        assert_eq!(
            offered_codecs(&CARD_ORDER, false),
            CARD_ORDER.into_iter().filter(|c| *c != "opus05").collect::<Vec<_>>()
        );
        // Unknown keys are not dropped
        assert_eq!(offered_codecs(&["sbc", "lc3"], false), ["sbc", "lc3"]);
    }

    // ---- A device's codec order (issue #86 round 8) ----

    #[test]
    fn keeps_every_codec_of_auto_order_once_the_ones_given_first() {
        assert_eq!(auto_order(NONE), AUTO_ORDER);
        let o = auto_order(&["AAC", "nope", "ldac", "aac", "faststream"]);
        assert_eq!(o[..3], ["aac", "ldac", "lhdcv5"]);
        assert_eq!(o.len(), AUTO_ORDER.len());
    }

    #[test]
    fn puts_aptx_adaptive_lls_card_beside_aptx_adaptive_supported_as_it_is() {
        assert_eq!(CARD_ORDER[2..4], ["aptxad", "aptxadll"]);
        // An order saved before the card
        assert_eq!(card_order(&["sbc", "aptxad", "ldac"])[..4], ["sbc", "aptxad", "aptxadll", "ldac"]);
        assert_eq!(card_order(&["aptxadll", "sbc"])[..2], ["aptxadll", "sbc"]);
        assert_eq!(codec_key("aptxadll"), "aptxad");
        let earfun = ["sbc", "aac", "aptxad"];
        assert_eq!(codec_support("aptxadll", &earfun, NONE), CodecSupport::Offered);
        assert_eq!(codec_support("aptxadll", &["sbc"], NONE), CodecSupport::NotOffered);
        assert_eq!(eff("aptxadll", &earfun).as_deref(), Some("aptxadll"));
        // Auto never takes Low Latency
        assert_eq!(effective_codec("auto", &earfun, &card_order(&["aptxadll", "aac"])).as_deref(), Some("aac"));
        assert!(caps_of(&[caps("aptxad", &[48000], &[], 0)], "aptxadll").is_some());
        assert_eq!(auto_bit_depth("aptxadll", None), 24);
    }

    #[test]
    fn puts_every_card_once_the_ones_given_first() {
        assert_eq!(card_order(NONE), CARD_ORDER);
        assert_eq!(CARD_ORDER.into_iter().filter(|c| AUTO_ORDER.contains(c)).collect::<Vec<_>>(), AUTO_ORDER);
        let o = card_order(&["SBCXQ", "nope", "sbc", "sbcxq", "faststream"]);
        assert_eq!(o[..4], ["sbcxq", "sbc", "faststream", "lhdcv5"]);
        let mut sorted = o.clone();
        sorted.sort();
        let mut all = CARD_ORDER.to_vec();
        all.sort();
        assert_eq!(sorted, all);
        assert!(is_default_order(NONE));
        assert!(is_default_order(&CARD_ORDER));
        assert!(!is_default_order(&o));
        assert_eq!(moved(&["a", "b", "c"], 2, 0), ["c", "a", "b"]);
        assert_eq!(moved(&["a", "b", "c"], 0, 1), ["b", "a", "c"]);
    }

    #[test]
    fn is_what_auto_and_a_fallback_follow() {
        let known = ["sbc", "aac", "ldac"];
        let aac_first = card_order(&["aac"]);
        assert_eq!(eff("auto", &known).as_deref(), Some("ldac"));
        assert_eq!(effective_codec("auto", &known, &aac_first).as_deref(), Some("aac"));
        // aptX HD not offered: the first card the device offers
        assert_eq!(effective_codec("aptxhd", &known, &card_order(&["aptxhd", "sbc", "ldac"])).as_deref(), Some("sbc"));
        // The tester's order, SBC XQ first: SBC XQ for an Opus it does not offer
        let earfun = ["sbc", "sbcxq", "aac", "aptx", "aptxad"];
        let order = card_order(&["sbcxq", "sbc", "aptxad", "aac", "aptx"]);
        assert_eq!(effective_codec("opus", &earfun, &order).as_deref(), Some("sbcxq"));
        // FastStream first: never Auto's pick nor a fallback
        assert_eq!(
            effective_codec("ldac", &["sbc", "faststream"], &card_order(&["faststream"])).as_deref(),
            Some("sbc")
        );
    }

    // ---- Auto's value shown beside Auto (issue #86) ----

    #[test]
    fn is_48_khz_unless_the_device_lacks_it() {
        assert_eq!(auto_sample_rate("aac", None), 48000);
        assert_eq!(auto_sample_rate("aac", Some(&caps("aac", &[44100], &[], 0))), 44100);
        assert_eq!(auto_sample_rate("ldac", Some(&caps("ldac", &[44100, 48000, 96000], &[], 0))), 48000);
        assert_eq!(auto_sample_rate("sbc", Some(&caps("sbc", &[44100], &[], 0))), 48000);
    }

    #[test]
    fn is_24_bit_where_the_codec_carries_it() {
        assert_eq!(auto_bit_depth("ldac", None), 24);
        assert_eq!(auto_bit_depth("aac", None), 16);
        assert_eq!(auto_bit_depth("lhdcv5", Some(&caps("lhdcv5", &[48000], &[16], 0))), 16);
        assert_eq!(auto_bit_depth("lhdcv5", Some(&caps("lhdcv5", &[48000], &[16, 24], 0))), 24);
    }

    #[test]
    fn matching_the_pc_is_the_listed_rate_nearest_the_pcs_its_family_first() {
        assert_eq!(nearest_rate(&[44100, 48000, 88200, 96000], 192000), 96000);
        assert_eq!(nearest_rate(&[44100, 48000, 88200, 96000], 176400), 88200);
        assert_eq!(nearest_rate(&[44100, 48000, 96000], 88200), 44100);
        assert_eq!(nearest_rate(&[16000, 32000, 48000], 44100), 48000);
        assert_eq!(nearest_rate(&[44100], 96000), 44100);
        assert_eq!(nearest_rate(&[48000, 96000], 32000), 48000);
        assert_eq!(nearest_rate(&[], 96000), 96000);
        assert_eq!(matched_sample_rate("ldac", Some(&caps("ldac", &[44100, 48000, 96000], &[], 0)), 192000), 96000);
        assert_eq!(matched_sample_rate("aac", Some(&caps("aac", &[44100, 48000], &[], 0)), 96000), 48000);
        assert_eq!(matched_sample_rate("aptx", Some(&caps("aptx", &[44100, 48000], &[], 0)), 88200), 44100);
    }

    #[test]
    fn matching_the_pc_is_16_bit_from_a_16_bit_output() {
        assert_eq!(matched_bit_depth("ldac", None, 16), 16);
        assert_eq!(matched_bit_depth("ldac", None, 32), 24);
        assert_eq!(matched_bit_depth("aptxad", None, 16), 24);
        assert_eq!(matched_bit_depth("lhdcv5", Some(&caps("lhdcv5", &[48000], &[16], 0)), 24), 16);
    }
}

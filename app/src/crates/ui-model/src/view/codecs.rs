//! The Codecs screen's rules (issue #86): which codec cards a device shows,
//! in which order, which one is chosen and how; the call codec and LE Audio
//! cards; and what a codec's "..." dialog offers (quality, sample rate, bit
//! depth, ABR, the SBC bitpool and frame setup) with the labels it shows.
//! The window (app/src/ui: screens/codecs.rs, ui/codec_cards.rs) draws these.

use ipc::{CodecValues, Profile, Snapshot, StreamInfo, StreamState};

use crate::codecs::{
    APTXAD_LL, BITPOOL_MAX, BITPOOL_MIN, Bars, CARD_ORDER, CODECS, CodecCaps, CodecChoice, CodecSupport,
    auto_bit_depth, auto_sample_rate, caps_of, card_of, card_order, carry_to_codec, codec_key, codec_label_key,
    codec_spec, codec_support, developer_only, effective_codec, fixed_bit_depth, fixed_kbps_text, is_default_order,
    low_latency_offered, matched_bit_depth, matched_sample_rate, offered_bit_depths, offered_codecs, offered_qualities,
    offered_sample_rates, only_choice, quality_ignored, unlisted_caps,
};
use crate::i18n::t;
use crate::profile::{AudioEntry, codecs_on_request, known_codecs, with_codec};
use crate::sbc::{
    SBC_ALLOCATIONS, SBC_BLOCKS, SBC_CHANNEL_MODES, SBC_SUBBANDS, SbcCaps, SbcField, sbc_kbps_text, sbc_offered,
    sbc_rate, sbc_setup, xq_max_bitpool,
};
use crate::t;

/// A "..." dialog's changes are saved this long after the last one: only
/// the end of a quick series (a slider moved)
pub const SAVE_DELAY_MS: u64 = 120;
/// After a codec chosen during a stream, the codec cards wait at least
/// this long, and until the stream is no longer starting again...
pub const SETTLE_MIN_MS: u64 = 1000;
/// ...but never longer than this (a start that hangs)
pub const SETTLE_MAX_MS: u64 = 15000;
/// The microphone's codec chosen: not closed within this long, there is
/// nothing to wait for (it kept what it had)
pub const REOPEN_WAIT_MS: u64 = 2000;

/// The stream is being started (again)
pub fn restarting(snap: Option<&Snapshot>) -> bool {
    snap.is_some_and(|s| matches!(s.stream.state, StreamState::Connecting | StreamState::Reconnecting))
}

/// The profile shown for a device: its own, or what it would start from
pub fn shown_profile(snap: &Snapshot, entry: &AudioEntry) -> Profile {
    entry.profile.clone().unwrap_or_else(|| crate::profile::device_base(&entry.device, &snap.profiles))
}

/// `values` differ from the profile's in the bitpools alone
pub fn only_bitpools(p: &Profile, values: &CodecChoice) -> bool {
    p.quality == values.quality
        && p.sample_rate == values.sample_rate
        && p.bit_depth == values.bit_depth
        && p.abr == values.abr
        && p.sbc_blocks == values.sbc_blocks
        && p.sbc_subbands == values.sbc_subbands
        && p.sbc_allocation == values.sbc_allocation
        && p.sbc_channel_mode == values.sbc_channel_mode
        && p.unlisted_options == values.unlisted_options
        && p.lhdcv5_low_latency == values.lhdcv5_low_latency
}

/// A "..." dialog's values as a profile keeps them for a codec (the choice
/// without the codec)
pub fn values_of(c: &CodecChoice) -> CodecValues {
    CodecValues {
        quality: c.quality.clone(),
        sample_rate: c.sample_rate,
        bit_depth: c.bit_depth,
        abr: c.abr,
        lhdcv5_low_latency: c.lhdcv5_low_latency,
        sbc_bitpool: c.sbc_bitpool,
        sbcxq_bitpool: c.sbcxq_bitpool,
        sbc_min_bitpool: c.sbc_min_bitpool,
        sbcxq_min_bitpool: c.sbcxq_min_bitpool,
        sbc_blocks: c.sbc_blocks,
        sbc_subbands: c.sbc_subbands,
        sbc_allocation: c.sbc_allocation.clone(),
        sbc_channel_mode: c.sbc_channel_mode.clone(),
        unlisted_options: c.unlisted_options,
    }
}

fn choice_of(codec: &str, v: &CodecValues) -> CodecChoice {
    CodecChoice {
        codec: codec.to_owned(),
        quality: v.quality.clone(),
        abr: v.abr,
        sample_rate: v.sample_rate,
        bit_depth: v.bit_depth,
        sbc_bitpool: v.sbc_bitpool,
        sbcxq_bitpool: v.sbcxq_bitpool,
        sbc_min_bitpool: v.sbc_min_bitpool,
        sbcxq_min_bitpool: v.sbcxq_min_bitpool,
        sbc_blocks: v.sbc_blocks,
        sbc_subbands: v.sbc_subbands,
        sbc_allocation: v.sbc_allocation.clone(),
        sbc_channel_mode: v.sbc_channel_mode.clone(),
        unlisted_options: v.unlisted_options,
        lhdcv5_low_latency: v.lhdcv5_low_latency,
    }
}

/// A codec order as a profile keeps it: none for the default one
pub fn order_keys<S: AsRef<str>>(order: &[S]) -> Vec<String> {
    if is_default_order(order) { Vec::new() } else { order.iter().map(|c| c.as_ref().to_owned()).collect() }
}

/// The device's profile with `codec` chosen (chooseCodec's edit: `values`
/// from its "..." dialog; `order`: the device's codec order, when one is not
/// saved yet; `forced`: chosen though the device does not offer it, asked
/// for anyway). An LE Audio entry's stays as it is.
pub fn chosen_profile(
    base: &Profile,
    le_audio: bool,
    codec: &str,
    values: Option<&CodecValues>,
    order: Option<&[&str]>,
    forced: bool,
) -> Profile {
    if le_audio {
        return base.clone();
    }
    let mut next = with_codec(base, codec, values);
    next.codec_forced = forced && codec != "auto";
    if let Some(order) = order {
        next.codec_order = order_keys(order);
    }
    next
}

/// The codec a device streams with now, when its profile streams (aptX
/// Adaptive LL's card while aptX Adaptive streams as its profile chose it in
/// Low Latency: not under Auto-select, which picks aptX Adaptive with its own
/// settings)
pub fn streaming_codec(snap: &Snapshot, entry: &AudioEntry) -> Option<&'static str> {
    let st = &snap.stream;
    let p = entry.profile.as_ref()?;
    if st.state != StreamState::Streaming || st.profile.as_deref() != Some(p.name.as_str()) {
        return None;
    }
    let key = codec_label_key(&st.info.as_ref()?.codec);
    let id = CODECS.iter().find(|c| c.id != "auto" && c.label == key)?.id;
    if id == "aptxad" && card_of(&p.codec, &p.quality) == APTXAD_LL.id && !snap.settings.auto_codec {
        Some(APTXAD_LL.id)
    } else {
        Some(id)
    }
}

/// The codec a device's cards (and Home's Codec card) show chosen: under
/// Auto (Auto-select on, `auto_all`, or no codec chosen for the device yet)
/// the one it streams with, else the one it would pick (None when the
/// device's codecs are not known); else the codec chosen. `order`: its
/// cards' order.
pub fn picked_codec<O: AsRef<str>>(snap: &Snapshot, entry: &AudioEntry, auto_all: bool, order: &[O]) -> Option<String> {
    let p = shown_profile(snap, entry);
    let codec = card_of(&p.codec, &p.quality).to_owned();
    // Outside developer mode its codecs are left out, as the core does: one
    // chosen is used as one the device does not offer, never asked for anyway
    let developer = snap.settings.developer_mode;
    let known = offered_codecs(known_codecs(&snap.devices, &entry.address), developer);
    let blocked = !developer && developer_only(&codec);
    // One it may offer once chosen stays chosen (issue #30)
    if !auto_all && !blocked && codecs_on_request(&snap.devices, &entry.address).iter().any(|k| k == codec_key(&codec))
    {
        return Some(codec);
    }
    // One chosen on its card though the device does not offer it: that one;
    // one it no longer offers: the one used instead (issue #86)
    if !auto_all && codec != "auto" {
        if blocked {
            return effective_codec(&codec, &known, order);
        }
        if p.codec_forced {
            return Some(codec);
        }
        return Some(effective_codec(&codec, &known, order).unwrap_or(codec));
    }
    streaming_codec(snap, entry).map(str::to_owned).or_else(|| effective_codec("auto", &known, order))
}

/// Whether a codec's card shows: the one picked always, the others unless
/// `hide` and the device did not offer them (a codec chosen that it does
/// not offer neither: the one used instead is picked). A device never
/// connected shows SBC alone, the codec every device has (issue #86: an
/// imported device's cards were a wall of unconfirmed ones).
pub fn codec_shown<S: AsRef<str>, R: AsRef<str>>(
    id: &str,
    picked: Option<&str>,
    known: &[S],
    hide: bool,
    on_request: &[R],
) -> bool {
    if !hide || Some(id) == picked {
        return true;
    }
    if known.is_empty() {
        return id == "sbc";
    }
    codec_support(id, known, on_request) != CodecSupport::NotOffered
}

/// Whether `order` puts the cards in `ids` (those shown) in their default
/// order; where the hidden ones are does not matter (issue #86)
pub fn shown_in_default_order<O: AsRef<str>, I: AsRef<str>>(order: &[O], ids: &[I]) -> bool {
    let shown = |c: &str| ids.iter().any(|i| i.as_ref() == c);
    let mine: Vec<&str> = order.iter().map(AsRef::as_ref).filter(|c| shown(c)).collect();
    CARD_ORDER.iter().filter(|c| shown(c)).enumerate().all(|(i, c)| mine.get(i) == Some(c))
}

/// One codec card of a device
#[derive(Debug, Clone, PartialEq)]
pub struct CardView {
    pub id: &'static str,
    /// Translation key of the name
    pub label: &'static str,
    pub bars: Option<Bars>,
    pub selected: bool,
    /// Its tag (None: untagged)
    pub support: Option<CodecSupport>,
}

/// A device's music codec cards (DeviceCodecs)
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceCardsView {
    /// Its cards' order, which Auto follows under Auto-select too
    pub order: Vec<&'static str>,
    pub picked: Option<String>,
    pub cards: Vec<CardView>,
    /// The device's codecs are not known yet (never connected)
    pub unknown: bool,
    /// The cards are not in their default order: Reset beside the hint
    pub reset: bool,
}

/// The device's cards: `hide` the codecs it does not offer, `auto_all`
/// Auto-select, `drag_order` the order while a card is dragged
pub fn device_cards(
    snap: &Snapshot,
    entry: &AudioEntry,
    hide: bool,
    auto_all: bool,
    drag_order: Option<&[&'static str]>,
) -> DeviceCardsView {
    let known = known_codecs(&snap.devices, &entry.address);
    let on_request = codecs_on_request(&snap.devices, &entry.address);
    let shown = shown_profile(snap, entry);
    let order: Vec<&'static str> = drag_order.map(<[_]>::to_vec).unwrap_or_else(|| card_order(&shown.codec_order));
    let picked = picked_codec(snap, entry, auto_all, &order);
    // Opus 05's card only in developer mode (issue #86); hidden, it keeps its
    // place in the order
    let ids: Vec<&'static str> = offered_codecs(&order, snap.settings.developer_mode)
        .into_iter()
        .filter(|id| codec_shown(id, picked.as_deref(), known, hide, on_request))
        .collect();
    let cards = ids
        .iter()
        .map(|&id| {
            let spec = codec_spec(id);
            let support = shown_support(id, known, on_request);
            CardView {
                id: spec.id,
                label: spec.label,
                bars: spec.bars,
                selected: picked.as_deref() == Some(id),
                // Only supported codecs shown: no "Supported" tag on each
                support: if hide && support == CodecSupport::Offered { None } else { Some(support) },
            }
        })
        .collect();
    DeviceCardsView { reset: !shown_in_default_order(&order, &ids), order, picked, cards, unknown: known.is_empty() }
}

/// On its card, SBC is supported before the first connection too: every
/// device has it (SBC XQ needs what the device lists, so not it)
fn shown_support(id: &str, known: &[String], on_request: &[String]) -> CodecSupport {
    if id == "sbc" && known.is_empty() { CodecSupport::Offered } else { codec_support(id, known, on_request) }
}

/// A card the device does not offer is asked for anyway (issue #86); the
/// codec chosen keeps how it was chosen when only its settings change
pub fn forced_for(snap: &Snapshot, entry: &AudioEntry, shown: &Profile, picked: Option<&str>, id: &str) -> bool {
    if id == card_of(&shown.codec, &shown.quality) && Some(id) == picked {
        return shown.codec_forced;
    }
    let known = known_codecs(&snap.devices, &entry.address);
    codec_support(id, known, codecs_on_request(&snap.devices, &entry.address)) == CodecSupport::NotOffered
}

/// `id` put where `over` is in the full order (hidden cards keep their place)
pub fn placed(list: &[&'static str], id: &str, over: &str) -> Vec<&'static str> {
    match (list.iter().position(|c| *c == id), list.iter().position(|c| *c == over)) {
        (Some(from), Some(to)) => crate::codecs::moved(list, from, to),
        _ => list.to_vec(),
    }
}

/// Alt+arrow keys: `id` next to the card before (`by` -1) or after it (1)
/// among those shown -> the new order and its place (1-based), None at
/// either end
pub fn moved_by(
    order: &[&'static str],
    shown: &[&'static str],
    id: &str,
    by: i32,
) -> Option<(Vec<&'static str>, usize)> {
    let at = shown.iter().position(|c| *c == id)?;
    let to = usize::try_from(at as i64 + i64::from(by)).ok()?;
    let over = shown.get(to)?;
    Some((placed(order, id, over), to + 1))
}

// ---- the microphone's codec (issue #86) ---------------------------------------

/// What the device's microphone link is doing, for its cards
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MicView {
    /// The stream is the device's
    pub ours: bool,
    /// The call link's codec while it is open (classic devices)
    pub call_codec: Option<String>,
    /// LE Audio: the microphone's rate while it runs
    pub le_mic_rate: Option<u32>,
    /// The call codecs the headphones listed
    pub listed_calls: Vec<String>,
    /// LE Audio: the microphone's rates the headphones allow
    pub mic_rates: Vec<u32>,
}

impl MicView {
    /// The microphone link is up
    pub fn up(&self, le_audio: bool) -> bool {
        if le_audio { self.le_mic_rate.is_some() } else { self.call_codec.is_some() }
    }
}

pub fn mic_view(snap: &Snapshot, entry: &AudioEntry) -> MicView {
    let st = &snap.stream;
    let ours = st.state != StreamState::Idle
        && st.profile.is_some()
        && st.profile.as_deref() == entry.profile.as_ref().map(|p| p.name.as_str());
    let voice = if ours { st.voice.as_ref() } else { None };
    MicView {
        ours,
        call_codec: voice.filter(|v| v.link == "hfp").map(|v| v.codec.clone()),
        le_mic_rate: voice.filter(|v| entry.le_audio && v.link == "le").map(|v| v.rate),
        listed_calls: if ours { st.voice_codecs.clone() } else { Vec::new() },
        mic_rates: if ours { st.mic_rates.clone() } else { Vec::new() },
    }
}

/// A card that only shows its codec, with its settings read only (issue
/// #86): the call codecs and LE Audio's
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedCard {
    pub id: &'static str,
    /// Translation key of the name
    pub label: &'static str,
    pub bars: Bars,
    /// Sample rate (Hz) and bit rate (kbps) when fixed
    pub rate: Option<u32>,
    pub kbps: Option<u32>,
    /// No bit rate shown (not known)
    pub no_bit_rate: bool,
}

const fn fixed(
    id: &'static str,
    label: &'static str,
    bars: (u8, u8, u8),
    rate: Option<u32>,
    kbps: Option<u32>,
) -> FixedCard {
    FixedCard {
        id,
        label,
        bars: Bars { quality: bars.0, latency: bars.1, stability: bars.2 },
        rate,
        kbps,
        no_bit_rate: false,
    }
}

/// The call codecs of a classic device's microphone link (HFP), highest
/// quality first. The link chooses the best both sides have (issue #86).
pub const VOICE_CARDS: [FixedCard; 3] = [
    fixed("lc3_swb", "codec.lc3_swb", (3, 4, 3), Some(32000), Some(64)),
    fixed("msbc", "codec.msbc", (2, 4, 4), Some(16000), Some(64)),
    fixed("cvsd", "codec.cvsd", (1, 4, 5), Some(8000), Some(64)),
];

/// The LE Audio microphone: LC3, one channel, at the rate it runs at
pub const LE_MIC_CARD: FixedCard =
    FixedCard { no_bit_rate: true, ..fixed("lc3_mic", "codec.lc3", (3, 4, 4), None, None) };

/// LE Audio's: LC3 as the device offers it best (the default), or its low
/// latency configuration (the profile's le_low_latency)
pub const LE_CARDS: [FixedCard; 2] =
    [fixed("lc3", "codec.lc3", (4, 3, 4), None, None), fixed("lc3_ll", "codec.lc3_ll", (3, 5, 3), None, None)];

/// A hearing aid's (ASHA, a proof of concept): G.722 at 16 kHz, 64 kbit/s
/// for each one, nothing to choose
pub const HEARING_AID_CARDS: [FixedCard; 1] = [fixed("g722", "codec.g722", (2, 3, 4), Some(16000), Some(64))];

/// The call codec cards shown and their tags: the one the link uses, those
/// the headphones list (CVSD always: every headset has it), and with `hide`
/// off all of them, tagged only then (like the music cards)
pub fn call_cards(listed: &[String], call_codec: Option<&str>, hide: bool) -> Vec<(FixedCard, Option<CodecSupport>)> {
    let supported = |id: &str| id == "cvsd" || listed.iter().any(|c| c == id);
    VOICE_CARDS
        .iter()
        .filter(|c| Some(c.id) == call_codec || supported(c.id) || !hide)
        .map(|c| {
            let tag = (!hide).then(|| if supported(c.id) { CodecSupport::Offered } else { CodecSupport::NotOffered });
            (*c, tag)
        })
        .collect()
}

/// A read-only card's sample rate and bit rate as its "..." shows them:
/// its own, else the stream's while it plays that card (`info`)
pub fn fixed_values(card: &FixedCard, picked: &str, info: Option<&StreamInfo>) -> (String, String) {
    let info = info.filter(|_| card.id == picked);
    let rate = card.rate.or(info.map(|i| i.sample_rate)).filter(|r| *r > 0);
    let kbps = card.kbps.or(info.map(|i| i.bitrate_kbps)).filter(|k| *k > 0);
    (
        rate.map_or_else(|| "—".to_owned(), |r| format!("{r} Hz")),
        kbps.map_or_else(|| "—".to_owned(), |k| format!("{k} kbps")),
    )
}

/// The LE Audio microphone's rates to choose between: Auto with the one it
/// takes, then each the headphones allow (one chosen they no longer allow
/// stays, marked); None while not known or only one
pub fn mic_rate_options(value: u32, rates: &[u32]) -> Option<Vec<Opt<u32>>> {
    if rates.len() < 2 {
        return None;
    }
    let label = |hz: u32| format!("{hz} Hz");
    let mut listed = rates.to_vec();
    if value != 0 && !rates.contains(&value) {
        listed.push(value);
    }
    let mut out = vec![Opt { value: 0, label: t!("codec.auto_value", label(rates[0])) }];
    out.extend(listed.into_iter().map(|hz| Opt {
        value: hz,
        label: if rates.contains(&hz) { label(hz) } else { t!("ui.profile.sbc_not_offered", label(hz)) },
    }));
    Some(out)
}

// ---- a codec's "..." dialog --------------------------------------------------------

/// The SBC / SBC XQ bitpool range the dialog offers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BitpoolRange {
    pub min: u32,
    pub max: u32,
}

/// The SBC / SBC XQ bitpool range the dialog offers at a rate: the
/// codec's (SBC XQ's lower at 48 kHz), within the device's when known
/// (issue #86: a device of max bitpool 39 is not offered 53; the core keeps
/// the encoder within it the same way)
pub fn bitpool_range(codec: &str, caps: Option<&SbcCaps>, rate: u32, unlisted: bool) -> BitpoolRange {
    // SBC XQ not held to the device's list: up to SBC's 53 (issue #86)
    let local = if codec == "sbcxq" && !unlisted { xq_max_bitpool(rate) } else { BITPOOL_MAX };
    let Some(caps) = caps.filter(|c| c.max_bitpool >= BITPOOL_MIN) else {
        return BitpoolRange { min: BITPOOL_MIN, max: local };
    };
    let max = local.min(caps.max_bitpool).max(BITPOOL_MIN);
    BitpoolRange { min: caps.min_bitpool.max(BITPOOL_MIN).min(max), max }
}

/// What the dialog goes by besides its values
#[derive(Debug, Clone, Copy)]
pub struct SettingsProps<'a> {
    /// Device.codec_caps (empty: not known), as the settings take them
    pub caps: &'a [CodecCaps],
    pub sbc_caps: Option<&'a SbcCaps>,
    /// Offers the SBC frame setup, which only counts then
    pub developer: bool,
    /// Developer mode: Auto follows the PC's output format, as far as the
    /// device lists it (no value beside Auto while the format is not known)
    pub match_source: bool,
    /// The format Windows plays the profile's source in: rate, bits (None:
    /// not known)
    pub source_format: Option<(u32, u32)>,
    /// Who listed what is offered, once it did: "Show only supported
    /// options" names it
    pub device_name: &'a str,
}

/// SBC and SBC XQ keep their own bitpool and minimum
fn bitpools(v: &CodecChoice) -> (u8, u8) {
    if v.codec == "sbcxq" { (v.sbcxq_bitpool, v.sbcxq_min_bitpool) } else { (v.sbc_bitpool, v.sbc_min_bitpool) }
}

fn set_bitpools(v: &mut CodecChoice, high: u8, low: u8) {
    if v.codec == "sbcxq" {
        v.sbcxq_bitpool = high;
        v.sbcxq_min_bitpool = low;
    } else {
        v.sbc_bitpool = high;
        v.sbc_min_bitpool = low;
    }
}

fn to_u8(x: u32) -> u8 {
    u8::try_from(x).unwrap_or(u8::MAX)
}

/// `from` with what the device does not list back to Auto, a bitpool
/// above the device's largest down to it, a quality capped like a higher
/// one to High (issue #86); all of it kept with unlisted options
pub fn fitted(from: &CodecChoice, props: &SettingsProps) -> CodecChoice {
    let s = codec_spec(&from.codec);
    let mut out = from.clone();
    if out.unlisted_options {
        return out;
    }
    let caps = caps_of(props.caps, s.id);
    if !offered_sample_rates(s.id, caps).contains(&out.sample_rate) {
        out.sample_rate = 0;
    }
    if !offered_bit_depths(s.id, caps).contains(&out.bit_depth) {
        out.bit_depth = 0;
    }
    if !s.qualities.is_empty()
        && !offered_qualities(s.id, out.sample_rate, caps.map_or(0, |c| c.max_bitrate), "")
            .iter()
            .any(|q| q.id == out.quality)
    {
        out.quality = "hq".into();
    }
    if s.bitpool {
        let range = bitpool_range(s.id, props.sbc_caps, sbc_rate(out.sample_rate, props.sbc_caps), false);
        let (mut high, mut low) = bitpools(&out);
        let max = to_u8(range.max);
        if high > max {
            high = max;
        }
        // The minimum likewise, and never above the maximum
        if low > max {
            low = max;
        }
        if u32::from(low) < range.min && low != 0 {
            low = to_u8(range.min);
        }
        if high != 0 && low > high {
            low = high;
        }
        set_bitpools(&mut out, high, low);
    }
    out
}

/// The values a codec's dialog opens with: the profile's fitted to the
/// codec, or what that codec was last set to (`kept`: the profile's
/// codec_settings for it)
pub fn settings_start(card: &str, profile: &Profile, kept: Option<&CodecValues>, props: &SettingsProps) -> CodecChoice {
    let s = codec_spec(card);
    // The profile's codec as its card (aptX Adaptive in Low Latency: LL's)
    let mut own = CodecChoice::from(profile);
    own.codec = card_of(&profile.codec, &profile.quality).to_owned();
    let p = match kept {
        Some(k) if s.id != own.codec => choice_of(s.id, k),
        _ => own,
    };
    // From another codec's values: without its ABR or SBC frame setup
    // (SBC and SBC XQ apart, issue #86)
    let mut from = carry_to_codec(&p, s.id);
    // SBC / SBC XQ: no quality to choose, Auto is the highest bitpool
    if s.bitpool {
        from.quality = "hq".into();
    }
    fitted(&from, props)
}

/// What the device listed for the codec (None: not known)
fn listed_caps<'a>(v: &CodecChoice, props: &SettingsProps<'a>) -> Option<&'a CodecCaps> {
    caps_of(props.caps, &v.codec)
}

/// What the dialog offers: what the device listed, or with "Show only
/// supported options" off everything the codec has
fn offered_caps<'a>(v: &CodecChoice, props: &SettingsProps<'a>) -> Option<&'a CodecCaps> {
    let listed = listed_caps(v, props);
    if v.unlisted_options { unlisted_caps(&v.codec, listed) } else { listed }
}

fn sbc_caps<'a>(v: &CodecChoice, props: &SettingsProps<'a>) -> Option<&'a SbcCaps> {
    if v.unlisted_options { None } else { props.sbc_caps }
}

fn range_of(v: &CodecChoice, props: &SettingsProps) -> BitpoolRange {
    let caps = sbc_caps(v, props);
    bitpool_range(&v.codec, caps, sbc_rate(v.sample_rate, caps), v.unlisted_options)
}

/// A bitpool from the profile outside the device's range (the slider
/// itself stays within it) -> the error under the slider
pub fn bitpool_error(v: &CodecChoice, props: &SettingsProps) -> Option<String> {
    if !codec_spec(&v.codec).bitpool {
        return None;
    }
    let r = range_of(v, props);
    let outside = |x: u8| x != 0 && !(u32::from(x) >= r.min && u32::from(x) <= r.max);
    let (high, low) = bitpools(v);
    (outside(high) || outside(low)).then(|| t!("error.sbc_bitpool_range", r.min, r.max))
}

/// Where Auto puts the highest bitpool: the quality's (High) within the
/// device's range, as the core does
fn auto_max(v: &CodecChoice, props: &SettingsProps) -> u32 {
    let caps = sbc_caps(v, props);
    let mut at = v.clone();
    set_bitpools(&mut at, 0, 0);
    let r = range_of(v, props);
    sbc_setup(&at, sbc_rate(v.sample_rate, caps), caps).bitpool.max(r.min).min(r.max)
}

/// The two handles' places: the highest where Auto puts it while on Auto,
/// the lowest at the range's start
fn handles(v: &CodecChoice, props: &SettingsProps) -> (u32, u32) {
    let r = range_of(v, props);
    let (high, low) = bitpools(v);
    let clamp = |x: u32| x.max(r.min).min(r.max);
    let hi = clamp(if high != 0 { u32::from(high) } else { auto_max(v, props).max(u32::from(low)) });
    let lo = clamp(if low != 0 { u32::from(low) } else { r.min }).min(hi);
    (lo, hi)
}

/// One change in the dialog
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Quality(String),
    SampleRate(u32),
    BitDepth(u32),
    /// ABR, or AAC's VBR
    Abr(bool),
    Lhdcv5LowLatency(bool),
    /// The slider's lowest handle moved (never past the highest)
    LowBitpool(u32),
    /// The slider's highest handle moved (never below the lowest)
    HighBitpool(u32),
    /// Both handles back to Auto
    BitpoolAuto,
    Blocks(u8),
    Subbands(u8),
    Allocation(String),
    ChannelMode(String),
    /// "Show only supported options" off (true) or on again
    Unlisted(bool),
}

/// Applies a change -> whether it goes to the profile (not while a bitpool
/// is outside the device's range)
pub fn apply_change(v: &mut CodecChoice, change: Change, props: &SettingsProps) -> bool {
    match change {
        Change::Quality(q) => v.quality = q,
        Change::SampleRate(r) => {
            v.sample_rate = r;
            // SBC XQ's top bitpool is lower at 48 kHz: a higher one comes down to it
            if codec_spec(&v.codec).bitpool {
                let top = to_u8(range_of(v, props).max);
                let (high, low) = bitpools(v);
                set_bitpools(v, high.min(top), low.min(top));
            }
        }
        Change::BitDepth(b) => v.bit_depth = b,
        Change::Abr(on) => v.abr = on,
        Change::Lhdcv5LowLatency(on) => v.lhdcv5_low_latency = on,
        Change::LowBitpool(x) => {
            let r = range_of(v, props);
            let (_, hi) = handles(v, props);
            let (high, _) = bitpools(v);
            set_bitpools(v, high, to_u8(x.max(r.min).min(r.max).min(hi)));
        }
        Change::HighBitpool(x) => {
            let r = range_of(v, props);
            let (lo, _) = handles(v, props);
            let (_, low) = bitpools(v);
            set_bitpools(v, to_u8(x.max(r.min).min(r.max).max(lo)), low);
        }
        Change::BitpoolAuto => set_bitpools(v, 0, 0),
        Change::Blocks(x) => v.sbc_blocks = x,
        Change::Subbands(x) => v.sbc_subbands = x,
        Change::Allocation(x) => v.sbc_allocation = x,
        Change::ChannelMode(x) => v.sbc_channel_mode = x,
        // "Show only supported options" turned on again: back within what
        // the device lists
        Change::Unlisted(on) => {
            let mut next = v.clone();
            next.unlisted_options = on;
            *v = fitted(&next, props);
        }
    }
    bitpool_error(v, props).is_none()
}

/// A choice of a list
#[derive(Debug, Clone, PartialEq)]
pub struct Opt<T> {
    pub value: T,
    pub label: String,
}

/// A setting: a fixed value shown, or a list to choose from
#[derive(Debug, Clone, PartialEq)]
pub enum Field<T> {
    Fixed(String),
    Choice { value: T, options: Vec<Opt<T>>, disabled: bool },
}

/// The SBC / SBC XQ bitpool slider (BitpoolSlider): the lowest bitpool
/// (the SET_CONFIGURATION's minimum and the floor of ABR) and the highest
/// (the bitpool the stream starts with), each 0 for Auto
#[derive(Debug, Clone, PartialEq)]
pub struct BitpoolView {
    pub min: u32,
    pub max: u32,
    /// The handles' places
    pub low: u32,
    pub high: u32,
    /// What each says (Auto: "Auto (n)")
    pub low_text: String,
    pub high_text: String,
    /// The highest's value beside the slider
    pub high_shown: String,
    /// Both on Auto: the Auto button is disabled
    pub auto: bool,
    pub error: Option<String>,
}

/// SBC's frame setup in developer mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKey {
    Blocks,
    Subbands,
    Allocation,
    ChannelMode,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FrameField {
    pub key: FrameKey,
    pub title: String,
    /// The values as text ("0" / "" = Auto)
    pub field: Field<String>,
}

/// Everything the dialog shows, in its order
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsView {
    pub title: String,
    /// "Show only supported options" (named after the device) and whether
    /// it is on; None until the device listed what it offers
    pub listed: Option<(String, bool)>,
    /// SBC and SBC XQ: the bit rate the bitpool gives, fixed
    pub quality: Field<String>,
    pub rate: Field<u32>,
    pub depth: Field<u32>,
    pub abr: Option<bool>,
    pub vbr: Option<bool>,
    pub low_latency: Option<bool>,
    pub bitpool: Option<BitpoolView>,
    /// Developer mode: blocks, subbands, allocation (and SBC's channel mode)
    pub frame: Vec<FrameField>,
    /// The channel mode, fixed for each codec unless chosen above
    pub channel_mode: Option<String>,
    /// OPPO's Hi-Res request again (LHDC V5, developer mode)
    pub ask_hi_res: bool,
}

/// "48000 Hz"-style rate the bit rate is shown at, from a codec's listed
/// rates (as sbc_rate with the codec's caps)
fn shown_rate(sample_rate: u32, caps: Option<&CodecCaps>) -> u32 {
    if sample_rate == 44100 || sample_rate == 48000 {
        return sample_rate;
    }
    match caps {
        Some(c) if !c.sample_rates.contains(&48000) && c.sample_rates.contains(&44100) => 44100,
        _ => 48000,
    }
}

fn bit_depth_label(bits: u32) -> String {
    t(&format!("bitdepth.{bits}"))
}

/// The dialog's content for its values `v`
pub fn settings_view(v: &CodecChoice, props: &SettingsProps) -> SettingsView {
    let spec = codec_spec(&v.codec);
    let name = t(spec.label);
    let listed_caps = listed_caps(v, props);
    let caps = offered_caps(v, props);
    let sbc = sbc_caps(v, props);
    let max_bitrate = caps.map_or(0, |c| c.max_bitrate);
    let listed =
        !props.device_name.is_empty() && if spec.bitpool { props.sbc_caps.is_some() } else { listed_caps.is_some() };
    let error = bitpool_error(v, props);
    // What streams: outside developer mode the frame setup kept is not used
    let mut streamed = v.clone();
    if !props.developer {
        streamed.sbc_blocks = 0;
        streamed.sbc_subbands = 0;
        streamed.sbc_allocation.clear();
        streamed.sbc_channel_mode.clear();
    }

    // SBC and SBC XQ: the bitpool decides the bit rate, shown as worked out
    // (issue #86); one quality (or a fixed bit rate): shown, not a choice
    let qualities = offered_qualities(&v.codec, v.sample_rate, max_bitrate, &v.quality);
    let quality_text = |label: &str, kbps: &Option<String>| match kbps {
        Some(k) => format!("{} ({k} kbps)", t(label)),
        None => t(label),
    };
    let quality = if spec.bitpool {
        let text = if error.is_some() { String::new() } else { sbc_kbps_text(&streamed, sbc) };
        Field::Fixed(if text.is_empty() { "–".into() } else { text })
    } else if qualities.len() > 1 {
        Field::Choice {
            value: v.quality.clone(),
            options: qualities
                .iter()
                .map(|q| Opt { value: q.id.to_owned(), label: quality_text(q.label, &q.kbps) })
                .collect(),
            disabled: quality_ignored(&v.codec, v.abr),
        }
    } else {
        Field::Fixed(match qualities.first() {
            Some(q) => quality_text(q.label, &q.kbps),
            None => fixed_kbps_text(&v.codec, shown_rate(v.sample_rate, caps)),
        })
    };

    // One rate the device takes: shown as a value, without Auto
    let offered_rates = offered_sample_rates(&v.codec, caps);
    let rate = match only_choice(&offered_rates, &v.sample_rate, &0) {
        Some(r) => Field::Fixed(format!("{r} Hz")),
        None => {
            let mut rates = offered_rates.clone();
            if !rates.contains(&v.sample_rate) {
                rates.push(v.sample_rate);
            }
            // Auto with the value it streams at (issue #86); matching the PC's
            // output, the value only once its format is known
            let auto_label = if !props.match_source {
                t!("codec.auto_value", format!("{} Hz", auto_sample_rate(&v.codec, caps)))
            } else {
                match props.source_format {
                    Some((r, _)) => t!("codec.auto_value", format!("{} Hz", matched_sample_rate(&v.codec, caps, r))),
                    None => t("codec.auto"),
                }
            };
            let listed_rates = offered_sample_rates(&v.codec, listed_caps);
            Field::Choice {
                value: v.sample_rate,
                options: rates
                    .into_iter()
                    .map(|r| {
                        let label = if r == 0 { auto_label.clone() } else { format!("{r} Hz") };
                        let label =
                            if listed_rates.contains(&r) { label } else { t!("ui.profile.sbc_not_offered", label) };
                        Opt { value: r, label }
                    })
                    .collect(),
                disabled: false,
            }
        }
    };

    // One bit depth (SBC, AAC, aptX, Opus: 16-bit; aptX Adaptive: 24-bit;
    // or the one the device lists): shown as a value, not as a list to open
    let offered_depths = offered_bit_depths(&v.codec, caps);
    let mut depths = offered_depths.clone();
    if !depths.contains(&v.bit_depth) {
        depths.push(v.bit_depth);
    }
    let only_depth =
        if depths.len() > 1 { only_choice(&offered_depths, &v.bit_depth, &0) } else { Some(fixed_bit_depth(&v.codec)) };
    let depth = match only_depth {
        Some(b) => Field::Fixed(bit_depth_label(b)),
        None => {
            let bits = if !props.match_source {
                auto_bit_depth(&v.codec, caps)
            } else {
                props.source_format.map_or(0, |(_, b)| matched_bit_depth(&v.codec, caps, b))
            };
            let auto_label = if bits != 0 { t!("codec.auto_value", bit_depth_label(bits)) } else { t("codec.auto") };
            let listed_depths = offered_bit_depths(&v.codec, listed_caps);
            Field::Choice {
                value: v.bit_depth,
                options: depths
                    .into_iter()
                    .map(|b| Opt {
                        value: b,
                        label: if b == 0 {
                            auto_label.clone()
                        } else if listed_depths.contains(&b) {
                            bit_depth_label(b)
                        } else {
                            t!("ui.profile.sbc_not_offered", bit_depth_label(b))
                        },
                    })
                    .collect(),
                disabled: false,
            }
        }
    };

    let bitpool = spec.bitpool.then(|| {
        let r = range_of(v, props);
        let (high, low) = bitpools(v);
        let (lo, hi) = handles(v, props);
        let auto_text = |x: u32| t!("ui.profile.sbc_bitpool_auto", x);
        BitpoolView {
            min: r.min,
            max: r.max,
            low: lo,
            high: hi,
            low_text: if low != 0 { lo.to_string() } else { auto_text(lo) },
            high_text: if high != 0 { hi.to_string() } else { auto_text(hi) },
            high_shown: if high != 0 { hi.to_string() } else { t!("ui.profile.sbc_bitpool_auto_high", hi) },
            auto: high == 0 && low == 0,
            error: error.clone(),
        }
    });

    // The frame setup: Auto, then what the device listed (the value chosen
    // stays, marked, when it did not list it: it falls back when
    // connecting); one value listed is shown fixed, as the rate and bit
    // depth are
    let frame_field = |key: FrameKey,
                       title: &str,
                       field: SbcField,
                       values: Vec<String>,
                       current: String,
                       auto: &str| {
        let label = |x: &str| match key {
            FrameKey::Allocation => t(&format!("ui.diag.cd_alloc.{x}")),
            FrameKey::ChannelMode => t(&format!("ui.diag.cd_mode.{x}")),
            _ => x.to_owned(),
        };
        let numeric = matches!(key, FrameKey::Blocks | FrameKey::Subbands);
        let none = if numeric { "0".to_owned() } else { String::new() };
        let offered = |caps: Option<&SbcCaps>, x: &str| {
            if numeric { sbc_offered(caps, field, x.parse::<u32>().unwrap_or(0)) } else { sbc_offered(caps, field, x) }
        };
        let listed: Vec<String> = values.iter().filter(|x| offered(sbc, x)).cloned().collect();
        let field = match only_choice(&listed, &current, &none) {
            Some(only) => Field::Fixed(label(&only)),
            None => {
                let mut options = vec![Opt { value: none.clone(), label: t!("codec.auto_value", label(auto)) }];
                options.extend(values.iter().filter(|x| **x == current || offered(sbc, x)).map(|x| Opt {
                    value: x.clone(),
                    label: if offered(props.sbc_caps, x) {
                        label(x)
                    } else {
                        t!("ui.profile.sbc_not_offered", label(x))
                    },
                }));
                Field::Choice { value: current, options, disabled: false }
            }
        };
        FrameField { key, title: t(title), field }
    };
    let mut frame = Vec::new();
    if spec.bitpool && props.developer {
        let nums = |xs: &[u32]| xs.iter().map(u32::to_string).collect::<Vec<_>>();
        let strs = |xs: &[&str]| xs.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
        frame.push(frame_field(
            FrameKey::Blocks,
            "ui.diag.cd_blocks",
            SbcField::Blocks,
            nums(&SBC_BLOCKS),
            v.sbc_blocks.to_string(),
            "16",
        ));
        frame.push(frame_field(
            FrameKey::Subbands,
            "ui.diag.cd_subbands",
            SbcField::Subbands,
            nums(&SBC_SUBBANDS),
            v.sbc_subbands.to_string(),
            "8",
        ));
        frame.push(frame_field(
            FrameKey::Allocation,
            "ui.diag.cd_allocation",
            SbcField::Allocations,
            strs(&SBC_ALLOCATIONS),
            v.sbc_allocation.clone(),
            "loudness",
        ));
        // SBC XQ is dual channel by definition
        if v.codec == "sbc" {
            frame.push(frame_field(
                FrameKey::ChannelMode,
                "ui.diag.cd_channel_mode",
                SbcField::ChannelModes,
                strs(&SBC_CHANNEL_MODES),
                v.sbc_channel_mode.clone(),
                "joint_stereo",
            ));
        }
    }
    // Fixed for each codec, unless chosen above (issue #86)
    let channel_mode = (spec.bitpool && (!props.developer || v.codec == "sbcxq")).then(|| {
        let mode = sbc_setup(&streamed, sbc_rate(v.sample_rate, sbc), sbc).mode;
        t(&format!("ui.diag.cd_mode.{mode}"))
    });

    SettingsView {
        title: t!("ui.profile.codec_settings", &name),
        listed: listed.then(|| (t!("ui.profile.codec_options_listed", props.device_name), !v.unlisted_options)),
        quality,
        rate,
        depth,
        abr: spec.abr.then_some(v.abr),
        vbr: spec.vbr.then_some(v.abr),
        low_latency: low_latency_offered(&v.codec, listed_caps).then_some(v.lhdcv5_low_latency),
        bitpool,
        frame,
        channel_mode,
        ask_hi_res: v.codec == "lhdcv5" && listed && props.developer,
    }
}

/// A frame field's value chosen (as text) -> its change
pub fn frame_change(key: FrameKey, value: &str) -> Change {
    match key {
        FrameKey::Blocks => Change::Blocks(value.parse().unwrap_or(0)),
        FrameKey::Subbands => Change::Subbands(value.parse().unwrap_or(0)),
        FrameKey::Allocation => Change::Allocation(value.to_owned()),
        FrameKey::ChannelMode => Change::ChannelMode(value.to_owned()),
    }
}

#[cfg(test)]
#[path = "codecs_tests.rs"]
mod tests;

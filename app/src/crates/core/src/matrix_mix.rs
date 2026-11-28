//! The matrix mixers (developer mode): which source speakers make each
//! output (FL FR RL RR), with a delay and polarity per output. One for
//! Auracast ([`Target::Auracast`], auracast_mix.json) and one for the
//! earbuds of an LE Audio stream to a device ([`Target::Tws`], FL FR only,
//! tws_mix.json, used once turned on there), each with the user's named
//! presets. One [`MixHandle`] per mixer is shared with the encode thread of
//! the stream running, so a change is heard from the next block of audio,
//! without restarting.

use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use ipc::{AuracastMix, AuracastMixPreset, AuracastMixState, AuracastSub};
use pipeline::mix::{
    CROSSOVER_DEFAULT_HZ, FULL_RANGE_DEFAULT, INPUTS, MixHandle, MixSettings, Outputs, auto_matrix, effective_matrix,
    present_columns,
};
use serde::{Deserialize, Serialize};

const OUTPUTS: usize = 4;

/// Which mixer
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// The broadcast's (outputs FL FR RL RR)
    Auracast,
    /// The earbuds' of an LE Audio stream to a device (FL FR)
    Tws,
}

impl Target {
    fn name(self) -> &'static str {
        match self {
            Target::Auracast => "Auracast mixer",
            Target::Tws => "TWS mixer",
        }
    }

    fn file(self, dir: &Path) -> PathBuf {
        match self {
            Target::Auracast => crate::paths::auracast_mix_file(dir),
            Target::Tws => crate::paths::tws_mix_file(dir),
        }
    }
}

/// auracast_mix.json, tws_mix.json
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct MixFile {
    current: Option<AuracastMix>,
    presets: Vec<AuracastMixPreset>,
    /// TWS: the mixer is used (from the next connection on)
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    enabled: bool,
}

/// The source and outputs of the stream running with the mixer
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Live {
    pub channels: usize,
    pub mask: u32,
    pub rear: bool,
    /// A subwoofer layout: what the third and fourth outputs are
    pub sub: AuracastSub,
}

/// The mixer's outputs for a stream with or without rear outputs, and its
/// subwoofer
pub fn outputs(rear: bool, sub: AuracastSub) -> Outputs {
    match (rear, sub) {
        (false, _) => Outputs::Front,
        (true, AuracastSub::None) => Outputs::Rear,
        (true, AuracastSub::Stereo) => Outputs::Sub,
        (true, AuracastSub::Mono) => Outputs::SubMono,
        (true, AuracastSub::Center) => Outputs::Center,
    }
}

/// The subwoofer of the mixer's outputs, as the API names it
pub fn sub_of(outputs: Outputs) -> AuracastSub {
    match outputs {
        Outputs::Front | Outputs::Rear => AuracastSub::None,
        Outputs::Sub => AuracastSub::Stereo,
        Outputs::SubMono => AuracastSub::Mono,
        Outputs::Center => AuracastSub::Center,
    }
}

fn initial(target: Target) -> MixHandle {
    let h = MixHandle::named(target.name());
    // Tests keep to their own folders: the defaults
    if cfg!(test) {
        return h;
    }
    let file = load(target, &crate::paths::config_dir());
    if let Some(m) = &file.current {
        h.set(settings(m));
    }
    h
}

static AURACAST: LazyLock<MixHandle> = LazyLock::new(|| initial(Target::Auracast));
static TWS: LazyLock<MixHandle> = LazyLock::new(|| initial(Target::Tws));
static LIVE_AURACAST: Mutex<Option<Live>> = Mutex::new(None);
static LIVE_TWS: Mutex<Option<Live>> = Mutex::new(None);

/// The mixer the encode thread of the stream reads
pub fn handle(target: Target) -> MixHandle {
    match target {
        Target::Auracast => AURACAST.clone(),
        Target::Tws => TWS.clone(),
    }
}

fn live_slot(target: Target) -> &'static Mutex<Option<Live>> {
    match target {
        Target::Auracast => &LIVE_AURACAST,
        Target::Tws => &LIVE_TWS,
    }
}

/// The stream running with the mixer (None: it ended, and solo and mute
/// with it: a listening test does not silence the next stream)
pub fn set_live(target: Target, live: Option<Live>) {
    *live_slot(target).lock().unwrap_or_else(|e| e.into_inner()) = live;
    if live.is_none() {
        let h = handle(target);
        let s = h.get();
        if s.monitoring() {
            h.set(s.unmonitored());
        }
    }
}

fn live(target: Target) -> Option<Live> {
    *live_slot(target).lock().unwrap_or_else(|e| e.into_inner())
}

/// TWS: the mixer for a stream to a device starting now, when turned on
pub fn tws_for_stream(dir: &Path) -> Option<MixHandle> {
    load(Target::Tws, dir).enabled.then(|| handle(Target::Tws))
}

/// A file that cannot be read is put aside as <name>.json.bad, so the next
/// save does not write over the presets in it
fn load(target: Target, dir: &Path) -> MixFile {
    let path = target.file(dir);
    let what = target.name();
    match std::fs::read_to_string(&path) {
        Ok(t) => serde_json::from_str(&t).unwrap_or_else(|e| {
            let aside = path.with_extension("json.bad");
            match std::fs::rename(&path, &aside) {
                Ok(()) => tracing::warn!("{what}: settings not read ({e}), kept as {}", aside.display()),
                Err(io) => tracing::warn!("{what}: settings not read ({e}) and not put aside ({io})"),
            }
            MixFile::default()
        }),
        Err(_) => MixFile::default(),
    }
}

fn save(target: Target, dir: &Path, file: &MixFile) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(file).map_err(std::io::Error::other)?;
    let path = target.file(dir);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// What the encode thread takes, from the UI's form (missing entries 0)
pub fn settings(m: &AuracastMix) -> MixSettings {
    let mut matrix = [[0.0f32; INPUTS]; OUTPUTS];
    for (row, r) in matrix.iter_mut().zip(&m.matrix) {
        for (g, x) in row.iter_mut().zip(r) {
            *g = *x;
        }
    }
    let mut delay_ms = [0.0f32; OUTPUTS];
    for (d, x) in delay_ms.iter_mut().zip(&m.delay_ms) {
        *d = *x;
    }
    let flags = |v: &[bool]| -> [bool; OUTPUTS] { std::array::from_fn(|i| v.get(i).copied().unwrap_or(false)) };
    let inputs = |v: &[bool]| -> [bool; INPUTS] { std::array::from_fn(|i| v.get(i).copied().unwrap_or(false)) };
    MixSettings {
        auto: m.auto,
        center_db: m.center_db,
        lfe_db: m.lfe_db,
        surround_db: m.surround_db,
        normalize: m.normalize,
        matrix,
        delay_ms,
        invert: flags(&m.invert),
        crossover_hz: if m.crossover_hz > 0.0 { m.crossover_hz } else { CROSSOVER_DEFAULT_HZ },
        mains_full_range: m.mains_full_range,
        full_range: if m.full_range.is_empty() { FULL_RANGE_DEFAULT } else { flags(&m.full_range) },
        solo_in: inputs(&m.solo_in),
        mute_in: inputs(&m.mute_in),
        solo_out: flags(&m.solo_out),
        mute_out: flags(&m.mute_out),
    }
    .held()
}

/// As the UI gets it
pub fn to_ipc(s: &MixSettings) -> AuracastMix {
    AuracastMix {
        auto: s.auto,
        center_db: s.center_db,
        lfe_db: s.lfe_db,
        surround_db: s.surround_db,
        normalize: s.normalize,
        matrix: rows(&s.matrix),
        delay_ms: s.delay_ms.to_vec(),
        invert: s.invert.to_vec(),
        crossover_hz: s.crossover_hz,
        mains_full_range: s.mains_full_range,
        full_range: s.full_range.to_vec(),
        solo_in: s.solo_in.to_vec(),
        mute_in: s.mute_in.to_vec(),
        solo_out: s.solo_out.to_vec(),
        mute_out: s.mute_out.to_vec(),
    }
}

fn rows(m: &pipeline::mix::Matrix) -> Vec<Vec<f32>> {
    m.iter().map(|r| r.to_vec()).collect()
}

/// The program's own presets (names are ids the UI translates)
pub fn builtin_presets(target: Target) -> Vec<AuracastMixPreset> {
    let all = [true; INPUTS];
    let standard = MixSettings::default();
    let manual = |matrix| MixSettings { auto: false, matrix, ..standard };
    let rear = auto_matrix(&all, false, Outputs::Rear, &standard);
    let fold = auto_matrix(&all, false, Outputs::Front, &standard);
    let zero = [0.0f32; INPUTS];
    let list = match target {
        Target::Auracast => vec![
            ("standard", standard),
            ("normalized", MixSettings { normalize: true, ..standard }),
            ("front_everywhere", manual([fold[0], fold[1], fold[0], fold[1]])),
            ("front_only", manual([fold[0], fold[1], zero, zero])),
            ("rear_only", manual([zero, zero, rear[2], rear[3]])),
            ("swapped", manual([rear[1], rear[0], rear[3], rear[2]])),
        ],
        Target::Tws => {
            // Half of each side, on both earbuds
            let mono: [f32; INPUTS] = std::array::from_fn(|i| (fold[0][i] + fold[1][i]) / 2.0);
            vec![
                ("standard", standard),
                ("normalized", MixSettings { normalize: true, ..standard }),
                ("swapped", manual([fold[1], fold[0], zero, zero])),
                ("mono", manual([mono, mono, zero, zero])),
            ]
        }
    };
    list.into_iter()
        .map(|(name, s)| AuracastMixPreset { name: name.into(), mix: to_ipc(&s.held()), builtin: true })
        .collect()
}

/// The state for the stream running, else for a source of `channels`
/// (default 8, the usual speakers) with or without rear outputs (TWS:
/// without unless asked), with a subwoofer layout or not
pub fn state(
    target: Target,
    dir: &Path,
    channels: Option<u32>,
    rear: Option<bool>,
    sub: Option<AuracastSub>,
) -> AuracastMixState {
    let file = load(target, dir);
    let s = handle(target).get();
    let live = live(target);
    let (channels, mask, rear, sub) = match live {
        Some(l) => (l.channels, l.mask, l.rear, l.sub),
        None => {
            let sub = sub.unwrap_or_default();
            let rear = rear.unwrap_or(target == Target::Auracast) || sub != AuracastSub::None;
            (channels.map_or(8, |c| c.clamp(1, 8) as usize), 0, rear, sub)
        }
    };
    let sub = if rear { sub } else { AuracastSub::None };
    let mut presets = builtin_presets(target);
    presets.extend(file.presets.into_iter().map(|p| AuracastMixPreset { builtin: false, ..p }));
    AuracastMixState {
        mix: to_ipc(&s),
        presets,
        live: live.is_some(),
        channels: channels as u32,
        present: present_columns(channels, mask).to_vec(),
        rear,
        sub,
        effective: rows(&effective_matrix(channels, mask, outputs(rear, sub), &s)),
        enabled: target == Target::Auracast || file.enabled,
    }
}

/// New settings: heard at once while the stream runs with the mixer, and
/// saved (but for solo and mute, which last until the stream ends)
pub fn set(target: Target, dir: &Path, m: &AuracastMix) -> std::io::Result<()> {
    let s = settings(m);
    handle(target).set(s);
    let mut file = load(target, dir);
    file.current = Some(to_ipc(&s.unmonitored()));
    save(target, dir, &file)
}

/// TWS: the mixer used from the next connection on (or not)
pub fn set_tws_enabled(dir: &Path, on: bool) -> std::io::Result<()> {
    let mut file = load(Target::Tws, dir);
    if file.enabled != on {
        tracing::info!("TWS mixer: {}", if on { "on from the next connection" } else { "off" });
    }
    file.enabled = on;
    save(Target::Tws, dir, &file)
}

/// Saves `m` as the preset `name` (replacing one of that name)
pub fn save_preset(target: Target, dir: &Path, name: &str, m: &AuracastMix) -> std::io::Result<()> {
    let name = name.trim();
    if name.is_empty() {
        return Err(std::io::Error::other("no name"));
    }
    // The built-in ones go by these names: one saved under such a name
    // could not be chosen or deleted
    if builtin_presets(target).iter().any(|p| p.name == name) {
        return Err(std::io::Error::other("the name of a built-in preset"));
    }
    let mut file = load(target, dir);
    let mix = to_ipc(&settings(m).unmonitored());
    match file.presets.iter_mut().find(|p| p.name == name) {
        Some(p) => p.mix = mix,
        None => file.presets.push(AuracastMixPreset { name: name.into(), mix, builtin: false }),
    }
    save(target, dir, &file)
}

pub fn delete_preset(target: Target, dir: &Path, name: &str) -> std::io::Result<()> {
    let mut file = load(target, dir);
    file.presets.retain(|p| p.name != name);
    save(target, dir, &file)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Target = Target::Auracast;

    #[test]
    fn the_form_round_trips_and_is_held_to_the_ranges() {
        let s = MixSettings {
            delay_ms: [1.25, 0.0, 600.0, 0.0],
            invert: [false, true, false, false],
            ..Default::default()
        };
        let m = to_ipc(&s);
        assert_eq!(m.matrix.len(), 4);
        assert!(m.matrix.iter().all(|r| r.len() == 8));
        let back = settings(&m);
        assert_eq!(back.delay_ms, [1.25, 0.0, 500.0, 0.0]);
        assert_eq!(back.invert, s.invert);
        // Short rows: the rest at 0
        let short = AuracastMix { matrix: vec![vec![1.0]], ..Default::default() };
        assert_eq!(settings(&short).matrix[0][..2], [1.0, 0.0]);
    }

    #[test]
    fn presets_are_saved_replaced_and_deleted() {
        let dir = std::env::temp_dir().join(format!("bc-mix-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let m = to_ipc(&MixSettings::default());
        save_preset(A, &dir, " Room ", &m).unwrap();
        let mut m2 = m.clone();
        m2.delay_ms[0] = 3.0;
        save_preset(A, &dir, "Room", &m2).unwrap();
        let st = state(A, &dir, Some(6), Some(false), None);
        let user: Vec<_> = st.presets.iter().filter(|p| !p.builtin).collect();
        assert_eq!(user.len(), 1);
        assert_eq!(user[0].mix.delay_ms[0], 3.0);
        assert_eq!(st.present, [true, true, true, true, true, true, false, false]);
        assert!(!st.rear);
        // 2.1: four outputs, the rear two the subwoofer's, fed as the fronts
        // but for the LFE, which is the subwoofer's alone
        let st = state(A, &dir, Some(6), Some(false), Some(AuracastSub::Stereo));
        assert!(st.rear && st.sub == AuracastSub::Stereo);
        for (sub, front) in [(2, 0), (3, 1)] {
            let (sub, front) = (&st.effective[sub], &st.effective[front]);
            assert_eq!((&sub[..3], &sub[4..]), (&front[..3], &front[4..]));
            assert_eq!((sub[3], front[3]), (1.0, 0.0));
        }
        // 3.1: the centre on the third output
        let st = state(A, &dir, Some(6), Some(false), Some(AuracastSub::Center));
        assert_eq!((st.effective[2][2], st.effective[0][2]), (1.0, 0.0));
        delete_preset(A, &dir, "Room").unwrap();
        assert!(state(A, &dir, None, None, None).presets.iter().all(|p| p.builtin));
        // Not under a built-in preset's name
        assert!(save_preset(A, &dir, "standard", &m).is_err());
        assert!(state(A, &dir, None, None, None).presets.iter().all(|p| p.builtin));
        // A file that cannot be read is kept aside, not written over
        save_preset(A, &dir, "Room", &m).unwrap();
        let path = crate::paths::auracast_mix_file(&dir);
        let saved = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, &saved[..saved.len() / 2]).unwrap();
        save_preset(A, &dir, "Other", &m).unwrap();
        let aside = std::fs::read_to_string(path.with_extension("json.bad")).unwrap();
        assert_eq!(aside, saved[..saved.len() / 2]);
        let names: Vec<_> =
            state(A, &dir, None, None, None).presets.into_iter().filter(|p| !p.builtin).map(|p| p.name).collect();
        assert_eq!(names, ["Other"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn builtin_presets_route_as_named() {
        let p = builtin_presets(A);
        let get = |n: &str| settings(&p.iter().find(|p| p.name == n).unwrap().mix);
        let front = get("front_only");
        assert!(!front.auto && front.matrix[2].iter().all(|g| *g == 0.0));
        let rear = get("rear_only");
        assert!(rear.matrix[0].iter().all(|g| *g == 0.0) && rear.matrix[2][4] > 0.0);
        let swapped = get("swapped");
        assert_eq!((swapped.matrix[0][1], swapped.matrix[1][0]), (1.0, 1.0));
        let tws = builtin_presets(Target::Tws);
        let get = |n: &str| settings(&tws.iter().find(|p| p.name == n).unwrap().mix);
        let swapped = get("swapped");
        assert_eq!((swapped.matrix[0][1], swapped.matrix[1][0], swapped.matrix[0][0]), (1.0, 1.0, 0.0));
        let mono = get("mono");
        assert_eq!(mono.matrix[0], mono.matrix[1]);
        assert_eq!(mono.matrix[0][..2], [0.5, 0.5]);
    }

    #[test]
    fn the_tws_mixer_is_its_own_and_off_until_turned_on() {
        let dir = std::env::temp_dir().join(format!("bc-tws-mix-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let st = state(Target::Tws, &dir, Some(2), None, None);
        assert!(!st.enabled && !st.rear);
        assert!(st.presets.iter().any(|p| p.name == "mono"));
        assert!(tws_for_stream(&dir).is_none());
        set_tws_enabled(&dir, true).unwrap();
        assert!(state(Target::Tws, &dir, None, None, None).enabled);
        assert!(tws_for_stream(&dir).is_some());
        save_preset(Target::Tws, &dir, "Mine", &to_ipc(&MixSettings::default())).unwrap();
        // In its own file: the Auracast mixer keeps its own
        assert!(crate::paths::tws_mix_file(&dir).exists());
        assert!(!crate::paths::auracast_mix_file(&dir).exists());
        assert!(state(A, &dir, None, None, None).enabled);
        assert!(state(A, &dir, None, None, None).presets.iter().all(|p| p.builtin));
        set_tws_enabled(&dir, false).unwrap();
        assert!(tws_for_stream(&dir).is_none());
        assert_eq!(state(Target::Tws, &dir, None, None, None).presets.iter().filter(|p| !p.builtin).count(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }
}

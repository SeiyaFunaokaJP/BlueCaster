//! The Auracast broadcast's volume (developer mode): its receivers have no
//! volume of ours to set, so the audio is made quieter before encoding.
//!
//! One level for the broadcast (0..127, as AVRCP's absolute volume), shown
//! as the stream's volume: the volume slider and stream.set_volume /
//! volume_step move it while a broadcast runs. The encode thread reads the
//! gain of that level from a [`GainHandle`] shared with it.
//!
//! "Follow Windows" ([`set_follow`]): the level and the Windows volume of
//! the device the audio comes from act as one ([`Follow`]), as the
//! headphones' volume does under volume sync (volume_sync.rs): the volume
//! keys move the broadcast, and the slider here moves Windows'. Loopback
//! capture is taken before that volume, so the broadcast would not follow
//! it otherwise. A muted device silences the broadcast, unless the session
//! muted it itself (mute_output.rs), where Windows' mute key does.
//! The Windows volume is not given back afterwards: it was the user's
//! volume all along. Not for the test tone or a recording device (Windows'
//! recording level is in that audio already).

use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use pipeline::gain::GainHandle;

use crate::stream::{SourceRequest, VolumeSync};
use crate::volume_sync::{Endpoint, EndpointChoice, EndpointProvider, WindowsEndpoints, scalar_to_level};

/// Full volume
pub const MAX_LEVEL: u8 = 127;

static LEVEL: AtomicU8 = AtomicU8::new(MAX_LEVEL);
static FOLLOW: AtomicBool = AtomicBool::new(false);
static LIVE: AtomicBool = AtomicBool::new(false);
static GAIN: LazyLock<GainHandle> = LazyLock::new(GainHandle::default);

/// The amplitude of `level`: its share squared, about the curve of the
/// Windows slider (50 % is -12 dB, 10 % -40 dB), 0 silent
pub fn gain_of(level: u8) -> f32 {
    let share = f32::from(level.min(MAX_LEVEL)) / f32::from(MAX_LEVEL);
    share * share
}

/// The gain the broadcast's encode thread reads
pub fn gain() -> GainHandle {
    GAIN.clone()
}

/// The broadcast's level (0..=127), kept between broadcasts
pub fn level() -> u8 {
    LEVEL.load(Ordering::Relaxed)
}

/// Sets the level (the running broadcast hears it at its next tick)
pub fn set_level(level: u8) {
    let level = level.min(MAX_LEVEL);
    if LEVEL.swap(level, Ordering::Relaxed) != level {
        tracing::debug!("broadcast volume: level {level} ({}%)", crate::volume_sync::percent(level));
    }
}

/// Whether the level follows the Windows volume of the captured device
pub fn follows() -> bool {
    FOLLOW.load(Ordering::Relaxed)
}

pub fn set_follow(on: bool) {
    if FOLLOW.swap(on, Ordering::Relaxed) != on {
        tracing::info!("broadcast volume: {} the Windows volume", if on { "follows" } else { "no longer follows" });
    }
}

/// A broadcast runs: the stream's volume is its level
pub fn live() -> bool {
    LIVE.load(Ordering::Relaxed)
}

pub(crate) fn set_live(on: bool) {
    LIVE.store(on, Ordering::Relaxed);
}

/// The gain the encode thread gets for `level`, silent while `silenced`
pub(crate) fn apply(level: u8, silenced: bool) {
    GAIN.set(if silenced { 0.0 } else { gain_of(level) });
}

/// The Windows device whose volume a broadcast of `source` follows; None:
/// the test tone and recording devices have none
pub fn endpoint_for(source: &SourceRequest) -> Option<EndpointChoice> {
    match source {
        SourceRequest::InputDevice { .. } => None,
        _ => EndpointChoice::for_stream(&VolumeSync::Auto, source),
    }
}

/// The volume of one running broadcast, on its session thread: the gain of
/// the level, the Windows volume followed while asked, and the level to
/// show as the stream's. Marks the broadcast live while it exists.
pub(crate) struct Session {
    follow: Option<(EndpointChoice, Follow)>,
    shown: Option<u8>,
}

impl Session {
    /// For a broadcast of `source` about to start: its first audio already
    /// has the level, the Windows volume and the mute a [`tick`](Session::tick)
    /// finds (`mutes_output` as there)
    pub fn new(source: &SourceRequest, mutes_output: bool) -> Session {
        set_live(true);
        let mut session = Session { follow: None, shown: None };
        session.step(source, mutes_output);
        session
    }

    /// One step of the broadcast of `source` (`mutes_output`: the session
    /// keeps the default output muted). Some: the level to show, changed.
    pub fn tick(&mut self, source: &SourceRequest, mutes_output: bool) -> Option<u8> {
        let level = self.step(source, mutes_output);
        (self.shown != Some(level)).then(|| {
            self.shown = Some(level);
            level
        })
    }

    /// Follows the Windows volume while asked and gives the encode thread
    /// its gain; the level now
    fn step(&mut self, source: &SourceRequest, mutes_output: bool) -> u8 {
        let choice = if follows() { endpoint_for(source) } else { None };
        if self.follow.as_ref().map(|(c, _)| c) != choice.as_ref() {
            self.follow = choice.map(|c| (c.clone(), Follow::new(c)));
        }
        let mut muted = false;
        if let Some((_, f)) = self.follow.as_mut() {
            let found = f.tick(level());
            if let Some(l) = found.level {
                set_level(l);
            }
            // The session's own mute of the default output is no sign
            muted = found.muted && !(mutes_output && f.on_default_device());
        }
        // Windows' mute key is the sign while the session keeps it muted
        let silenced = muted || (mutes_output && crate::mute_output::key_silenced());
        let level = level();
        apply(level, silenced);
        level
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        set_live(false);
    }
}

/// What one [`Follow::tick`] found
#[derive(Debug, Default, PartialEq)]
pub struct Followed {
    /// The broadcast takes this level (Windows' volume changed)
    pub level: Option<u8>,
    /// The device is muted
    pub muted: bool,
}

/// Keeps the broadcast's level and one device's Windows volume equal; lives
/// on the broadcast's session thread, ticked with it. When it starts, the
/// broadcast takes the Windows volume.
pub struct Follow<P: EndpointProvider = WindowsEndpoints> {
    provider: P,
    choice: EndpointChoice,
    endpoint: Option<P::Endpoint>,
    /// Levels last seen in step; None: not yet
    last_windows: Option<u8>,
    last_level: Option<u8>,
    /// Device that could not be opened (logged once)
    failed: Option<String>,
}

impl Follow<WindowsEndpoints> {
    pub fn new(choice: EndpointChoice) -> Self {
        Follow::with_provider(WindowsEndpoints, choice)
    }
}

impl<P: EndpointProvider> Follow<P> {
    pub fn with_provider(provider: P, choice: EndpointChoice) -> Self {
        tracing::debug!("broadcast volume: following {choice:?}");
        Follow { provider, choice, endpoint: None, last_windows: None, last_level: None, failed: None }
    }

    /// One step with the broadcast's `level` now
    pub fn tick(&mut self, level: u8) -> Followed {
        if !self.attach() {
            return Followed::default();
        }
        let Some(endpoint) = self.endpoint.as_ref() else { return Followed::default() };
        let muted = endpoint.muted().unwrap_or(false);
        let Some(windows) = endpoint.volume().map(scalar_to_level) else {
            // Gone (unplugged): opened again when it is back
            self.endpoint = None;
            return Followed { level: None, muted };
        };
        let changed = |last: Option<u8>, now: u8| last != Some(now);
        if changed(self.last_windows, windows) {
            // Started, or the volume keys, the flyout or the mixer
            if self.last_windows.is_none() {
                tracing::info!("broadcast volume: takes the Windows volume, {}%", crate::volume_sync::percent(windows));
            } else {
                tracing::debug!("broadcast volume: Windows -> broadcast {windows}");
            }
            self.last_windows = Some(windows);
            self.last_level = Some(windows);
            return Followed { level: (windows != level).then_some(windows), muted };
        }
        if changed(self.last_level, level) {
            // The slider here (or stream.set_volume)
            let ok = endpoint.set_volume(crate::volume_sync::level_to_scalar(level));
            let taken = endpoint.volume().map(scalar_to_level);
            tracing::debug!("broadcast volume: broadcast -> Windows {level}: ok {ok}, took {taken:?}");
            self.last_windows = Some(taken.unwrap_or(level));
            self.last_level = Some(level);
        }
        Followed { level: None, muted }
    }

    /// The device followed is the default output device: the one a session
    /// that mutes the output keeps muted
    pub fn on_default_device(&self) -> bool {
        match &self.choice {
            EndpointChoice::Default => true,
            EndpointChoice::Device(id) => self.provider.default_device().as_deref() == Some(id.as_str()),
        }
    }

    /// Opens the device; with the default one, again when it changed
    fn attach(&mut self) -> bool {
        let id = match &self.choice {
            EndpointChoice::Device(id) => id.clone(),
            EndpointChoice::Default => match self.provider.default_device() {
                Some(id) => id,
                None => return false,
            },
        };
        if self.endpoint.as_ref().is_some_and(|e| e.device_id() == id) {
            return true;
        }
        self.endpoint = None;
        self.last_windows = None;
        self.last_level = None;
        match self.provider.open(&id) {
            Ok(e) => {
                self.endpoint = Some(e);
                self.failed = None;
                true
            }
            Err(e) => {
                if self.failed.as_deref() != Some(id.as_str()) {
                    tracing::warn!("broadcast volume: cannot open the output device's volume: {e}");
                    self.failed = Some(id);
                }
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use super::*;
    use crate::volume_sync::level_to_scalar;

    #[derive(Default)]
    struct Devices {
        volume: RefCell<HashMap<String, f32>>,
        muted: RefCell<HashMap<String, bool>>,
        default: RefCell<Option<String>>,
    }

    struct FakeEndpoint(String, Rc<Devices>);

    impl Endpoint for FakeEndpoint {
        fn device_id(&self) -> &str {
            &self.0
        }
        fn volume(&self) -> Option<f32> {
            self.1.volume.borrow().get(&self.0).copied()
        }
        fn set_volume(&self, scalar: f32) -> bool {
            self.1.volume.borrow_mut().insert(self.0.clone(), scalar);
            true
        }
        fn muted(&self) -> Option<bool> {
            self.1.muted.borrow().get(&self.0).copied()
        }
    }

    struct FakeProvider(Rc<Devices>);

    impl EndpointProvider for FakeProvider {
        type Endpoint = FakeEndpoint;
        fn default_device(&self) -> Option<String> {
            self.0.default.borrow().clone()
        }
        fn open(&self, id: &str) -> Result<FakeEndpoint, String> {
            if !self.0.volume.borrow().contains_key(id) {
                return Err(format!("no device {id}"));
            }
            Ok(FakeEndpoint(id.into(), self.0.clone()))
        }
    }

    fn devices(ids: &[(&str, u8)]) -> Rc<Devices> {
        let d = Rc::new(Devices::default());
        for (id, level) in ids {
            d.volume.borrow_mut().insert(id.to_string(), level_to_scalar(*level));
        }
        *d.default.borrow_mut() = ids.first().map(|(id, _)| id.to_string());
        d
    }

    fn windows(d: &Devices, id: &str) -> u8 {
        scalar_to_level(d.volume.borrow()[id])
    }

    #[test]
    fn the_gain_follows_the_level() {
        assert_eq!(gain_of(MAX_LEVEL), 1.0);
        assert_eq!(gain_of(0), 0.0);
        assert_eq!(gain_of(200), 1.0);
        let half = 20.0 * gain_of(64).log10();
        assert!((-12.5..-11.5).contains(&half), "{half} dB");
    }

    #[test]
    fn the_broadcast_takes_the_windows_volume_first() {
        let d = devices(&[("{spk}", 40)]);
        let mut f = Follow::with_provider(FakeProvider(d.clone()), EndpointChoice::Default);
        assert_eq!(f.tick(127), Followed { level: Some(40), muted: false });
        // In step now: nothing to do
        assert_eq!(f.tick(40), Followed::default());
        assert_eq!(windows(&d, "{spk}"), 40);
    }

    #[test]
    fn each_side_moves_the_other() {
        let d = devices(&[("{spk}", 100)]);
        let mut f = Follow::with_provider(FakeProvider(d.clone()), EndpointChoice::Device("{spk}".into()));
        assert_eq!(f.tick(100).level, None);
        // The volume keys
        d.volume.borrow_mut().insert("{spk}".into(), level_to_scalar(60));
        assert_eq!(f.tick(100).level, Some(60));
        assert_eq!(f.tick(60).level, None);
        // The slider of the broadcast
        assert_eq!(f.tick(90).level, None);
        assert_eq!(windows(&d, "{spk}"), 90);
        assert_eq!(f.tick(90), Followed::default());
    }

    #[test]
    fn a_muted_device_is_reported() {
        let d = devices(&[("{spk}", 100)]);
        d.muted.borrow_mut().insert("{spk}".into(), true);
        let mut f = Follow::with_provider(FakeProvider(d.clone()), EndpointChoice::Default);
        assert!(f.tick(100).muted);
        d.muted.borrow_mut().insert("{spk}".into(), false);
        assert!(!f.tick(100).muted);
    }

    #[test]
    fn a_new_default_device_is_taken_up() {
        let d = devices(&[("{spk}", 100), ("{hdmi}", 30)]);
        let mut f = Follow::with_provider(FakeProvider(d.clone()), EndpointChoice::Default);
        assert_eq!(f.tick(127).level, Some(100));
        *d.default.borrow_mut() = Some("{hdmi}".into());
        assert_eq!(f.tick(100).level, Some(30));
    }

    #[test]
    fn a_device_that_cannot_be_opened_changes_nothing() {
        let d = devices(&[("{spk}", 100)]);
        let mut f = Follow::with_provider(FakeProvider(d), EndpointChoice::Device("{gone}".into()));
        assert_eq!(f.tick(50), Followed::default());
    }

    #[test]
    fn a_device_plugged_in_again_is_followed_again() {
        let d = devices(&[("{dac}", 100)]);
        let mut f = Follow::with_provider(FakeProvider(d.clone()), EndpointChoice::Device("{dac}".into()));
        assert_eq!(f.tick(100).level, None);
        // Unplugged: its volume cannot be read, nor the device opened
        d.volume.borrow_mut().remove("{dac}");
        assert_eq!(f.tick(100), Followed::default());
        assert_eq!(f.tick(100), Followed::default());
        // Back: the broadcast takes its volume, and the slider moves it again
        d.volume.borrow_mut().insert("{dac}".into(), level_to_scalar(70));
        assert_eq!(f.tick(100).level, Some(70));
        assert_eq!(f.tick(50).level, None);
        assert_eq!(windows(&d, "{dac}"), 50);
    }

    #[test]
    fn only_a_chosen_device_that_is_the_default_one_is_the_sessions() {
        let d = devices(&[("{spk}", 100), ("{hdmi}", 30)]);
        let follow = |choice| Follow::with_provider(FakeProvider(d.clone()), choice);
        assert!(follow(EndpointChoice::Default).on_default_device());
        assert!(follow(EndpointChoice::Device("{spk}".into())).on_default_device());
        assert!(!follow(EndpointChoice::Device("{hdmi}".into())).on_default_device());
    }

    #[test]
    fn recording_devices_and_the_tone_follow_nothing() {
        let input = SourceRequest::InputDevice { id: "{mic}".into(), name: "Microphone".into() };
        assert_eq!(endpoint_for(&input), None);
        assert_eq!(endpoint_for(&SourceRequest::TestTone), None);
        assert_eq!(endpoint_for(&SourceRequest::SystemLoopback), Some(EndpointChoice::Default));
        let output = SourceRequest::OutputDevice { id: "{spk}".into(), name: "Speakers".into(), auto_switch: false };
        assert_eq!(endpoint_for(&output), Some(EndpointChoice::Device("{spk}".into())));
    }
}

//! The headset microphone's cable as Windows' default input while its
//! device is connected (issue #86): apps that record from the default
//! microphone then record the headphones' microphone, and Auto opens it for
//! them. The counterpart of "Set as Windows default output while connected"
//! (stream.rs DefaultDeviceGuard), for the recording side ("CABLE Output")
//! of the output the microphone plays into ("CABLE Input").
//!
//! The core's state follows each change of the stream and the profiles with
//! a [`Want`]: switched while connected, kept while connecting again (a
//! profile started again with new settings, a lost link), released once
//! the device is disconnected. A release waits [`RELEASE_GRACE`], so a
//! profile started again does not flip the default input for a moment.
//! The device before is restored only while ours is still the default: one
//! the user chose meanwhile stays.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// How long a release waits for the device to connect again
pub const RELEASE_GRACE: Duration = Duration::from_secs(3);

/// Windows' capture endpoints, behind a trait so the logic is tested with
/// fakes
pub trait InputEndpoints: Send + Sync {
    /// The default input device's ID
    fn default_input(&self) -> Option<String>;
    /// The recording side of the render endpoint `render_id`
    fn capture_side(&self, render_id: &str) -> Option<String>;
    /// Makes `id` the default input; false when it failed
    fn set_default_input(&self, id: &str) -> bool;
}

/// Windows' own endpoints
pub struct WindowsInputs;

impl InputEndpoints for WindowsInputs {
    fn default_input(&self) -> Option<String> {
        audio_capture::default_capture_device()
    }

    fn capture_side(&self, render_id: &str) -> Option<String> {
        audio_capture::paired_capture_device(render_id)
    }

    fn set_default_input(&self, id: &str) -> bool {
        match audio_capture::set_default_capture_device(id) {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!("default input: could not switch: {e}");
                false
            }
        }
    }
}

/// What the default input should do now
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Want {
    /// The recording side of this render endpoint (the microphone's output)
    /// is the default input
    Switch(String),
    /// As it is (connecting, reconnecting)
    Keep,
    /// The device before is the default input again (after the grace)
    Release,
}

#[derive(Debug, Default)]
struct State {
    /// The microphone output switched to (also when it has no recording
    /// side, so that it is not looked up again)
    render: Option<String>,
    /// Its recording side made the default
    capture: Option<String>,
    /// The default input before
    restore: Option<String>,
    /// Counts the changes: a release waiting for its grace is dropped when
    /// anything else came after it
    generation: u64,
}

/// Switches and restores the default input ([`Want`])
pub struct DefaultInput {
    state: Arc<Mutex<State>>,
    endpoints: Arc<dyn InputEndpoints>,
    grace: Duration,
}

fn lock(m: &Mutex<State>) -> MutexGuard<'_, State> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl DefaultInput {
    pub fn new(endpoints: Arc<dyn InputEndpoints>, grace: Duration) -> DefaultInput {
        DefaultInput { state: Arc::new(Mutex::new(State::default())), endpoints, grace }
    }

    /// Windows' endpoints, releasing after [`RELEASE_GRACE`]
    pub fn windows() -> DefaultInput {
        DefaultInput::new(Arc::new(WindowsInputs), RELEASE_GRACE)
    }

    /// Follows what the default input should do now
    pub fn follow(&self, want: Want) {
        let mut s = lock(&self.state);
        match want {
            Want::Keep => s.generation += 1,
            Want::Switch(render) => {
                s.generation += 1;
                if s.render.as_deref() != Some(render.as_str()) {
                    switch(&mut s, self.endpoints.as_ref(), render);
                }
            }
            Want::Release => {
                if s.render.is_none() {
                    return;
                }
                s.generation += 1;
                if self.grace.is_zero() {
                    release(&mut s, self.endpoints.as_ref());
                    return;
                }
                let generation = s.generation;
                drop(s);
                let state = self.state.clone();
                let endpoints = self.endpoints.clone();
                let grace = self.grace;
                let spawned = std::thread::Builder::new().name("default-input".into()).spawn(move || {
                    std::thread::sleep(grace);
                    let mut s = lock(&state);
                    if s.generation == generation {
                        release(&mut s, endpoints.as_ref());
                    }
                });
                if spawned.is_err() {
                    self.release_now();
                }
            }
        }
    }

    /// Restores the device before at once (the core exits)
    pub fn release_now(&self) {
        let mut s = lock(&self.state);
        s.generation += 1;
        release(&mut s, self.endpoints.as_ref());
    }
}

fn switch(s: &mut State, endpoints: &dyn InputEndpoints, render: String) {
    let Some(capture) = endpoints.capture_side(&render) else {
        tracing::info!("default input: the microphone output has no recording side, left as it is");
        // One switched to before stays switched until the release
        s.render = Some(render);
        return;
    };
    let before = endpoints.default_input();
    // Another cable chosen while switched: the device from the start is
    // restored
    if s.restore.is_none() && s.capture.is_none() {
        s.restore = before.clone().filter(|b| *b != capture);
    }
    if before.as_deref() != Some(capture.as_str()) {
        tracing::info!("default input: the microphone's recording side while connected");
        endpoints.set_default_input(&capture);
    }
    s.render = Some(render);
    s.capture = Some(capture);
}

fn release(s: &mut State, endpoints: &dyn InputEndpoints) {
    s.render = None;
    let ours = s.capture.take();
    let restore = s.restore.take();
    let (Some(ours), Some(restore)) = (ours, restore) else { return };
    if endpoints.default_input().as_deref() == Some(ours.as_str()) {
        tracing::info!("default input: restoring the device before");
        endpoints.set_default_input(&restore);
    } else {
        tracing::info!("default input: changed meanwhile, left as it is");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Fake {
        default: Mutex<Option<String>>,
        sets: Mutex<Vec<String>>,
    }

    impl InputEndpoints for Fake {
        fn default_input(&self) -> Option<String> {
            self.default.lock().unwrap().clone()
        }
        fn capture_side(&self, render_id: &str) -> Option<String> {
            match render_id {
                "{cable-in}" => Some("{cable-out}".into()),
                "{hifi-in}" => Some("{hifi-out}".into()),
                _ => None,
            }
        }
        fn set_default_input(&self, id: &str) -> bool {
            *self.default.lock().unwrap() = Some(id.into());
            self.sets.lock().unwrap().push(id.into());
            true
        }
    }

    fn fake(default: &str) -> Arc<Fake> {
        let f = Fake::default();
        *f.default.lock().unwrap() = Some(default.into());
        Arc::new(f)
    }

    fn now(f: &Arc<Fake>) -> String {
        f.default_input().unwrap_or_default()
    }

    #[test]
    fn switched_while_connected_and_restored_after() {
        let f = fake("{pc-mic}");
        let d = DefaultInput::new(f.clone(), Duration::ZERO);
        d.follow(Want::Switch("{cable-in}".into()));
        assert_eq!(now(&f), "{cable-out}");
        // The same again (every stream event): no second switch
        d.follow(Want::Switch("{cable-in}".into()));
        d.follow(Want::Keep);
        assert_eq!(f.sets.lock().unwrap().len(), 1);
        d.follow(Want::Release);
        assert_eq!(now(&f), "{pc-mic}");
        // Nothing switched: nothing to release
        d.follow(Want::Release);
        assert_eq!(f.sets.lock().unwrap().len(), 2);
    }

    #[test]
    fn another_cable_restores_the_first_device() {
        let f = fake("{pc-mic}");
        let d = DefaultInput::new(f.clone(), Duration::ZERO);
        d.follow(Want::Switch("{cable-in}".into()));
        d.follow(Want::Switch("{hifi-in}".into()));
        assert_eq!(now(&f), "{hifi-out}");
        d.follow(Want::Release);
        assert_eq!(now(&f), "{pc-mic}");
    }

    #[test]
    fn a_default_chosen_meanwhile_stays() {
        let f = fake("{pc-mic}");
        let d = DefaultInput::new(f.clone(), Duration::ZERO);
        d.follow(Want::Switch("{cable-in}".into()));
        *f.default.lock().unwrap() = Some("{webcam}".into());
        d.follow(Want::Release);
        assert_eq!(now(&f), "{webcam}");
    }

    #[test]
    fn already_the_default_is_left_so() {
        let f = fake("{cable-out}");
        let d = DefaultInput::new(f.clone(), Duration::ZERO);
        d.follow(Want::Switch("{cable-in}".into()));
        d.follow(Want::Release);
        assert_eq!(now(&f), "{cable-out}");
        assert!(f.sets.lock().unwrap().is_empty());
    }

    #[test]
    fn an_output_without_a_recording_side_changes_nothing() {
        let f = fake("{pc-mic}");
        let d = DefaultInput::new(f.clone(), Duration::ZERO);
        d.follow(Want::Switch("{speakers}".into()));
        d.follow(Want::Release);
        assert_eq!(now(&f), "{pc-mic}");
        assert!(f.sets.lock().unwrap().is_empty());
    }

    #[test]
    fn a_release_waits_and_a_reconnect_cancels_it() {
        let f = fake("{pc-mic}");
        let d = DefaultInput::new(f.clone(), Duration::from_millis(100));
        d.follow(Want::Switch("{cable-in}".into()));
        // Started again with new settings: Idle, then connecting and streaming
        d.follow(Want::Release);
        d.follow(Want::Keep);
        d.follow(Want::Switch("{cable-in}".into()));
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(now(&f), "{cable-out}");
        assert_eq!(f.sets.lock().unwrap().len(), 1);
        // Disconnected for good: restored after the grace
        d.follow(Want::Release);
        assert_eq!(now(&f), "{cable-out}");
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(now(&f), "{pc-mic}");
    }

    #[test]
    fn released_at_once_when_the_core_exits() {
        let f = fake("{pc-mic}");
        let d = DefaultInput::new(f.clone(), Duration::from_secs(60));
        d.follow(Want::Switch("{cable-in}".into()));
        d.follow(Want::Release);
        d.release_now();
        assert_eq!(now(&f), "{pc-mic}");
    }
}

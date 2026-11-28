//! Which channel each output of an LE Audio stream to a device plays (the
//! device's settings, followed while the stream runs): left and right
//! swapped, and what an earbud left alone plays while the other one of its
//! set is gone. One [`ChannelsHandle`] is shared with the encode thread,
//! which applies it to every LC3 frame from the next one on.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use le_audio::plan::ChannelSource;

/// What an earbud plays while the other one of its set is gone
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AloneChannel {
    /// Its own side, as the stream was planned: what the earbud does with
    /// it is the earbud's business
    Auto,
    /// The left channel of the sound
    Left,
    /// The right channel of the sound
    Right,
    /// Both channels mixed (the program's behaviour before the setting)
    #[default]
    Mono,
}

impl AloneChannel {
    /// "auto" | "left" | "right" | "mono" (profiles.json, IPC); anything
    /// else: Mono
    pub fn from_key(key: &str) -> AloneChannel {
        match key {
            "auto" => AloneChannel::Auto,
            "left" => AloneChannel::Left,
            "right" => AloneChannel::Right,
            _ => AloneChannel::Mono,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            AloneChannel::Auto => "auto",
            AloneChannel::Left => "left",
            AloneChannel::Right => "right",
            AloneChannel::Mono => "mono",
        }
    }

    fn bits(self) -> u8 {
        match self {
            AloneChannel::Mono => 0,
            AloneChannel::Auto => 1,
            AloneChannel::Left => 2,
            AloneChannel::Right => 3,
        }
    }

    fn from_bits(b: u8) -> AloneChannel {
        match b & 3 {
            1 => AloneChannel::Auto,
            2 => AloneChannel::Left,
            3 => AloneChannel::Right,
            _ => AloneChannel::Mono,
        }
    }
}

/// The device's channel settings
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Channels {
    /// Left and right swapped (also rear left and right)
    pub swap: bool,
    pub alone: AloneChannel,
}

impl Channels {
    /// Where an output planned to play `planned` takes its audio from;
    /// `alone`: it is the only earbud of its set still playing
    pub fn source(self, planned: ChannelSource, alone: bool) -> ChannelSource {
        if alone {
            match self.alone {
                AloneChannel::Left => return ChannelSource::Left,
                AloneChannel::Right => return ChannelSource::Right,
                AloneChannel::Mono => return ChannelSource::Mono,
                AloneChannel::Auto => {}
            }
        }
        if !self.swap {
            return planned;
        }
        match planned {
            ChannelSource::Left => ChannelSource::Right,
            ChannelSource::Right => ChannelSource::Left,
            ChannelSource::RearLeft => ChannelSource::RearRight,
            ChannelSource::RearRight => ChannelSource::RearLeft,
            ChannelSource::SubLeft => ChannelSource::SubRight,
            ChannelSource::SubRight => ChannelSource::SubLeft,
            other @ (ChannelSource::Mono | ChannelSource::SubMono | ChannelSource::Center) => other,
        }
    }

    fn bits(self) -> u8 {
        self.alone.bits() | if self.swap { 4 } else { 0 }
    }

    fn from_bits(b: u8) -> Channels {
        Channels { swap: b & 4 != 0, alone: AloneChannel::from_bits(b) }
    }
}

/// The settings, shared with the encode thread; [`Channels::default`] at
/// first (nothing swapped, an earbud alone gets the mono mix)
#[derive(Clone, Debug, Default)]
pub struct ChannelsHandle(Arc<AtomicU8>);

impl ChannelsHandle {
    pub fn set(&self, c: Channels) {
        let before = Channels::from_bits(self.0.swap(c.bits(), Ordering::Relaxed));
        if before != c {
            tracing::info!(
                "LE Audio: channels {}, an earbud alone plays {}",
                if c.swap { "swapped" } else { "as they are" },
                c.alone.key()
            );
        }
    }

    pub fn get(&self) -> Channels {
        Channels::from_bits(self.0.load(Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ChannelSource::*;

    #[test]
    fn swapping_exchanges_the_sides_and_leaves_mono() {
        let c = Channels { swap: true, alone: AloneChannel::Auto };
        assert_eq!(c.source(Left, false), Right);
        assert_eq!(c.source(Right, false), Left);
        assert_eq!(c.source(RearLeft, false), RearRight);
        assert_eq!(c.source(Mono, false), Mono);
        assert_eq!(Channels::default().source(Left, false), Left);
    }

    #[test]
    fn an_earbud_alone_plays_what_was_chosen() {
        let with = |alone| Channels { swap: true, alone };
        // The sound's own channel, swapped or not
        assert_eq!(with(AloneChannel::Left).source(Right, true), Left);
        assert_eq!(with(AloneChannel::Right).source(Right, true), Right);
        assert_eq!(with(AloneChannel::Mono).source(Left, true), Mono);
        // Auto: its own side, as planned (swapped too)
        assert_eq!(with(AloneChannel::Auto).source(Left, true), Right);
        assert_eq!(Channels { swap: false, alone: AloneChannel::Auto }.source(Left, true), Left);
        // Not alone: the setting does not count
        assert_eq!(with(AloneChannel::Mono).source(Left, false), Right);
    }

    #[test]
    fn the_handle_keeps_both_and_the_keys_go_both_ways() {
        let h = ChannelsHandle::default();
        assert_eq!(h.get(), Channels { swap: false, alone: AloneChannel::Mono });
        for alone in [AloneChannel::Auto, AloneChannel::Left, AloneChannel::Right, AloneChannel::Mono] {
            for swap in [false, true] {
                h.set(Channels { swap, alone });
                assert_eq!(h.get(), Channels { swap, alone });
            }
            assert_eq!(AloneChannel::from_key(alone.key()), alone);
        }
        assert_eq!(AloneChannel::from_key("anything"), AloneChannel::Mono);
    }
}

//! The LE Audio channels of the stream to a device: each device profile's
//! own (`le_swap_channels`, `le_alone_channel` in profiles.json). One stream
//! runs at a time, so the encode threads share one [`ChannelsHandle`]: the
//! profile that starts streaming sets it, and a change saved while it
//! streams takes effect from the next frame, without reconnecting.

use std::sync::LazyLock;

use pipeline::channels::{AloneChannel, Channels, ChannelsHandle};

static HANDLE: LazyLock<ChannelsHandle> = LazyLock::new(ChannelsHandle::default);

/// The channels the encode threads read
pub fn handle() -> ChannelsHandle {
    HANDLE.clone()
}

/// `le_alone_channel` as saved: a known key, mono (the default) as none
pub fn saved_key(key: &str) -> String {
    match AloneChannel::from_key(key) {
        AloneChannel::Mono => String::new(),
        a => a.key().to_string(),
    }
}

/// The stream's channels from now on ([`Channels::default`] for a stream
/// without a profile)
pub fn apply(channels: Channels) {
    HANDLE.set(channels);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_as_a_known_key_and_mono_as_none() {
        assert_eq!(saved_key("left"), "left");
        assert_eq!(saved_key("auto"), "auto");
        assert_eq!(saved_key("mono"), "");
        assert_eq!(saved_key("rear"), "");
    }
}

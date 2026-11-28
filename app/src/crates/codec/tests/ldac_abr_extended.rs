//! Developer mode's extended ABR for LDAC (#600): one quality step up per
//! second of a drained queue. Its own test binary, as the override is
//! global.

use codec::dev::{DevOverrides, set_overrides};
use codec::ldac::LdacEncoder;
use codec::{Encoder, EncoderConfig, Pcm, Quality};

fn config() -> EncoderConfig {
    EncoderConfig {
        sample_rate: 48000,
        channels: 2,
        quality: Quality::High,
        mtu: 667,
        bits_per_sample: 16,
        abr: true,
        peer: Default::default(),
    }
}

/// ABR steps (100 ms each) from MQ back to HQ after a deep queue
fn steps_back_to_hq(enc: &mut LdacEncoder) -> usize {
    let pcm = vec![0i16; enc.pcm_frames_per_encode() * 2];
    let mut out = vec![0u8; enc.max_output_bytes()];
    let mut encode = |enc: &mut LdacEncoder| {
        for _ in 0..8 {
            enc.encode(Pcm::I16(&pcm), &mut out).unwrap();
        }
    };
    for _ in 0..10 {
        enc.abr_adjust(20);
        encode(enc);
    }
    assert_eq!(enc.bitrate_kbps(), 330);
    let mut steps = 0;
    while enc.bitrate_kbps() != 990 {
        enc.abr_adjust(0);
        encode(enc);
        steps += 1;
        assert!(steps < 3000, "still at {} kbps after {steps} ABR steps", enc.bitrate_kbps());
    }
    steps
}

#[test]
fn extended_abr_climbs_one_step_per_second() {
    let libldac = steps_back_to_hq(&mut LdacEncoder::new(&config()).unwrap());

    set_overrides(DevOverrides { abr_extended: true, ..Default::default() });
    let mut enc = LdacEncoder::new(&config()).unwrap();
    let fast = steps_back_to_hq(&mut enc);
    set_overrides(DevOverrides::default());

    // Three steps of 1 s, then HQ, where the queue backed up: libldac's
    // own 2 s step or the stall wait (10 s), whichever comes first. libldac
    // alone takes about 30 s.
    assert!((30..=130).contains(&fast), "{fast} steps (libldac alone: {libldac})");
    assert!(fast * 2 < libldac, "{fast} steps, libldac alone {libldac}");

    // It still steps down
    for _ in 0..10 {
        enc.abr_adjust(20);
        let pcm = vec![0i16; enc.pcm_frames_per_encode() * 2];
        let mut out = vec![0u8; enc.max_output_bytes()];
        for _ in 0..8 {
            enc.encode(Pcm::I16(&pcm), &mut out).unwrap();
        }
    }
    assert_eq!(enc.bitrate_kbps(), 330);
}

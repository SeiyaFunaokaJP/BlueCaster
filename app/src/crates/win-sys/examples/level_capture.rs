//! Development helper for headset mode's Auto: records from a capture
//! endpoint for a while, as an application using the microphone would, and
//! prints the level of every 50 ms from the start (dBFS peak; "zero" for
//! digital silence), to see how soon the microphone's sound arrives.
//!
//!     cargo run -p win-sys --example level_capture -- "CABLE Output (VB-Audio Virtual" 5
//!
//! The first argument is part of the endpoint's friendly name, the second
//! the seconds to record (default 5).

use std::time::{Duration, Instant};

use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    AUDCLNT_SHAREMODE_SHARED, DEVICE_STATE_ACTIVE, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator,
    MMDeviceEnumerator, eCapture,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, STGM_READ};

fn main() -> windows::core::Result<()> {
    let mut args = std::env::args().skip(1);
    let wanted = args.next().unwrap_or_else(|| "CABLE Output".into()).to_lowercase();
    let secs: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(5);
    // SAFETY: plain COM initialisation on this thread.
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
    // SAFETY: documented COM calls on live objects.
    unsafe {
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let devices = enumerator.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)?;
        let mut found = None;
        for i in 0..devices.GetCount()? {
            let device = devices.Item(i)?;
            let store = device.OpenPropertyStore(STGM_READ)?;
            let value = store.GetValue(&PKEY_Device_FriendlyName)?;
            let name = PropVariantToStringAlloc(&value).map(|p| p.to_string().unwrap_or_default()).unwrap_or_default();
            if name.to_lowercase().contains(&wanted) {
                found = Some((device, name));
                break;
            }
        }
        let Some((device, name)) = found else {
            eprintln!("no active capture endpoint whose name contains {wanted:?}");
            std::process::exit(1);
        };
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
        let format = client.GetMixFormat()?;
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 10_000_000, 0, format, None)?;
        let capture: IAudioCaptureClient = client.GetService()?;
        // Float32 shared-mode mix format: 4 bytes a sample
        let channels = (*format).nChannels as usize;
        let rate = (*format).nSamplesPerSec as u64;
        let per_slot = rate / 20;
        client.Start()?;
        println!("recording from {name} for {secs} s, {rate} Hz, {channels} ch");
        let t0 = Instant::now();
        let until = t0 + Duration::from_secs(secs);
        let (mut frames, mut slot_frames, mut peak, mut nonzero) = (0u64, 0u64, 0f32, false);
        let mut line = String::new();
        while Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
            loop {
                let mut data = std::ptr::null_mut();
                let mut n = 0u32;
                let mut flags = 0u32;
                if capture.GetBuffer(&mut data, &mut n, &mut flags, None, None).is_err() || n == 0 {
                    break;
                }
                let silent = flags & 2 != 0; // AUDCLNT_BUFFERFLAGS_SILENT
                let samples = std::slice::from_raw_parts(data as *const f32, n as usize * channels);
                for f in samples.chunks(channels) {
                    if !silent {
                        for &x in f {
                            peak = peak.max(x.abs());
                            nonzero |= x != 0.0;
                        }
                    }
                    slot_frames += 1;
                    if slot_frames == per_slot {
                        let ms = frames * 1000 / rate;
                        let level =
                            if nonzero { format!("{:.0}", 20.0 * peak.max(1e-9).log10()) } else { "zero".into() };
                        line.push_str(&format!("{ms:>5} {level:>5} |"));
                        if line.len() > 100 {
                            println!("{line}");
                            line.clear();
                        }
                        frames += slot_frames;
                        slot_frames = 0;
                        peak = 0.0;
                        nonzero = false;
                    }
                }
                let _ = capture.ReleaseBuffer(n);
            }
        }
        println!("{line}");
        client.Stop()?;
    }
    Ok(())
}

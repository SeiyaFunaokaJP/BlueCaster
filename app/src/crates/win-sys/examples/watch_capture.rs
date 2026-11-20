//! Development helper for headset mode's Auto: prints, with timestamps, when
//! the capture side paired with a render endpoint starts and stops being
//! recorded ([`win_sys::audio_policy::capture_sessions_active`]), polled
//! every 10 ms, and how long a poll takes.
//!
//!     cargo run -p win-sys --example watch_capture -- "CABLE Input" 20
//!
//! The first argument is part of the render endpoint's friendly name, the
//! second the seconds to watch (default 20), the third "cached" to poll
//! with `CaptureSessions` (the endpoints looked up once).

use std::time::{Duration, Instant};

use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{DEVICE_STATE_ACTIVE, IMMDeviceEnumerator, MMDeviceEnumerator, eRender};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, STGM_READ};

fn main() -> windows::core::Result<()> {
    let mut args = std::env::args().skip(1);
    let wanted = args.next().unwrap_or_else(|| "CABLE Input".into()).to_lowercase();
    let secs: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(20);
    // SAFETY: plain COM initialisation on this thread.
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
    // SAFETY: documented COM calls on live objects.
    let (id, name) = unsafe {
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let devices = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
        let mut found = None;
        for i in 0..devices.GetCount()? {
            let device = devices.Item(i)?;
            let store = device.OpenPropertyStore(STGM_READ)?;
            let value = store.GetValue(&PKEY_Device_FriendlyName)?;
            let name = PropVariantToStringAlloc(&value).map(|p| p.to_string().unwrap_or_default()).unwrap_or_default();
            if name.to_lowercase().contains(&wanted) {
                found = Some((device.GetId()?.to_string().unwrap_or_default(), name));
                break;
            }
        }
        let Some(found) = found else {
            eprintln!("no active render endpoint whose name contains {wanted:?}");
            std::process::exit(1);
        };
        found
    };
    // "cached": CaptureSessions (looked up once), else a full look-up a poll
    let cached = match args.next().as_deref() {
        Some("cached") => win_sys::audio_policy::CaptureSessions::open(&id)?,
        _ => None,
    };
    println!("watching the capture side of {name} for {secs} s ({})", if cached.is_some() { "cached" } else { "full" });
    let t0 = Instant::now();
    let until = t0 + Duration::from_secs(secs);
    let mut last = None;
    let mut slowest = Duration::ZERO;
    let (mut total, mut polls) = (Duration::ZERO, 0u32);
    while Instant::now() < until {
        let start = Instant::now();
        let now = match &cached {
            Some(c) => Some(c.active()),
            None => win_sys::audio_policy::capture_sessions_active(&id)?,
        };
        let took = start.elapsed();
        slowest = slowest.max(took);
        total += took;
        polls += 1;
        if now != last {
            println!("{:>8.3} s  {now:?}", t0.elapsed().as_secs_f64());
            last = now;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    println!("{polls} polls: mean {} us, slowest {} us", (total / polls.max(1)).as_micros(), slowest.as_micros());
    Ok(())
}

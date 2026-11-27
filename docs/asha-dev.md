---
title: Hearing aids (developer note)
layout: default
nav_exclude: true
english_only: true
---

# Hearing aids (developer note)

Status: proof of concept, in developer mode. Verified on the emulator only.
No real hearing aid has been tested.

## What it does

ASHA (Audio Streaming for Hearing Aids) is how Android sends sound to
hearing aids over Bluetooth LE. BlueCaster can do the same, from the window
and from the command line, in two ways:

- **Hearing aids only.** One hearing aid, or the left and right of a pair.
  A pair plays stereo. A single hearing aid plays both sides mixed.
- **Bimodal.** One hearing aid plays alongside an A2DP stream to another
  device: a hearing aid in one ear, another device in the other. The A2DP
  stream is unchanged. The hearing aid plays its own side and can be delayed
  so that both ears hear the sound at the same time.

The sound is G.722 at 16 kHz, mono per hearing aid. This covers frequencies
up to 7 kHz: good for speech, limited for music. ASHA has no microphone path.

## In the window

Hearing aids need Bluetooth LE, and BlueCaster starts Bluetooth LE only when
LE Audio is turned on:

1. Turn on **Developer mode** in Settings, then **Use LE Audio when
   available (experimental)** on Adapters. The adapter does not need to
   support LE Audio itself.
2. Turn off Bluetooth on the phone. A hearing aid advertises only while it
   is not connected to a phone.
3. Scan on Audio Devices. A hearing aid is listed with the tag **Hearing
   aid**. A pair appears as one entry.
4. Add the hearing aid and connect to it. BlueCaster finds the other side of
   a pair automatically. If BlueCaster does not find the other side within
   4 seconds, the connected hearing aid plays both sides mixed, and the
   status shows that only one is connected.

The volume slider and Windows Volume Sync set the hearing aids' own volume. The
Codecs screen shows only G.722; there is nothing to choose.

For bimodal, open a classic device's card on Audio Devices and choose a
saved hearing aid under **Hearing aid for the other ear**. The hearing aid
then connects whenever that device streams. **Hearing aid volume** sets how
loud the hearing aid plays. **Hearing aid delay** delays the hearing aid's
sound by up to 500 ms. Changes to either take effect immediately, even while
the device is streaming.

The status line shows what the hearing aid is doing:

- **Connected. Connecting to the hearing aid...**
- **Connected. The hearing aid is playing too.**
- **Connected. The hearing aid was not found.** BlueCaster keeps searching
  every 5 seconds.
- **Connected. Turn on LE Audio to use the hearing aid.**

## From the command line

Quit BlueCaster from the tray first. Only one program can use the adapter
at a time.

List the LE devices nearby to find the hearing aid's address:

```powershell
bluecaster-core.exe --cli --le-probe
```

Turn off Bluetooth on the phone first. A hearing aid advertises only while
it is not connected to a phone.

A hearing aid, or a pair by the address of either side, with system audio:

```powershell
bluecaster-core.exe --cli --asha <address>
```

One hearing aid alongside an A2DP device, delayed 60 ms:

```powershell
bluecaster-core.exe --cli -d <A2DP address> --asha <address> --asha-delay 60
```

- `--asha-volume <n>` sets the hearing aids' volume, from -128 (muted) to 0.
  The default is -20. Not with `-d`.
- `--asha-delay <ms>` delays the hearing aids by 0 to 500 ms.
- `--test-tone` plays 1 kHz on the left and 1.5 kHz on the right instead of
  system audio.

The first connection pairs the hearing aid with BlueCaster. A hearing aid
that is paired with a phone may need to be put in pairing mode.

## How it works

| Part | File |
|------|------|
| LE credit based channels in BTstack | `app/src/crates/btstack-sys/csrc/shim.c`, `src/le.rs` |
| The audio channel of each hearing aid and its send queue | `app/src/crates/bt-host/src/le/modules/asha.rs` |
| G.722 encoder | `app/src/crates/codec/src/g722.rs` |
| Downmix, low-pass, 16 kHz, 20 ms frames, delay | `app/src/crates/pipeline/src/asha.rs` |
| Connecting, starting, stopping; the hearing aid beside an A2DP stream | `app/src/crates/core/src/asha.rs` |
| The session of a hearing aid or a pair | `app/src/crates/core/src/asha_stream.rs` |
| Hearing aids in a scan (`hearing_aid_entries`) | `app/src/crates/core/src/service.rs` |
| The hearing aid for the other ear on a device's card | `app/src/ui/src/ui/hearing_aid.rs` |
| `--asha` options | `app/src/core/src/cli.rs` |

The window and the command line start the same sessions. A saved hearing aid
is an LE entry whose profile has `hearing_aid` set. A classic device's
profile stores its hearing aid in `hearing_aid_beside`, the delay in
`hearing_aid_delay_ms` and the volume in `hearing_aid_volume`.

The steps follow Android's
[ASHA specification](https://source.android.com/docs/core/connect/bluetooth/asha)
and, where the specification leaves a choice, what Android itself sends:

1. Connect and pair the hearing aid, then read its GATT services.
2. Request the link parameters ASHA streams on: a 20 ms connection interval
   (latency 10, supervision timeout 1 s, connection event length 7.5 ms),
   the 2M PHY and 167-byte link layer packets. The changes take effect in
   the background.
3. Read ReadOnlyProperties (side, pair, HiSyncId, render delay, codecs) and
   LE_PSM_OUT, and open the audio channel on that PSM.
4. Wait for the 20 ms interval, then tell each hearing aid about the new
   interval (status 2).
5. Write Start to AudioControlPoint, one hearing aid after the other, and
   wait for AudioStatus each time. Start carries the codec, audio type 0,
   the volume, and whether the other side is already streaming. Then tell
   the side that started first that the other side is connected (status 1).
6. Send one SDU per hearing aid every 20 ms: a sequence counter byte and 160
   bytes of G.722. Both sides of a pair use the same counter, which starts
   at 0.

A controller without the 2M PHY rejects the PHY request, and the stream runs
on 1M. If the interval does not reach 20 ms within 3 seconds, BlueCaster
logs a warning and starts anyway.

Each hearing aid has a queue of four SDUs (80 ms). When a hearing aid cannot
keep up, BlueCaster drops the oldest SDU, so the sound never falls behind.

In bimodal mode the hearing aid's encode thread gets a copy of what the A2DP
stream captures. BlueCaster does not measure the delay between the two paths.
Set the delay by ear. When the hearing aid disconnects, BlueCaster searches
for it every 5 seconds while the A2DP stream keeps playing.

## Testing

```powershell
python tools/emu/run_test.py --asha
```

This runs three cases against emulated hearing aids
(`tools/emu/emu_asha.py`): a pair, a single hearing aid, and a hearing aid
alongside an A2DP sink. Each case
checks the start command, the number of SDUs, their counters and the tone
each hearing aid received.

## Not done yet

- Testing with real hearing aids. Expect differences in pairing, private addresses and
  connection parameters.
- Telling one side when the other side disconnects (status 0). When a
  hearing aid disconnects, BlueCaster sets up the whole stream again instead
  (up to 3 times, with auto-reconnect on).
- A separate setting for hearing aids. They share the LE Audio switch for
  now.
- Remembering the pair. BlueCaster searches for the other side at every
  connection.
- Bimodal: the hearing aid's volume is a setting on the device's card. The
  volume slider and Windows Volume Sync still change only the A2DP device's
  volume.
- Bimodal: the A2DP device still gets both sides. Still to do: send the
  A2DP device only its own side, and measure the delay between the two
  paths.

---
title: AFH Blend (developer note)
layout: default
nav_exclude: true
english_only: true
---

# AFH Blend - developer note

**Blend** is an AFH policy alongside Auto, Off and Manual, available only in
developer mode (`blend-mild`, `blend-medium`, `blend-strong`; without
developer mode the core runs a saved `blend-*` as `auto`). It is not the
default and must not become the default until the measurements below show it
helps. What users see is described in [Link Quality and AFH](usage#afh-blend).

## Why

Auto avoids only the 2.4 GHz Wi-Fi channel the PC is connected to. On
2026-10-17, avoiding every access point at -75 dBm or louder (v1 and early
v2) removed a loud but idle neighbour's channel 11, left 56 of 79 channels, and
raised gaps over 40 ms from about 6 to 30-95 a minute. A loud access point
alone is not evidence of interference. The controller's own AFH map is: it
shows which channels the radio found bad. Blend combines both signals and
keeps far more channels than the specification's minimum of 20.

## How it decides

Code: `app/src/crates/bt-host/src/afh.rs` (`blend`, `ControllerHistory`),
tests in `app/src/crates/bt-host/tests/afh.rs`; the scans run in
`app/src/crates/core/src/adapter.rs` (`WifiAfh`).

Per Bluetooth channel n (2402 + n MHz):

| Signal | Value 0..1 |
|:--|:--|
| Wi-Fi | access points heard within +-11 MHz, power summed; -90 dBm = 0, -50 dBm = 1, linear in dB between |
| Controller | share of the last 24 Read AFH Channel Map readings (one every 5 s, about 2 minutes) that left n out |

- Score = mean of the signals present. A single signal counts in full; with
  no signal, the score is 0.
- Readings are not counted for channels the host classification marked bad
  (the controller leaves those out because it was told to), nor for the 2
  readings after a new classification. Observations older than 120
  readings (10 minutes) expire, so a channel kept out long enough loses its
  controller signal and is judged by Wi-Fi alone.
- The PC's own 2.4 GHz channel is avoided first, exactly as in Auto.
- Next come channels at or above the threshold, highest score first, until
  the floor is reached. A channel that Blend marked bad last time needs 0.1
  less (hysteresis).

| Strictness | Threshold | Channels kept at least |
|:--|:--|:--|
| Mild | 0.75 | 56 (one Wi-Fi channel's worth) |
| Medium (chosen first) | 0.55 | 48 |
| Strong | 0.35 | 40 |

Consequences worth knowing when reading results:

- With the PC on 2.4 GHz Wi-Fi, Mild behaves exactly like Auto (Auto already leaves 56).
- A loud (-55 dBm) access point that the controller keeps using scores 0.44,
  so only Strong avoids it. At -75 dBm (the 2026-10-17 trap) no strictness does.
- Without location access there is no scan, and the controller's map
  decides alone. Blend then only reinforces what the controller already
  does, which may still matter (the controller re-tests left-out channels).
- The classification is recomputed every 30 s, as in Auto.

The scan is `win_sys::wlan::scan_24ghz` (WlanScan, then the BSS list). It runs on
a separate thread every 120 s while Blend is active, and stops 3 minutes after the last
classification that needed it; a scan older than 10 minutes is ignored. A
scan briefly takes the PC's Wi-Fi off its channel, which may itself cause
gaps; that is why the plan below has a scan-only control.

The debug log has one line per classification:
`AFH blend medium: 52 of 79 usable, left out 24-46, 60-63; scan 7 access points, controller readings 84`
and one per scan: `AFH blend: scan heard ch 6 -48 dBm, ...`.

## Measurement plan

Goal: for each strictness, compare gaps over 40 ms per minute and dropouts
per minute against Auto, with real adapters, in environments where Auto is
known to work well and where it does not.

### Setup

- **Sender**: the Windows PC with a build of this branch and the adapter
  under test (start with the UB500 / RTL8761BU from the 2026-10-17 test, then
  every other adapter at hand: Intel, Broadcom, CSR). Use a copy of the
  settings folder via `BLUECASTER_CONFIG_DIR`, never the real one.
- **Receiver**: `tools/linux_sink` on a Linux PC, as in
  [Peer Receiver Test](usage#receiver). BlueCaster is the central, so the host
  classification applies just as it does with headphones. Keep both at fixed positions, 3 m
  apart, the same for every run; note which receiver adapter was used.
- **Codec**: LDAC 990 kbps (the most demanding) for all runs; then one round
  with SBC to confirm that nothing behaves fundamentally differently.
- **Location access** on for the CLI (`Diagnostics > Radio > Scan` lists
  networks). One extra round with it off checks the controller-only path.

### Metrics

From the receiver's CSV (`--csv`, one row per second, counters cumulative
for the stream):

- **Gaps over 40 ms per minute**: `long_gaps`, last row minus the row at
  minute 2, divided by the minutes between them
- **Dropouts per minute**: `underruns`, calculated the same way (200 ms playout buffer)

Also record for each run: `lost` / `late`, the sender's packets dropped before
sending (CLI end summary), the channels in use (sender log `radio: AFH
mode ... channels in use`), and the blend lines above. To cross-check
against the 2026-10-17 method, do one run per policy with `--hci-capture` (gaps
between Number Of Completed Packets events).

### Runs

Each run lasts 10 minutes; report the first 2 minutes separately (Blend
has no controller history yet). Interleave the policies so that any change
in the room affects all of them:

```
auto, blend-medium, auto, blend-mild, auto, blend-strong   (x3)
```

plus one `off` run per scenario. Commands:

```bash
sudo sh run.sh --csv run-<scenario>-<policy>-<n>.csv          # Linux
```

```powershell
$env:BLUECASTER_LOG = "info,bt_host::afh=debug,bluecaster_core::adapter=debug"
bluecaster-core.exe --cli --remote-sink auto -c ldac --test-tone --duration 600 --afh blend-medium
```

Scenarios (each with all policies):

| | PC Wi-Fi | Other 2.4 GHz traffic | What it tests |
|:--|:--|:--|:--|
| A | 2.4 GHz, busy (a download or video on the PC) | none | Blend does no harm compared with Auto |
| B | 5 GHz | another device streaming video on a 2.4 GHz access point 1-3 m away | where Auto does nothing and Blend should help |
| C | 5 GHz or off | quiet 2.4 GHz | Blend leaves a clean band alone |
| D | 5 GHz | a loud but idle 2.4 GHz access point (the 2026-10-17 case) | Blend does not repeat that mistake |

Scan-only control (scenario B): run Auto and press **Diagnostics > Radio > Scan**
by hand every 2 minutes. If gaps also cluster at the scan times here,
the cost comes from the scan interval, not the classification.

### Reading the results

For each scenario and policy: the mean and range over the 3 runs (minutes
2-10) of gaps over 40 ms per minute and of dropouts per minute.

- A strictness is worth keeping if it is no worse than Auto (within
  the run-to-run range) in A, C and D, clearly better (say a third fewer
  gaps) in B, and has no more dropouts anywhere.
- If none qualifies, tune before measuring again. The knobs, all in `afh.rs`: the
  thresholds and floors (`Strictness`), `WIFI_QUIET_DBM` / `WIFI_LOUD_DBM`,
  the history length (`HISTORY_OBSERVATIONS`), and the even weighting of
  the two signals in `channel_scores`; the scan interval is in `adapter.rs`.
- Even after a good result, Blend stays in developer mode until more than one
  adapter and more than one location agree.

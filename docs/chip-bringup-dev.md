---
title: Chip bring-up and probing (developer note)
layout: default
nav_exclude: true
english_only: true
---

# Chip bring-up and probing - developer note

**Status: the generic foundation is done (steps 1 to 4). Loaders for MediaTek,
Qualcomm and other chips are left to contributors who have the hardware; the
procedure is in [Adding Chip Support](chip-support).**

BlueCaster drives the adapter directly, so every chip family that needs a firmware
download needs its own loader (Realtek in BTstack, Broadcom PatchRAM, Intel
since #261). MediaTek (MT7921 / MT7922) and Qualcomm (QCA61x4) have no loader and
are shown as unsupported. Anyone who wants to add one needs to see what the chip
says, and the maintainers have no MediaTek or Qualcomm hardware. So BlueCaster
provides the tools, not the vendor code:

1. a **common loader interface**, so a new vendor is a state machine plus
   tests, not changes across the stack (#282);
2. a **chip probe** that any adapter on WinUSB can run; it only reads, and it
   records everything it sends and receives (#287);
3. **USB control transfers** for loaders, the USB descriptors in the probe,
   and a **sample loader** with tests to start from (#292);
4. developer-mode **tools**: a report button after the probe, **chip
   scripts** (a console and step files), and a loader's **progress** on the
   Adapters screen.

## What it builds on

| Piece | Where | Used for |
|:--|:--|:--|
| Raw mode: packets go to the app before `hci_init` | `Stack::raw_begin` / `raw_send` / `raw_end` (`btstack-sys`) | probe and loaders |
| Raw packets in the HCI capture | `log_packet` in `btstack-sys/src/callbacks.rs` | probe and loader traffic in `.pklg` |
| Flight recorder and diagnostic bundle | `bt-host/src/capture.rs`, `core/src/bundle.rs` | carry the probe reports |
| Bootloader mode (events on bulk IN, no HCI Reset on close) | `hci-transport/src/usb.rs` | `Action::Bootloader` |
| Vendor detection | `bt-host/src/chips.rs` (`ChipVendor`) | picks the loader |

## 1. Common loader interface

`bt-host/src/bringup.rs`: `Action`, `Outcome`, `LoadError` and the
`ChipLoader` trait (`vendor`, `start`, `on_event`, `on_control`,
`on_timeout`, `missing_file_hint`). `HostApp::start_loader` / `run_loader` in
`app.rs` carry out the actions for any loader; `intel::Loader` implements the
trait.

A loader does no I/O itself. `Action::UsbControl` is the only action with
an immediate result: the runner performs the transfer
(`HciBackend::control`, nusb on the USB transport; H4 refuses it) and passes
the result to `on_control` before the next action. Control transfers are not
HCI packets, so `Stack::raw_control` writes each transfer and its result to the
HCI capture as a log line, which keeps the whole sequence in order there.

`bringup.rs` also has `hci_command` / `parse_event`, and `bringup/sim.rs`
runs a loader against a simulated controller using the runner's rules (the
timer, `UsbControl` before the next action), so loader tests do not need
their own driver. Support is decided per chip: `chips::LOADER_CHIPS` lists the chips of an
otherwise unsupported vendor that have a loader, and `chip_supported` /
`chip_support_tag` control the name tag, the adapter note (and with it
WinUSB on the Adapters screen) and the unsupported-adapter warning.

`bringup/sample.rs` is a complete loader for a made-up
chip, with tests
against a simulated controller. It is compiled only for tests.

## 2. Chip probe

`bt-host/src/probe.rs`: a `ChipLoader` that never writes to the chip. It runs in
place of HCI init (`BtHost::probe`) on any adapter with WinUSB, including
vendors marked unsupported, and over TCP on the emulator's controller. It sends no HCI
Reset, not even when the adapter is closed afterwards (the probe turns on
bootloader mode), because a bootloader may not accept one.

Each step is answered, times out (1 s) or fails; then the next step runs:

1. HCI: Read Local Version, Read BD_ADDR, Read Local Supported Commands /
   Features, Read Buffer Size, Read Local Name, LE Read Local Supported
   Features, LE Read Buffer Size. Events that answer no step (such as a bootloader's
   vendor events) are kept too.
2. USB (standard GET_DESCRIPTOR on the default pipe): the device descriptor,
   then the manufacturer and product strings it references.
3. The runner adds the descriptors Windows keeps (`HciBackend::usb_descriptors`):
   one line per interface setting and the raw configuration descriptors.

The probe sends no vendor commands: which ones are harmless differs per chip, and
whoever writes the chip's loader should work that out from captures (see the
guide). A vendor's read steps belong in its loader.

Output: `logs\probe\probe-<time>.json` with a decoded summary (company, HCI /
LMP version, address, name, buffers, USB IDs and strings) and every request
in hex as sent and received (HCI: command and event; USB: SETUP packet and
the bytes read); the newest 10 are kept. The diagnostic bundle includes the
reports only together with the HCI captures, because they contain the adapter's address.

How to run it: in developer mode, open Adapters and click **Read chip info** on an adapter
with WinUSB (developer mode also offers WinUSB for an unsupported adapter);
or run `bluecaster-core --cli -u <adapter> --probe`, or with `--hci-tcp` on the
emulator (tools/emu).

## 4. Tools in developer mode

**Report button**: after **Read chip info**, the adapter's card keeps the
summary and opens a `chip_report.yml` issue prefilled with the summary, the version
and the Windows build (`chip_report_url` in `crates/ui-model/src/links.rs`). The report
itself goes into the issue as a JSON block (`ProbeResult.report`); one too long
for the URL leaves the required field empty, and Explorer shows the file for
attaching (#518).

**Chip scripts** (`bt-host/src/script.rs`): plain text, one request per line
(`hci`, `bulk`, `usb in/out`, `wait`, `expect`, `bootloader`, `upload`,
`upload-bulk`),
parsed with line numbers and run by `ScriptLoader`, a `ChipLoader`, through
`BtHost::run_script` (the probe and scripts share `BtHost::run_tool`). Scripts are
the only way to send writes from outside the code, so:

- `Step::reads_only` lists what runs freely: HCI OGF 0x04, OGF 0x05 except
  Reset Failed Contact Counter, a few reads of OGF 0x03 / 0x08, standard
  GET_STATUS / GET_DESCRIPTOR / GET_CONFIGURATION / GET_INTERFACE, `wait`,
  `expect`, `bootloader`. Anything else needs `allow_writes` (the window's
  switch, `--allow-writes`), which the core checks
- `chips::denied_opcodes(vendor)` never run, whatever is allowed: these are vendor
  commands that write memory that persists across a power cycle (so far, CSR BCCMD)
- uploads read only from the firmware folder; the script stops at its first
  failure; bootloader mode is on from the start (no HCI Reset on close)

The window checks the script as you type (`adapter.check_script`) and
runs it (`adapter.run_script`); the report, which includes the script, is saved as `script-<time>.json` beside
the probe reports.

**Loader progress**: `ChipLoader::step` / `progress`, reported after each
batch of actions through `HostConfig::on_loader` (a hook, because the loader runs
inside `BtHost::start`, before the core has the stack's handle), into
`ServiceEvent::Loader` and `AdapterState::loader`. Intel reports its steps
and the Secure Send percentage; tools do not report progress.

## Open questions

- MediaTek modules often use OEM VIDs (AzureWave 0x13D3, Foxconn 0x0489) that
  Realtek modules also use, so detection may need VID:PID pairs or the probe's
  Read Local Version company ID (MediaTek 0x0046) once WinUSB is installed.
- Should the probe also run once automatically on an unsupported
  adapter that is already on WinUSB? For now it runs only on request.

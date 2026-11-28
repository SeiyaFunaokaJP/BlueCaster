---
title: Auracast modes (developer note)
layout: default
nav_exclude: true
english_only: true
---

# Auracast modes

Every Auracast layout BlueCaster has, grouped four ways, with what works on
which adapter and the settings of the scenarios tested so far. Developer
mode only. Background: [LE Audio developer note](le-audio-dev.md),
"Broadcast". Results are from the ASUS BT600 (Realtek 8761) unless noted.

## Terms

- **BIG**: one broadcast on air (Broadcast Isochronous Group). Each BIG is
  a broadcast with its own name.
- **BIS**: one audio stream in a BIG.
- **Subgroup**: BIS of a BIG that share a codec configuration and a label
  (Program_Info). A receiver left to itself takes the first subgroup.
- **Location**: Audio_Channel_Allocation of a BIS (FL, FR, BL, ...).
  Receivers pick BIS by it. HomeBuddy and Avantree speakers take only FL,
  FR or a BIS without a location ("mono"); they refuse BL, BR and FC.
- **Assistant**: BlueCaster as a BASS client. It connects to a receiver and
  tells it which broadcast and which BIS to play (Add Source, BIS_Sync).
  Set up in the Auracast tab's **Receivers** section or with `--assist`.

## The modes

Names as `--auracast-layout` and `stream.start_broadcast`'s `layout` take
them (`-` and `_` are the same).

| Mode | In the Auracast tab | On air |
|---|---|---|
| `stereo` | Channels: Stereo | 1 BIG, 2 BIS (FL FR) |
| `mono` | Channels: Mono | 1 BIG, 1 BIS (downmix) |
| `split` | Channels: Left / right separately | 2 BIGs "<name> L" / "<name> R", 1 mono BIS each |
| `single` | 1 BIG, 4 BIS: FL FR, rear without location | 1 subgroup: FL FR + 2 BIS without a location |
| `single_located` | 1 BIG, 4 BIS: FL FR BL BR | 1 subgroup: FL FR BL BR |
| `single_pairs` | 1 BIG, 4 BIS: Two FL FR subgroups | 2 subgroups of FL FR: front, rear |
| `single_per_bis` | 1 BIG, 4 BIS: One subgroup per BIS | 4 labelled subgroups of 1 BIS |
| `single_2_1` | 1 BIG, 4 BIS: 2.1, subwoofer pair first | subwoofer FL FR (low passed) + front FL FR |
| `single_2_1_mono` | 1 BIG, 4 BIS: 2.1, one mono subwoofer BIS | mono subwoofer + front FL FR (3 BIS) |
| `single_3_1` | 1 BIG, 4 BIS: 3.1 | mono subwoofer + FL FR + centre without a location |
| `matrix_<front>_<rear>` | Channels: Matrix; Front / Rear each Stereo, Separately, Subgroups or Off | per pair: Stereo = 1 BIG of FL FR; Separately = 2 mono BIGs; Subgroups = 1 BIG of 2 labelled mono subgroups; Off = nothing |

The matrix gives 15 modes (4 × 4 minus `off_off`). Older names are
aliases: `quad` = `matrix_separate_separate`, `front_rear` =
`matrix_stereo_separate`, `pairs` = `matrix_stereo_stereo`,
`rear_subgroups` = `matrix_stereo_subgroups`, `subgroups` =
`matrix_subgroups_subgroups`. Every mode except `stereo`, `mono` and
`split` takes the matrix mixer's outputs, so it needs a 5.1 or 7.1 source
(for example VB-Audio Hi-Fi Cable) to carry more than two channels.

In total: 3 basic + 7 one-BIG + 15 matrix = 25 modes.

### Duplicates

The same on air:

| Mode | Same as | Difference |
|---|---|---|
| `matrix_stereo_off` | `stereo` | goes through the matrix mixer; name suffix |
| `matrix_separate_off` | `split` | name suffixes FL / FR instead of L / R |
| `matrix_off_stereo` | `matrix_stereo_off` | fed from the rear outputs |
| `matrix_off_separate` | `matrix_separate_off` | fed from the rear outputs |
| `matrix_off_subgroups` | `matrix_subgroups_off` | fed from the rear outputs |
| `single_located` | `single` | rear located BL BR, which HomeBuddy and Avantree refuse |

The same idea in two BIGs instead of one (two BIGs lose SDUs on the BT600,
so the one-BIG mode replaces them):

| Two BIGs | One BIG |
|---|---|
| `matrix_stereo_stereo` | `single_pairs` |
| `matrix_subgroups_subgroups` | `single_per_bis` |

## By channels

No 5.1 or 7.1 mode exists: the matrix mixes a 5.1 or 7.1 source down to at
most four outputs. No 3.0 either.

| Channels | Modes | Count |
|---|---|---|
| 1.0 | `mono` | 1 |
| 2.0 | `stereo`, `split` | 2 |
| 2.0 | one matrix pair on: `matrix_stereo_off`, `matrix_separate_off`, `matrix_subgroups_off`, `matrix_off_stereo`, `matrix_off_separate`, `matrix_off_subgroups` | 6 |
| 2.1 | `single_2_1`, `single_2_1_mono` | 2 |
| 3.1 | `single_3_1` | 1 |
| 4.0 | `single`, `single_located`, `single_pairs`, `single_per_bis` | 4 |
| 4.0 | both matrix pairs on (3 × 3) | 9 |

## By BIG / BIS

**One BIG** (clean on the BT600 at the right rate, see below):

| BIS | Subgroups | Modes |
|---|---|---|
| 1 | 1 | `mono` |
| 2 | 1 (FL FR) | `stereo`, `matrix_stereo_off`, `matrix_off_stereo` |
| 2 | 2 (mono each) | `matrix_subgroups_off`, `matrix_off_subgroups` |
| 3 | 2 (sub + FL FR) | `single_2_1_mono` |
| 4 | 1 | `single`, `single_located` |
| 4 | 2 | `single_pairs`, `single_2_1`, `single_3_1` |
| 4 | 4 | `single_per_bis` |

**Two BIGs** (the BT600's maximum):

| Each BIG | Modes |
|---|---|
| 1 mono BIS | `split`, `matrix_separate_off`, `matrix_off_separate` |
| 2 BIS | `matrix_stereo_stereo`, `matrix_stereo_subgroups`, `matrix_subgroups_stereo`, `matrix_subgroups_subgroups` |

**Three or four BIGs** (the BT600 refuses the third BIG with 0x0D; the
Auracast tab only warns):

| BIGs | Modes |
|---|---|
| 3 | `matrix_stereo_separate`, `matrix_separate_stereo`, `matrix_subgroups_separate`, `matrix_separate_subgroups` |
| 4 | `matrix_separate_separate` |

## By the need for an assistant

A receiver left to itself can choose a broadcast by name, take the BIS of
its own location (FL / FR) in a subgroup, and, with a picker of its own
(JBL), choose a subgroup by its label. A simple receiver takes the first
subgroup.

**No assistant needed**:

| Mode | How each receiver finds its channel |
|---|---|
| `mono` | one programme for all |
| `stereo`, `matrix_stereo_off`, `matrix_off_stereo` | a stereo receiver takes FL FR; one set to left or right takes its BIS |
| `split`, `matrix_separate_off`, `matrix_off_separate` | broadcast by name |
| `matrix_stereo_stereo` | "<name> Front" / "<name> Rear" by name |
| `matrix_stereo_separate`, `matrix_separate_stereo`, `matrix_separate_separate` | a broadcast each, by name (not on the BT600: 3-4 BIGs) |

**Assistant needed unless the receiver has a subgroup picker**:

| Mode | What a simple receiver gets without an assistant |
|---|---|
| `matrix_subgroups_off`, `matrix_off_subgroups` | the first mono subgroup (left) |
| `matrix_stereo_subgroups`, `matrix_separate_subgroups` | front by name; rear only with a picker or an assistant |
| `matrix_subgroups_stereo`, `matrix_subgroups_separate` | rear by name; front only with a picker or an assistant |
| `matrix_subgroups_subgroups` | the first subgroup of each broadcast |
| `single_pairs` | the front (first subgroup); the rear needs a picker or an assistant |
| `single_per_bis` | BIS 1 (FL) only |
| `single_2_1`, `single_2_1_mono`, `single_3_1` | the subwoofer (first subgroup, e.g. MUVO FLEX by itself); the main pair and centre need a picker or an assistant |

**Assistant always needed** (one subgroup, the channel is chosen only by
BIS number):

| Mode | Why |
|---|---|
| `single` | the front is taken by location; the rear BIS have none |
| `single_located` | a BL / BR receiver would pick the rear by itself, but HomeBuddy and Avantree refuse BL / BR even when an assistant asks |

## By sample rate

The Auracast tab's quality picks the preset: High = `48_4_2` (48 kHz,
96 kbit/s per channel), Medium = `32_2_2` (32 kHz, 64 kbit/s), Standard =
`24_2_2` (24 kHz, 48 kbit/s). `--le-preset` also takes `16_2_2` and
`48_2_2`; they are not tested. One BIG has one rate. Only the matrix modes
can give the rear a rate of its own (Rear quality, `--auracast-rear-preset`).

| Modes | 48 kHz | 32 kHz | 24 kHz |
|---|---|---|---|
| 1 BIG, 1-2 BIS (`mono`, `stereo`, one matrix pair) | clean, 0 dropped | clean | clean |
| 2 BIGs × 1 BIS (`split`, `matrix_separate_off`) | works (two JBL GO 4, no offset) | not measured | works (two JBL GO 4) |
| 2 BIGs × 2 BIS (`matrix_stereo_stereo`, ...) | ~27 % of SDU sets dropped | not measured | starts; drops not measured |
| 1 BIG, 3 BIS (`single_2_1_mono`) | not measured | not measured separately | not measured separately |
| 1 BIG, 4 BIS (other `single…`) | 12-60 % dropped (by run) | 0 dropped; can fall into half rate, now recovered by a BIG recreate | 0 dropped in 22 min (0.3 % in another 12 min run) |

Receivers and rates:

| Receiver | 48 kHz | 32 kHz | 24 kHz |
|---|---|---|---|
| HomeBuddy, Avantree speakers | yes | yes | yes |
| Creative MUVO FLEX | not tested | yes | yes |
| JBL Go 4 and similar | yes | **no** | yes |
| Earbuds (Xiaomi Buds 5, LE520pro) | yes | yes | yes |

nRF5340 DK ([tools/nrf5340-hci](../tools/nrf5340-hci/README.md), up to
four BIGs, four BIS in total), tested with JBL only: one BIG of two BIS is
stable. Four BIGs start, but at 48 kHz with RTN 2 the air is nearly full
and the sound drops out. With `BLUECASTER_AURACAST_RTN=1` and
`BLUECASTER_AURACAST_LATENCY_MS=20` all layouts play fairly well, with
occasional stutters.

## Tested scenarios and their settings

The command lines run `bluecaster-core.exe --cli` with developer mode and
LE Audio on. `<A>`, `<B>`, ... are the receivers' addresses as BlueCaster
lists them (`--le-scan`). In the app, receivers for the **Receivers**
section must be saved LE Audio devices with BASS.

### Stereo at the best quality (any receiver)

- Tab: Quality High (48 kHz), Channels Stereo, source = the output to
  play.
- CLI: `--auracast BlueCaster --auracast-volume 50`
- Result: 1 BIG, 2 BIS, 0 dropped on the BT600.

### Left and right on two speakers without an assistant (JBL)

- Tab: Quality High or Standard, Channels Left / right separately. On each
  speaker choose "<name> L" or "<name> R".
- CLI: `--auracast BlueCaster --auracast-split --le-preset 24_2_2 --auracast-volume 50`
- Result: two BIGs at 24 and 48 kHz, no audible offset (two JBL GO 4).

### Left and right on two HomeBuddy via the assistant

- Tab: Channels Stereo; Receivers: one speaker on FL, the other on FR.
- CLI: `--auracast BlueCaster --assist <A>=1 --assist <B>=2 --auracast-volume 50`
- Result: each plays its channel; they start together.

### 4.0 on four Avantree speakers (5.1 films)

- Source: a 5.1 output (VB-Audio Hi-Fi Cable set to 5.1, the player's
  output on it).
- Tab: Quality Medium (32 kHz), or Standard (24 kHz) if it drops. Channels
  1 BIG, 4 BIS, BIS layout "FL FR, rear without location". Who plays what:
  4 mono, one speaker per slot. Matrix mixer: Automatic matrix on.
- CLI test with a tone per channel: `--auracast BlueCaster --auracast-layout single --le-preset 32_2_2 --test-tone-quad --assist <A>=1 --assist <B>=2 --assist <C>=3 --assist <D>=4 --auracast-volume 50`
- Use the mixer's **S** (solo) and **M** (mute) on a row or column to
  check which speaker plays which channel. They are not saved.
- Result: very good surround in a small quiet room, also at night. The
  channel balance needs adjusting per film. A centre speaker is not
  needed: localisation is good. Plain stereo in 4.0 also sounds good. The
  LFE is not heard: these speakers cannot reproduce it. 24 kHz: 0 dropped
  in 22 min.
- Known problem: after a BIG recreate (half rate) two of the four speakers
  may stay silent until the next start. See PR 593, "Known problems".

### 4.0 with earbuds on the front

- Tab: Channels 1 BIG, 4 BIS, "FL FR, rear without location"; Who plays
  what: 1 stereo + 2 mono (the earbuds on the front, two HomeBuddy on the
  rear).
- CLI: `--auracast BlueCaster --auracast-layout single --le-preset 32_2_2 --assist <E>=1,2 --assist <A>=3 --assist <B>=4 --auracast-volume 50`
- Result: four distinct channels at once, in sync. Earbuds play BIS 1, 2
  whatever they are asked, so they fit only the front.

### 2.1 with a standalone subwoofer

- Tested: Quality Medium or Standard. Channels 1 BIG, 4 BIS, BIS layout
  "2.1: subwoofer pair first, then the main pair". Who plays what: the main
  pair (two Avantree). Crossover 80 Hz (default). MUVO FLEX joins by itself
  and takes the first subgroup.
- CLI: `--auracast BlueCaster --auracast-layout single-2-1 --le-preset 32_2_2 --assist <A>=3 --assist <B>=4 --auracast-volume 50`
- MUVO FLEX plays only the left BIS of the subwoofer pair, so it gets half
  of the bass. "2.1: one mono subwoofer BIS first, then the main pair (3
  BIS)" (`single-2-1-mono`, main pair on BIS 2 / 3) fixes that. It has the
  same subwoofer subgroup as the tested 3.1 mode, but was not run on its
  own.

### 3.1

- Tab: BIS layout "3.1: one mono subwoofer BIS first, then FL FR and the
  center"; Who plays what: 3 mono (or 1 stereo + 1 center).
- CLI: `--auracast BlueCaster --auracast-layout single-3-1 --le-preset 32_2_2 --assist <A>=2 --assist <B>=3 --assist <C>=4 --auracast-volume 50`
- Result: the centre on a third speaker plays (it goes without a location,
  which Avantree accept).

### JBL with its own picker

- Quality Standard or High (no 32 kHz). Modes with labelled subgroups:
  `single_pairs` ("Front" / "Rear"), `single_per_bis` or a 2.1 / 3.1 mode
  ("Subwoofer" / "Front"). Choose the subgroup on the speaker.
- In the two-BIG `matrix_stereo_subgroups` the speaker showed "Rear left"
  or "Rear right" seemingly at random, so subgroup choice there is not
  reliable.

## Encryption (not tested yet)

Any mode can be encrypted. Nothing about it depends on the layout.

- Setting: the Auracast tab's **Password**, `--auracast-code <code>` or
  `code` in `stream.start_broadcast`. The Broadcast_Code is 4 to 16 bytes
  of UTF-8. Empty means an open broadcast.
- On air: every BIG of the layout is created encrypted with the same code,
  and the broadcast advertises the PBP "encrypted" feature. A BIG recreate
  after half rate reuses the same code. Each BIS PDU gains a 4-byte MIC,
  so the air time grows slightly. This matters for 4 BIS at 48 kHz or on a
  nearly full nRF5340.
- With the assistant: when a receiver reports "Broadcast_Code required",
  BlueCaster sends it the code (BASS Set Broadcast_Code) by itself. A wrong
  code shows as "wrong Broadcast_Code". So HomeBuddy and Avantree in the
  **Receivers** section should work without anyone typing the code.
- Without the assistant: the code has to be typed where the broadcast is
  chosen, on a phone or in the receiver's own app. A standalone sink with
  no way to enter it (MUVO FLEX) probably cannot join an encrypted
  broadcast at all. A 2.1 / 3.1 subwoofer of that kind then stays silent.
- With **Disconnect once playing** the code is sent before the link
  closes. A receiver that loses the broadcast later cannot be given the
  code again.
- The restored Auracast form brings the code back after a restart. The
  code is not in `auracast_last.json`: it is kept encrypted for the
  Windows user in `pairing\auracast_code.bin`, as the link keys are, and
  never goes into a diagnostic bundle.

## Test knobs

| Variable | Effect |
|---|---|
| `BLUECASTER_AURACAST_RTN=<n>` | retransmissions for every BIG (applied with more than one BIG) |
| `BLUECASTER_AURACAST_LATENCY_MS=<ms>` | max transport latency for every BIG (5-4000) |
| `BLUECASTER_AURACAST_NO_RECREATE=1` | leave a BIG that sends at half rate as it is |

## Known problems

- **Half rate on the BT600**: a 4-BIS BIG sometimes runs at half rate,
  usually while the assistant connects. BlueCaster now terminates and
  creates the BIG again after 5 slow seconds (at most 3 times). Some
  receivers do not re-sync after that (see the 4.0 scenario).
- **High CPU load**: the loopback capture can stall (once 15 s of silence)
  and the encoder falls behind; dropouts follow.
- **Two BIGs** lose SDUs on the BT600 at 48 kHz with 2 BIS each.

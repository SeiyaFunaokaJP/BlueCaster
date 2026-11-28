"""The hearing aid cases of run_test.py --asha (ASHA, a proof of concept).

Each case runs bluecaster-core.exe --cli --asha with the test tone (1 kHz
left, 1.5 kHz right) against emulated hearing aids. The command line starts
the same sessions the window does (crates/core asha_stream.rs, and the
hearing aid beside an A2DP stream of asha.rs):
  asha-pair     emu_asha.py --layout pair: left and right of a pair; the
                session is given the left one and finds the right one
  asha-left     emu_asha.py --layout left: one hearing aid, both sides
                mixed, its volume set once it streams
  asha-bimodal  emu_sink.py --asha right: an A2DP sink (SBC) and a right
                hearing aid beside it, delayed 40 ms

Checked from both logs and the G.722 each hearing aid wrote (decoded with
g722.py):
  - the 2M PHY and the 20 ms connection interval was asked for, with a connection event
    length, and each hearing aid was told about it before its start
  - each hearing aid was started with G.722, the audio type Android sends
    (unknown), the volume and whether the other side streams already; the
    side started first is then told that the other one is there
  - one SDU per 20 ms for the time streamed, none lost by its counter, none
    of another size, and BlueCaster dropped none
  - the tone of its side: 1 kHz left, 1.5 kHz right, both for one hearing
    aid alone
  - bimodal: the A2DP stream ran for the whole duration beside it, the
    hearing aid started with its own volume, and the status told when it
    was being connected and when it played
Outputs go to tools/emu/out/<case>/.
"""

import math
import os
import re
import shutil

import g722

LEFT, RIGHT = '00:A2:D0:00:00:22', '00:A2:D0:00:00:23'
SINK = '00:A2:D0:00:00:02'
SECONDS = 6
BIMODAL_SECONDS = 12
VOLUME = -30
# case: (emulator script, its options, bluecaster-core options, {side: (tones heard, tones not heard)})
CASES = {
    'asha-pair': ('emu_asha.py', ['--layout', 'pair'], ['--asha', f'{LEFT},{RIGHT}'],
                  {'left': ([1000], [1500]), 'right': ([1500], [1000])}),
    'asha-left': ('emu_asha.py', ['--layout', 'left'], ['--asha', LEFT, '--asha-volume', str(VOLUME)],
                  {'left': ([1000, 1500], [])}),
    'asha-bimodal': ('emu_sink.py', ['--asha', 'right'],
                     ['-d', SINK, '-c', 'sbc', '--asha', RIGHT, '--asha-delay', '40', '--asha-volume', str(VOLUME)],
                     {'right': ([1500], [1000])}),
}


def tone_share(pcm, hz):
    """Share of the power of `pcm` (16 kHz) at `hz`"""
    w = 2 * math.pi * hz / 16000
    re_ = sum(s * math.cos(w * n) for n, s in enumerate(pcm))
    im = sum(s * math.sin(w * n) for n, s in enumerate(pcm))
    return 2 * (re_ * re_ + im * im) / len(pcm) / (sum(s * s for s in pcm) or 1)


def run_case(rt, args, case, port):
    script, emu_args, cli_args, sides = CASES[case]
    out = os.path.join(args.out, case)
    shutil.rmtree(out, ignore_errors=True)
    os.makedirs(out)
    bimodal = script == 'emu_sink.py'
    seconds = BIMODAL_SECONDS if bimodal else SECONDS
    port = rt.free_port(port)
    emu_log = os.path.join(out, 'emu.log')
    base = os.path.join(out, 'aid')
    emu = rt.start_sink(port, os.path.join(out, 'emu.pklg'), emu_log, script=script,
                        extra=emu_args + ['--asha-out' if bimodal else '--out', base])
    try:
        env = dict(os.environ, BLUECASTER_CONFIG_DIR=os.path.join(out, 'config'), BLUECASTER_LOG='debug')
        rc, text = rt.run_cli([args.bluecaster, '--cli', '--hci-tcp', f'127.0.0.1:{port}',
                               '--hci-capture', os.path.join(out, 'bluecaster.pklg'), '--test-tone',
                               '--duration', str(seconds)] + cli_args, env, seconds + 90)
        with open(os.path.join(out, 'bluecaster.log'), 'w', encoding='utf-8') as f:
            f.write(text)
    finally:
        emu.terminate()
        emu.wait(10)
    emu_text = rt.read_text(emu_log)
    failures = []
    if rc != 0:
        failures.append(f'BlueCaster exited with {rc} (see bluecaster.log)')
    # Hearing aids alone start at -20, and --asha-volume sets theirs
    # afterwards, as the volume slider does; the one beside an A2DP stream
    # starts with its own volume
    volume = VOLUME if bimodal and '--asha-volume' in cli_args else -20
    details = []
    for index, (side, (heard, silent)) in enumerate(sides.items()):
        # The side started second is told that the other one streams
        other = 1 if index > 0 else 0
        start = re.search(rf'^asha: {side} start codec=(\d+) type=(\d+) volume=(-?\d+) other=(\d+)$', emu_text, re.M)
        if not start:
            failures.append(f'the {side} hearing aid was not started')
            continue
        got = tuple(int(g) for g in start.groups())
        if got != (1, 0, volume, other):
            failures.append(f'{side}: started with codec, type, volume, other = {got}, '
                            f'expected {(1, 0, volume, other)}')
        if '--asha-volume' in cli_args and not bimodal and f'asha: {side} volume {VOLUME}' not in emu_text:
            failures.append(f'{side}: the volume {VOLUME} was not set')
        told = f'asha: {side} status 1' in emu_text
        if len(sides) == 2 and told != (index == 0):
            failures.append(f'{side}: {"" if told else "not "}told that the other side is connected')
        update = emu_text.find(f'asha: {side} status 2 16')
        if update < 0 or update > start.start():
            failures.append(f'{side}: not told about the 20 ms connection interval before its start')
        stop = re.search(rf'^asha: {side} stop frames=(\d+) lost=(\d+) bad=(\d+)$', emu_text, re.M)
        if not stop:
            failures.append(f'the {side} hearing aid was not stopped')
            continue
        frames, lost, bad = (int(g) for g in stop.groups())
        # Bimodal: the hearing aid joins once the A2DP stream runs (it is
        # found and paired first)
        least = 50 * (seconds - (6 if bimodal else 1))
        if frames < least or frames > 50 * seconds + 10:
            failures.append(f'{side}: {frames} SDUs, expected {least} to {50 * seconds + 10}')
        if lost or bad:
            failures.append(f'{side}: {lost} SDUs lost, {bad} of another size')
        try:
            with open(f'{base}-{side}.g722', 'rb') as f:
                data = f.read()
        except OSError:
            data = b''
        # A second of it, after the first
        pcm = g722.decode(data[8000:16000])
        shares = {hz: tone_share(pcm, hz) if pcm else 0.0 for hz in (1000, 1500)}
        if abs(sum(shares[hz] for hz in heard) - 1) > 0.1 or any(shares[hz] < 0.9 / len(heard) - 0.1 for hz in heard):
            failures.append(f'{side}: not the tone of its side (shares {shares}, expected {heard} Hz)')
        if any(shares[hz] > 0.02 for hz in silent):
            failures.append(f'{side}: hears the other side too (shares {shares})')
        details.append(f'{side} {frames} SDUs ' + '+'.join(f'{hz} Hz {shares[hz]:.2f}' for hz in heard))
    # BlueCaster's own counts, as it printed them last
    counts = re.findall(r'ASHA: (?:left|right) \d+ sent, .*$', text, re.M)
    final = re.findall(r'(\d+) sent, (\d+) dropped', counts[-1]) if counts else []
    if not final:
        failures.append('no "ASHA: ... sent" line (see bluecaster.log)')
    elif any(int(dropped) for _, dropped in final):
        failures.append(f'BlueCaster dropped SDUs: {final}')
    updates = len(re.findall(r'le connection update interval=16 latency=10 timeout=100 ce=12', emu_text))
    if updates < len(sides):
        failures.append(f'{updates} request(s) for the 20 ms connection interval, expected {len(sides)}')
    if len(re.findall(r'le set phy tx=2 rx=2', emu_text)) < len(sides):
        failures.append('the 2M PHY was not asked for on every link')
    if bimodal:
        a2dp = re.search(r'Sent (\d+) media packets .* (\d+) dropped', text)
        if not a2dp:
            failures.append('no A2DP summary line')
        else:
            packets, dropped = int(a2dp.group(1)), int(a2dp.group(2))
            if packets < 80 * seconds or dropped:
                failures.append(f'A2DP: {packets} packets sent, {dropped} dropped, expected {80 * seconds} or more '
                                'and none dropped')
            details.append(f'A2DP {packets} packets')
        if 'plays beside this stream, 40 ms later' not in text:
            failures.append('the delay was not taken')
        # The status line tells what the hearing aid does
        for status in ('Connected. Connecting to the hearing aid...', 'Connected. The hearing aid is playing too.'):
            if f'[Streaming] {status}' not in text:
                failures.append(f'the status never said "{status}"')
    return failures, ', '.join(details) or 'nothing received'


def main(rt, args):
    wanted = args.asha or list(CASES)
    unknown = [n for n in wanted if n not in CASES]
    if unknown:
        raise SystemExit(f'Unknown hearing aid case(s) {unknown}; known: {list(CASES)}')
    results = []
    for index, case in enumerate(wanted):
        print(f'--- {case} ...', flush=True)
        failures, detail = run_case(rt, args, case, args.base_port + 700 + index)
        results.append((case, failures))
        print(f'    {"PASS" if not failures else "FAIL"}  {detail}')
        for failure in failures:
            print(f'      - {failure}')
    passed = sum(1 for _, f in results if not f)
    print(f'\n{passed}/{len(results)} hearing aid cases passed. Details: {args.out}')
    raise SystemExit(0 if passed == len(results) else 1)

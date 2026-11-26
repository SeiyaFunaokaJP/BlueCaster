"""Virtual ASHA hearing aids (Audio Streaming for Hearing Aids), for testing BlueCaster's ASHA support.

Runs Bumble virtual controllers on one in-process link, as
emu_le_audio_sink.py does:
  - controller "BlueCaster" is served as H4 over TCP; BlueCaster connects to
    it with `bluecaster-core.exe --cli --hci-tcp 127.0.0.1:<port> ...`
  - one or two hearing aid controllers, each driven by a Bumble host with the
    ASHA GATT service (0xFDF0: ReadOnlyProperties, AudioControlPoint,
    AudioStatus, Volume, LE_PSM_OUT) and an LE credit based L2CAP channel
    server on the PSM that LE_PSM_OUT names

Layouts (--layout):
  pair   two hearing aids, left and right, one HiSyncId (binaural)
  left   one hearing aid, left (monaural)
  right  one hearing aid, right (monaural)

A hearing aid takes one SDU per 20 ms on its channel: a sequence counter
byte followed by 160 bytes of G.722 (16 kHz mono, 64 kbit/s). With --out the
G.722 bytes of each side are written to <out>-left.g722 / <out>-right.g722,
without the counter.

Prints "READY <host:port> aids=<addr>[,<addr>]" once BlueCaster can connect,
then one line per step for run_test.py:
  asha: <side> start codec=<n> type=<n> volume=<n> other=<n>
  asha: <side> status <n>
  asha: <side> volume <n>
  asha: <side> stop frames=<n> lost=<n> bad=<n>
  asha: <side> link down frames=<n> lost=<n> bad=<n>
"lost" counts the gaps of the sequence counter, "bad" the SDUs that are not
161 bytes. Stops on SIGTERM / Ctrl+C.
"""

import argparse
import asyncio
import logging
import os
import struct

import bumble.logging
from bumble.core import AdvertisingData
from bumble.profiles.asha import AshaService, DeviceCapabilities, OpCode
from bumble.transport import open_transport

from le_audio.controller import LeEmuController, LeLocalLink
from le_audio.sink import SinkContext, create_device, power_on_and_advertise

BLUECASTER_ADDRESS = '00:A2:D0:00:00:11'
AID_ADDRESSES = {'left': '00:A2:D0:00:00:22', 'right': '00:A2:D0:00:00:23'}
ASHA_SERVICE_UUID = 0xFDF0
HI_SYNC_ID = bytes.fromhex('a2d0000000000001')   # manufacturer ID (2) + set ID (6), the same on both sides
SDU_BYTES = 161                                  # sequence counter + 160 bytes of G.722 (20 ms)

LAYOUTS = {'pair': ['left', 'right'], 'left': ['left'], 'right': ['right']}

log = logging.getLogger('emu_asha')


def say(side, text):
    print(f'asha: {side} {text}', flush=True)


class Aid:
    """One hearing aid: its ASHA service and what arrived on its channel."""

    def __init__(self, side, sink, binaural, out=None, render_delay_ms=160):
        self.side = side
        self.sink = sink
        self.frames = self.lost = self.bad = 0
        self.expected = None
        self.out = open(f'{out}-{side}.g722', 'wb') if out else None
        capability = (DeviceCapabilities.IS_RIGHT if side == 'right' else 0) | \
                     (DeviceCapabilities.IS_DUAL if binaural else 0)
        self.service = AshaService(capability, HI_SYNC_ID, sink.device, audio_sink=self.on_sdu,
                                   render_delay_milliseconds=render_delay_ms)
        self.capability = capability
        self.service.on(AshaService.EVENT_STARTED, self.on_started)
        self.service.on(AshaService.EVENT_STOPPED, lambda: self.report('stop'))
        self.service.on(AshaService.EVENT_DISCONNECTED, lambda: self.report('link down'))
        self.service.on(AshaService.EVENT_VOLUME_CHANGED, self.on_volume)
        # Bumble's service does not tell a status command (the other side came
        # or went, new connection parameters) apart: watch the writes here
        write = self.service.audio_control_point_characteristic.value.write

        async def on_control_point(connection, value):
            if value and value[0] == OpCode.STATUS and len(value) > 1:
                say(self.side, f'status {value[1]}')
            await write(connection, value)
        self.service.audio_control_point_characteristic.value.write = on_control_point
        sink.device.add_service(self.service)
        # Service data: protocol version, capabilities, the first 4 bytes of the HiSyncId
        sink.adv_structures.append((AdvertisingData.SERVICE_DATA_16_BIT_UUID,
                                    struct.pack('<HBB', ASHA_SERVICE_UUID, 1, capability) + HI_SYNC_ID[:4]))
        sink.adv_structures.append((AdvertisingData.COMPLETE_LIST_OF_16_BIT_SERVICE_CLASS_UUIDS,
                                    struct.pack('<H', ASHA_SERVICE_UUID)))

    def on_started(self):
        service = self.service
        self.frames = self.lost = self.bad = 0
        self.expected = None
        # The volume is a signed byte: -128 (muted) to 0
        say(self.side, f'start codec={int(service.active_codec)} type={int(service.audio_type)} '
                       f'volume={struct.unpack("b", bytes([service.volume]))[0]} other={service.other_state}')

    def on_volume(self):
        say(self.side, f'volume {struct.unpack("b", bytes([self.service.volume]))[0]}')

    def on_sdu(self, data):
        if len(data) != SDU_BYTES:
            self.bad += 1
            return
        sequence = data[0]
        if self.expected is not None and sequence != self.expected:
            self.lost += (sequence - self.expected) & 0xFF
        self.expected = (sequence + 1) & 0xFF
        self.frames += 1
        if self.out:
            self.out.write(data[1:])
            self.out.flush()

    def report(self, what):
        say(self.side, f'{what} frames={self.frames} lost={self.lost} bad={self.bad}')


async def start_aids(link, sides, out=None, capture=None, render_delay_ms=160):
    """The hearing aids of `sides` ('left', 'right') on `link`, powered on and
    advertising (emu_sink.py --asha puts one beside its A2DP sink)."""
    aids = []
    for index, side in enumerate(sides):
        sink = SinkContext(index=index, address=AID_ADDRESSES[side], name=f'BC ASHA {side[0].upper()}',
                           locations=0, sink_ases=0, channel_counts=[])
        side_capture = None
        if capture:
            root, ext = os.path.splitext(capture)
            side_capture = f'{root}-{side}{ext}'
        create_device(link, sink, side_capture)
        aids.append(Aid(side, sink, len(sides) == 2, out, render_delay_ms))
    for aid in aids:
        await power_on_and_advertise(aid.sink)
    return aids


async def main(args):
    link = LeLocalLink()
    transport = await open_transport(f'tcp-server:127.0.0.1:{args.port}')
    LeEmuController('BlueCaster', host_source=transport.source, host_sink=transport.sink, link=link,
                    public_address=BLUECASTER_ADDRESS)
    aids = await start_aids(link, LAYOUTS[args.layout], args.out, args.capture, args.render_delay_ms)
    print(f'READY 127.0.0.1:{args.port} aids={",".join(aid.sink.address for aid in aids)}', flush=True)
    await asyncio.get_running_loop().create_future()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('--port', type=int, default=9001, help='TCP port for BlueCaster (H4)')
    parser.add_argument('--layout', choices=sorted(LAYOUTS), default='pair')
    parser.add_argument('--out', metavar='BASE', help='write the received G.722 to BASE-left.g722 / BASE-right.g722')
    parser.add_argument('--capture', metavar='FILE.pklg',
                        help="PacketLogger captures of the hearing aids' HCI traffic (-left / -right before the extension)")
    parser.add_argument('--render-delay-ms', type=int, default=160,
                        help='RenderDelay of ReadOnlyProperties (default 160)')
    parser.add_argument('--log-level', default='WARNING')
    args = parser.parse_args()
    bumble.logging.setup_basic_logging(args.log_level)
    log.setLevel(logging.INFO)
    asyncio.run(main(args))

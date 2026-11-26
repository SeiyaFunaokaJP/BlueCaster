"""Millisecond timing for the emulators' event loops on Windows.

asyncio's clock there is time.monotonic() = GetTickCount64(), which moves
in 15.6 ms steps, and a timer fires when it is due within that resolution.
An emulated controller that carries audio at fixed intervals then runs in
bursts, up to ~16 ms early or late and sometimes two steps apart:
  - LE Audio: SDU deliveries and their Number Of Completed Packets came in
    bursts (le_audio/controller.py);
  - HFP: the hands-free unit sends a SCO packet every 3.75 ms into a queue
    of 8, and a wait of two steps (31 ms) filled it. A packet was dropped,
    and the tone BlueCaster records jumped (hfp_audio.py).

precise_event_loop_timing() makes the loop's clock the performance counter
and its waits use 1 ms system timer resolution. Call it before the event
loop is created: timers set by the old clock would fire at the wrong time.
"""

import asyncio
import sys
import time

_done = False


def precise_event_loop_timing():
    global _done
    if _done or sys.platform != 'win32':
        return
    _done = True
    import ctypes
    asyncio.BaseEventLoop.time = lambda self: time.perf_counter()
    original_init = asyncio.BaseEventLoop.__init__

    def init(self, *args, **kwargs):
        original_init(self, *args, **kwargs)
        self._clock_resolution = 1e-4
    asyncio.BaseEventLoop.__init__ = init
    try:
        ctypes.windll.winmm.timeBeginPeriod(1)
    except OSError:
        pass

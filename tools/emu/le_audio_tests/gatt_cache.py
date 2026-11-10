"""Kept GATT services (le_gatt_cache.txt) cases and checks of run_test.py
--le-audio (standard library only).

The sink (Bumble) has a Database Hash and drops the link once: BlueCaster
discovers on the first connection and keeps the services with the hash; on
the reconnect it reads the hash again and skips discovery, or, when the
sink added a service in between (emu option --gatt-change,
le_audio/gatt_cache.py), discovers again and finds it.
"""

import re

from .case import Case

STEREO = r'LC3 48000 Hz 10 ms 155 octets, 2 channel\(s\) over 1 CIS'

CASES = [
    Case('gatt-cache', 'stereo', STEREO, sink_args=['--drop-link-after', '3'],
         tags=('gatt_cache',), streams=2, lossy=True, duration=14),
    Case('gatt-cache-changed', 'stereo', STEREO, sink_args=['--gatt-change', '--drop-link-after', '3'],
         tags=('gatt_cache',), streams=2, lossy=True, duration=14),
    # An empty Database Hash (as some earbuds answer): kept for Service Changed
    Case('gatt-cache-no-hash', 'stereo', STEREO, sink_args=['--empty-db-hash', '--drop-link-after', '3'],
         tags=('gatt_cache',), streams=2, lossy=True, duration=14),
    # ... and a changed database: Service Changed on the reconnect drops the
    # kept services and the next attempt discovers (the failed attempt and
    # the wait before the next one take about 5 s of the run)
    Case('gatt-cache-no-hash-changed', 'stereo', STEREO,
         sink_args=['--empty-db-hash', '--gatt-change', '--drop-link-after', '3'],
         tags=('gatt_cache',), streams=2, lossy=True, duration=20, min_audio=0.35),
]


def check(case, ctx):
    if 'gatt_cache' not in case.tags:
        return []
    log, sink_log = ctx['bluecaster_log'], ctx['sink_log']
    failures = []
    lists = [int(n) for n in re.findall(r'LE Audio: \S+: (\d+) services, \d+ characteristics', log)]
    if case.name == 'gatt-cache-no-hash':
        if not re.search(r'no usable Database Hash \(0x00, 0 bytes\): kept, Service Changed tells of changes', log):
            failures.append('the empty Database Hash was not taken as none with Service Changed')
        kept = len(re.findall(r'kept services used \(Service Changed, no Database Hash\), discovery skipped', log))
        if kept != 1:
            failures.append(f'kept services used {kept} time(s), expected once (the reconnect)')
        if len(lists) < 2:
            failures.append(f'{len(lists)} service list(s) in the log, expected one per connection')
        elif lists[1] != lists[0]:
            failures.append(f'the kept list has {lists[1]} services, the discovered one {lists[0]}')
        return failures
    if case.name == 'gatt-cache-no-hash-changed':
        if 'Service Changed indicated' not in sink_log:
            failures.append('the sink did not indicate Service Changed')
        if not re.search(r'kept services used \(Service Changed, no Database Hash\)', log):
            failures.append('the reconnect did not use the kept services')
        if not re.search(r'kept services dropped \(Service Changed\)', log):
            failures.append('Service Changed did not drop the kept services')
        if len(lists) < 3 or lists[-1] != lists[0] + 1:
            failures.append(f'service lists {lists}: no discovery found the added service')
        return failures
    skipped = len(re.findall(r'services unchanged \(Database Hash\), discovery skipped', log))
    if not re.search(r'services kept with their Database Hash', log):
        failures.append('the first connection did not keep the services with their hash')
    if len(lists) < 2:
        failures.append(f'{len(lists)} service list(s) in the log, expected one per connection')
    if case.name == 'gatt-cache':
        if skipped != 1:
            failures.append(f'discovery skipped {skipped} time(s), expected once (the reconnect)')
        elif len(lists) >= 2 and lists[1] != lists[0]:
            failures.append(f'the kept list has {lists[1]} services, the discovered one {lists[0]}')
    else:
        if 'database changed' not in sink_log:
            failures.append('the sink did not change its database')
        if skipped:
            failures.append('discovery was skipped although the database changed')
        if not re.search(r'Database Hash changed \(0x00\), discovering the services', log):
            failures.append('the changed Database Hash was not noticed')
        if len(lists) >= 2 and lists[1] != lists[0] + 1:
            failures.append(f'service lists {lists}: the added service was not found')
    return failures

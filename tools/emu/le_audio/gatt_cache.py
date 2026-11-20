"""A database that changes between links, for BlueCaster's kept services.

Bumble's Generic Attribute service has a Database Hash (Core 5.1 GATT
Caching) on every sink already. With --gatt-change the first sink adds a
service when its first link goes down, which changes the hash: on the
reconnect BlueCaster must discover again and see the new service. It logs

  GATT <sink>: database changed (service 0xFFF0 added)

With --empty-db-hash the Database Hash reads as an empty value (as some
earbuds answer): BlueCaster keeps the services because of Service Changed.
With --gatt-change as well, the sink indicates Service Changed on the
reconnect as soon as BlueCaster subscribes to it, and logs

  GATT <sink>: Service Changed indicated
"""

import asyncio
import logging
import struct

from bumble import gatt

log = logging.getLogger('emu_le_audio')

ADDED = gatt.UUID.from_16_bits(0xFFF0, 'BlueCaster test service')


def add_arguments(parser):
    parser.add_argument('--gatt-change', action='store_true',
                        help='the first sink adds a service when its first link goes down')
    parser.add_argument('--empty-db-hash', action='store_true',
                        help='the Database Hash reads as an empty value')


def configure_sink(sink, args):
    gs = sink.device.gatt_service
    if args.empty_db_hash and gs is not None and gs.database_hash_characteristic is not None:
        gs.database_hash_characteristic.value = gatt.CharacteristicValue(read=lambda _connection: b'')
    if not args.gatt_change or sink.index != 0:
        return
    state = {'changed': False, 'indicated': False}

    if args.empty_db_hash and gs is not None and gs.service_changed_characteristic is not None:
        sc = gs.service_changed_characteristic

        def on_subscription(connection, _notify, indicate):
            if not indicate or not state['changed'] or state['indicated']:
                return
            state['indicated'] = True
            log.info('GATT %s: Service Changed indicated', sink.name)
            asyncio.get_event_loop().create_task(
                sink.device.indicate_subscriber(connection, sc, struct.pack('<HH', 0x0001, 0xFFFF)))

        sc.on('subscription', on_subscription)

    def on_disconnection(_reason):
        if state['changed']:
            return
        state['changed'] = True
        char = gatt.Characteristic(gatt.UUID.from_16_bits(0xFFF1), gatt.Characteristic.Properties.READ,
                                   gatt.Characteristic.READABLE, b'\x01')
        sink.device.add_service(gatt.Service(ADDED, [char]))
        log.info('GATT %s: database changed (service 0xFFF0 added)', sink.name)

    @sink.device.on('connection')
    def on_connection(connection):
        connection.on('disconnection', on_disconnection)

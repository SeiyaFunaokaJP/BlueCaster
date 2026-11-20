---
title: Интеграция (IPC)
layout: default
parent: Главная
nav_order: 2.7
---

# Интеграция (IPC)
{: .no_toc }

Как стороннему инструменту работать с ядром BlueCaster: показывать состояние потока, запускать и останавливать профили, менять громкость или обращаться к собственным службам наушников через Bluetooth-соединение BlueCaster. Инструменты, которые это делают, перечислены на странице [Совместимые инструменты](works-with).

{: .warning }
> Протокол ещё может измениться в ходе бета-версий 2.0.2. Точное описание протокола — [`app/src/crates/ipc/src/protocol.rs`](https://github.com/SeiyaFunaokaJP/BlueCaster/blob/main/app/src/crates/ipc/src/protocol.rs) и сгенерированные из него типы TypeScript в [`app/src/ui/src/ipc/bindings`](https://github.com/SeiyaFunaokaJP/BlueCaster/tree/main/app/src/ui/src/ipc/bindings). Эта страница объясняет, как ими пользоваться, но не перечисляет все поля.

## Содержание
{: .no_toc .text-delta }

1. TOC
{:toc}

## Подключение {#connecting}

Ядро (`bluecaster-core.exe`) работает всё время, пока запущен BlueCaster, с окном или без него. Оно ожидает подключений на именованном канале того пользователя, от имени которого запущено:

```
\\.\pipe\bluecaster-<user SID>
```

- `<user SID>` — строка SID вошедшего в систему пользователя (`S-1-5-21-...`), например из `whoami /user`. У каждого пользователя свои ядро и канал. DACL канала пускает только этого пользователя, а удалённые клиенты отклоняются.
- Канал работает в байтовом режиме. Одновременно может быть подключено несколько клиентов (окно BlueCaster — один из них). Все они получают одно и то же состояние и одни и те же события.
- До 2.0.x канал назывался `\\.\pipe\a2dpwb-<user SID>`. BlueCaster 3.0.0 принимает подключения по **обоим** именам и сохранит старое имя ещё как минимум на 6 месяцев после 3.0.0. Клиенту следует сначала пробовать `bluecaster-<user SID>`, а если не получится — старое имя. Версия протокола остаётся 1.
- Если канала нет, ядро не запущено. Проверяйте это по каналу, а не по мьютексу ядра.
- Сборки для разработки, запущенные с `BLUECASTER_INSTANCE=<name>`, ожидают подключений на `\\.\pipe\bluecaster-<SID>-<name>`, поэтому могут работать одновременно с установленной версией.

### Кадры

Каждое сообщение в обоих направлениях — один кадр: длина JSON в виде **u32 little-endian**, затем сам JSON (UTF-8). Кадры больше 4 MiB отклоняются.

### Рукопожатие

1. Клиент отправляет `hello` в течение 5 секунд после подключения:
   ```json
   {"type": "hello", "protocol": 1, "client": "mytool/1.2.0"}
   ```
   `client` — любое имя и версия на ваш выбор; они попадут в журнал ядра.
2. Ядро отвечает `welcome` либо отвечает `reject` и закрывает канал (неверная версия протокола или hello не пришёл):
   ```json
   {"type": "welcome", "protocol": 1, "server": "bluecaster-core/2.0.2-beta.12", "features": ["rfcomm"]}
   {"type": "reject", "reason": "protocol 2 not supported (core speaks 1)"}
   ```
3. Ядро отправляет `snapshot` со всем своим состоянием, затем события `event`.

Версия протокола должна совпадать точно. Обратно совместимые дополнения вместо этого объявляются в `features` (см. [Совместимость](#compatibility)).

```mermaid
sequenceDiagram
    participant C as Client
    participant B as bluecaster-core
    C->>B: hello (protocol 1)
    alt протокол совпадает
        B-->>C: welcome (features)
        B-->>C: snapshot (всё состояние, rev)
        loop Пока есть подключение, в любом порядке
            B-->>C: event (rev + 1)
            C->>B: request (id)
            B-->>C: response (тот же id)
        end
        B-->>C: bye (ядро завершает работу)
    else неверный протокол или нет hello за 5 с
        B-->>C: reject, канал закрыт
    end
```

## Состояние и события {#state-and-events}

```json
{"type": "snapshot", "rev": 41, "state": {"adapter": {...}, "devices": [...], "stream": {...}, "profiles": [...], "active_profile": "WH-1000XM5", "settings": {...}, ...}}
{"type": "event", "rev": 42, "event": {"kind": "stream", "value": {"state": "streaming", "profile": "WH-1000XM5", "info": {"codec": "ldac", "bitrate_kbps": 990, ...}, "volume": 80, ...}}}
```

Клиенту не нужно собственное состояние: достаточно хранить последний снимок и применять к нему каждое событие. Каждое событие состояния **целиком заменяет одну часть** снимка:

| Событие `kind` | Заменяет |
|:--|:--|
| `adapter` | `adapter` |
| `devices` | `devices` |
| `scanning`, `le_scanning` | `scanning`, `le_scanning` |
| `stream` | `stream` (состояние, сообщение, профиль, сведения о кодеке, громкость, заряд батареи) |
| `profiles` | `profiles` и `active_profile` |
| `settings` | `settings` |
| `update` | `update` |
| `capture` | `capture` |
| `v1_running`, `verbose`, `crashes` | одноимённое поле |

`stream.state` принимает одно из значений `idle`, `connecting`, `streaming`, `reconnecting`, `error`. `stream.message` сообщает причину в виде [ключа сообщения](#errors-and-messages).

**Ревизии.** Каждое событие состояния содержит следующий номер ревизии (`rev` + 1). Пропуск в номерах означает, что событие потеряно: отправьте `state.snapshot`, чтобы получить новый снимок. Если клиент читает слишком медленно, ядро само отправит ему новый `snapshot`. События, не относящиеся к состоянию (`link`, `log`, `receiver`, `rfcomm_data`, `rfcomm_closed`), содержат текущую ревизию и не увеличивают её.

Завершая работу, ядро отправляет событие `bye` и закрывает канал.

## Запросы {#requests}

```json
{"type": "request", "id": 7, "method": "stream.start", "params": {"profile": "WH-1000XM5"}}
{"type": "response", "id": 7, "result": null, "error": null}
```

- `id` — любое число на выбор клиента; ответ содержит тот же `id`. Запросы выполняются параллельно, поэтому ответы могут приходить в другом порядке.
- Для методов без параметров `params` не указывается.
- На запрос, который меняет состояние, ядро отвечает после завершения изменения, а само изменение также приходит событием. Показывайте состояние по событиям, а не по ответу.

### Ошибки и сообщения {#errors-and-messages}

```json
{"type": "response", "id": 8, "result": null, "error": {"code": "rfcomm.not_connected", "message": {"key": "ipc.error.rfcomm_not_connected", "args": []}}}
```

`code` предназначен для программ (`adapter.driver_missing`, `app.streaming`, ...). `message` — ключ перевода из языковых файлов BlueCaster ([`app/lang`](https://github.com/SeiyaFunaokaJP/BlueCaster/tree/main/app/lang)) с аргументами в стиле printf, как и тексты состояния в снимке. Инструмент может найти ключ там или показать для кода свой текст.

### Методы {#methods}

Методы для инструментов:

| Метод | Параметры | Результат |
|:--|:--|:--|
| `state.snapshot` | -- | `Snapshot` |
| `stream.start` | `profile` | `null`; сначала останавливает уже идущий поток |
| `stream.stop` | -- | `null` |
| `stream.set_volume` | `volume` (абсолютная громкость AVRCP 0-127) | `null` |
| `profile.set_active` | `name` | `null` |
| `device.scan` | `seconds` (3-600) | `Device[]`, также в виде события `devices` |
| `device.scan_stop` | -- | `null`; после этого завершается идущий `device.scan` |
| `audio.devices` | -- | `AudioDevice[]` (источники захвата) |
| `diag.subscribe` | `logs` (уровень или `null`), `link` (bool), `receiver` (bool, необязательно) | `null`; включает или выключает события `log` / `link` / `receiver` для этого подключения |
| `diag.info` | `{"mask": bool}` (необязательно) | `DiagInfo` (текст «Копировать диагностику») |
| `rfcomm.open`, `rfcomm.send`, `rfcomm.close` | см. [Сквозной канал RFCOMM](#rfcomm-passthrough) | |

Методы окна BlueCaster, которые инструменту стоит вызывать с осторожностью: `profile.save` / `profile.delete`, `settings.set` (заменяет **все** настройки: отправляйте последние полученные `settings` со своим изменением, а не частичный объект), `device.forget` / `device.rename`, `adapter.*` (`adapter.assign_winusb` и `adapter.restore_driver` показывают запрос UAC; `adapter.probe` и `adapter.run_script` — инструменты для чипов из режима разработчика — останавливают стек Bluetooth), `app.quit`. Остальные методы обслуживают собственные экраны окна и могут измениться без предупреждения: `stream.start_custom`, прочие `device.*` (`device.import_windows_pairings` показывает запрос UAC), `audio.apps`, `audio.mixer` / `audio.mixer_set`, `diag.*`, кроме двух указанных выше, `receiver.*` и `app.*`. Полный список с параметрами и результатами — перечисление `Method` в `protocol.rs`.

## Сквозной канал RFCOMM {#rfcomm-passthrough}

Доступен, если `welcome.features` содержит `"rfcomm"` (2.0.2-beta.10 и новее). Через уже установленное соединение BlueCaster инструмент может обращаться к собственным службам RFCOMM наушников (их используют фирменные приложения: заряд, шумоподавление, эквалайзер). Сама Windows наушники, подключённые через BlueCaster, не видит.

- `rfcomm.open {address, uuid}` → `{channel_id, mtu}`. `address` — адрес устройства, на которое сейчас идёт передача. Ради этого ядро не подключается и не выполняет сопряжение (`rfcomm.not_connected`). `uuid` — полный 128-битный UUID или 16-битный `"xxxx"` на основе Bluetooth Base UUID. Ответ приходит в течение 20 с, даже если запрос стоит в очереди за другими открытиями, поэтому ждите не менее 25 с. Ошибки: `rfcomm.not_connected`, `rfcomm.service_not_found`, `rfcomm.open_failed`, `rfcomm.busy`.
- `rfcomm.send {channel_id, data}` → `null`. `data` — base64 (стандартный, с дополнением), не более 64 KiB на запрос. Данные отправляются кадрами не больше `mtu` в порядке запросов. Ошибки: `rfcomm.no_channel`, `rfcomm.busy` (очередь заполнена, не отправлено), `rfcomm.timeout` (неизвестно, отправлены ли данные: не повторяйте запрос, а откройте канал заново).
- Принятые данные приходят событиями `rfcomm_data` `{channel_id, data}` только тому подключению, которое открыло канал.
- `rfcomm.close {channel_id}` → `null`, когда данные, отправленные до него, уйдут. Событие `rfcomm_closed` после этого не приходит.
- Канал RFCOMM также закрывается, когда завершается подключение к именованному каналу, или с событием `rfcomm_closed` `{channel_id, reason}`: `remote` (устройство закрыло его или соединение разорвалось), `disconnected` (подключение A2DP завершилось), `stopped` (стек Bluetooth остановлен), `overflow` (клиент оставил непрочитанными более 1 MiB принятых данных).

## Совместимость {#compatibility}

- **Возможности.** Проверяйте `welcome.features`, прежде чем использовать дополнение. Ядра до 2.0.2-beta.10 не передают это поле.
- **Неизвестные запросы.** На запрос, который ядро не может разобрать (неизвестный метод, неверные параметры), приходит ошибка `ipc.bad_request`, а подключение остаётся открытым. Ядра до 2.0.2-beta.10 вместо этого закрывают подключение. Любой другой нечитаемый кадр закрывает его.
- **Новые поля.** Со временем ядро добавляет поля в состояние и в события. Игнорируйте незнакомые поля и не отклоняйте из-за них сообщение.
- **Переименование в 3.0.0** ([#193](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/193)): канал становится `\\.\pipe\bluecaster-<SID>`. Не менее 6 месяцев после 3.0.0 ядро продолжит принимать подключения и на `\\.\pipe\a2dpwb-<SID>` с тем же протоколом. Об удалении старого имени будет заранее объявлено в заметках к выпуску. Сначала пробуйте `bluecaster-<SID>`, а если не получится — `a2dpwb-<SID>`. Мьютекс ядра становится `Local\BlueCaster_Core`. В течение того же срока ядро удерживает и `Local\A2DPWB_v2_Core`; об удалении старого канала и старого мьютекса будет объявлено одновременно.

## Пример {#example}

Минимальный клиент на Python (только стандартная библиотека), который выводит состояние потока и следит за его изменениями:

```python
import json, struct, subprocess

sid = subprocess.run(["whoami", "/user", "/fo", "csv", "/nh"],
                     capture_output=True, text=True).stdout.strip().split(",")[1].strip('"')
pipe = open(rf"\\.\pipe\bluecaster-{sid}", "r+b", buffering=0)

def send(msg):
    b = json.dumps(msg).encode()
    pipe.write(struct.pack("<I", len(b)) + b)

def read_exact(n):
    buf = b""
    while len(buf) < n:
        chunk = pipe.read(n - len(buf))
        if not chunk:
            raise EOFError
        buf += chunk
    return buf

def recv():
    (n,) = struct.unpack("<I", read_exact(4))
    return json.loads(read_exact(n))

send({"type": "hello", "protocol": 1, "client": "example/0.1"})
welcome = recv()
assert welcome["type"] == "welcome", welcome
snap = recv()
print("stream:", snap["state"]["stream"]["state"])
while True:
    m = recv()
    if m["type"] == "event" and m["event"]["kind"] == "stream":
        s = m["event"]["value"]
        print("stream:", s["state"], (s.get("info") or {}).get("codec"))
    elif m["type"] == "event" and m["event"]["kind"] == "bye":
        break
```

Чтобы проверить инструмент, не затрагивая установленный BlueCaster, запустите сборку для разработки с заданными `BLUECASTER_INSTANCE` и `BLUECASTER_CONFIG_DIR` (см. [Сборка](building)) и подключитесь к её каналу.

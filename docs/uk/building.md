---
title: Збирання
layout: default
parent: Головна
nav_order: 3
---

# Збирання з вихідного коду
{: .no_toc }

## Зміст
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Вимоги {#requirements}

- **Rust** (stable, 1.90 або новіший) через [rustup](https://rustup.rs/)
- **Visual Studio 2022 або новіша** (або її Build Tools) з робочим навантаженням «Desktop development with C++», бо бібліотеки C (BTstack, кодеки) компілює MSVC
- **Git** (для підмодулів)
- **PowerShell 7** (`pwsh`) для `build.ps1` / `build.bat` (`winget install Microsoft.PowerShell`)
- **Python 3.11 або новіший** для перевірок (`tools/check_lang.py`) і [тесту з емулятором](#emu)
- Для збірок ARM64: компоненти Visual Studio «MSVC v143 - VS 2022 C++ ARM64 build tools» і, на ПК x64, «C++ Clang Compiler for Windows» (`ring` із rustls асемблює свій код ARM64 за допомогою clang; `installer/build-release.ps1` автоматично знаходить clang із Visual Studio), а також `rustup target add aarch64-pc-windows-msvc`

## Швидке збирання {#quick-build}

```bash
git clone --recursive https://github.com/SeiyaFunaokaJP/BlueCaster.git
cd BlueCaster
pwsh ./build.ps1
```

Або двічі клацніть `build.bat` у Провіднику: він запускає `build.ps1` і наприкінці чекає натискання клавіші (`build.bat -Test` передає ключі далі). `build.ps1` спершу перевіряє, чи є Rust і підмодулі, а потім збирає три програми в `target/release/`: `bluecaster-core.exe` (резидентне ядро: Bluetooth, звук, область сповіщень), `bluecaster.exe` (вікно) і `bluecaster-driver.exe` (допоміжна програма з правами адміністратора, яка встановлює й відновлює драйвер WinUSB). Тримайте всі три в одній папці; `bluecaster-core.exe --cli` передає звук без вікна.

Ключі: `-Debug` (профіль dev, `target/debug/`), `-Test` (додатково запускає перевірки з розділу «Тести»), `-Decoder` (додатково збирає інструмент декодування), `-Emu` (додатково запускає [тест з емулятором](#emu); `-Emu -Codecs sbc,ldac` перевіряє лише ці кодеки), `-Release -Arch x64|arm64` (запускає лише скрипт випуску, див. «Пакети випуску»).

**Без скрипту** (що запускає `build.ps1`):

```bash
cargo build --release -p bluecaster-core-exe -p bluecaster-driver -p bluecaster-ui
```

## Якщо ви вже клонували без `--recursive`

```bash
git submodule update --init --recursive
```

Виконайте цю команду знову після pull, який оновлює бібліотеку. Якщо бібліотека в `extern/` не на коміті, указаному в `extern/versions.txt`, збирання зупиняється, а в повідомленні про помилку пояснено, як усе ж зібрати іншу версію.

## Структура проєкту {#project-structure}

```
BlueCaster/
├── app/
│   ├── src/
│   │   ├── core/           bluecaster-core.exe: resident core, tray, IPC server, --cli
│   │   ├── ui/             bluecaster.exe: the window and the tray flyout (GPUI)
│   │   ├── driver/         bluecaster-driver.exe: WinUSB assign / restore (elevated)
│   │   └── crates/         Libraries: btstack-sys, hci-transport, bt-host, codec, the codec -sys crates,
│   │                       pipeline, audio-capture, win-sys, core (bluecaster-core), ipc, le-audio, lc3-sys,
│   │                       aptx-adaptive (aptX Adaptive encoder), ui-model and ui-host (the window's logic)
│   ├── lang/               Localization (en.json is the source; tools/check_lang.py checks the others)
│   └── resources/          Icon and manifest
├── Cargo.toml              Cargo workspace
├── installer/              Release script, Inno Setup script, updater notes (installer/README.md)
├── compat/                 MSVC compatibility headers for the AOSP code
├── docs/                   Documentation (this site)
├── extern/                 Third-party C libraries (git submodules)
│   ├── btstack/            BTstack Bluetooth stack
│   ├── libldac/            AOSP LDAC encoder
│   ├── libopenaptx/        aptX / aptX HD / aptX LL encoder
│   ├── fdk-aac/            Fraunhofer AAC encoder
│   ├── lhdcv5-enc/         LHDC V5 encoder
│   └── opus/               libopus (Opus encoder)
└── tools/
    ├── a2dp_decode/        bluecaster_decode: checks / decodes the media stream in an HCI capture (.pklg)
    ├── emu/                End-to-end test against a virtual Bluetooth sink (Python)
    └── linux_sink/         bluecaster_sink: measuring A2DP receiver on Linux (Python, Bumble)
```

## Тести {#tests}

```bash
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
python tools/check_lang.py
```

`cargo test --workspace` перевіряє й вікно: `ui-model` містить правила екранів, переклади й форматування, і для його тестів вікно не потрібне.

Щоб переглянути екрани без ядра й адаптера, запустіть `bluecaster.exe`, указавши в `BLUECASTER_UI_DEMO` шлях до JSON-файлу зі знімком стану ядра. `BLUECASTER_UI_ROUTE` відкриває один екран, а `BLUECASTER_UI_SIZE` задає розмір вікна. Формат файлу описано в `app/src/ui/src/demo.rs`.

## Пакети випуску {#release-packages}

```powershell
pwsh installer/build-release.ps1 -Arch x64
```

записує в `target/dist` інсталятор (`BlueCaster-<version>-win64-setup.exe`), портативний ZIP-архів, ZIP-архів символів (файли `.pdb` для аварійних дампів) і маніфест оновлювача. Потрібен Inno Setup 7.1.0 (`winget install JRSoftware.InnoSetup.7 --version 7.1.0`). Про підписування оновлень у програмі та інші подробиці див. `installer/README.md`.

## Тестування без обладнання (tools/emu) {#emu}

`tools/emu` передає кожен кодек із BlueCaster на віртуальний Bluetooth-приймач на тому самому ПК — без адаптера й навушників. Він працює лише у Windows, а для інструмента декодування потрібні **Python 3.11 або новіший** і **CMake** (входить до Visual Studio).

```bash
python tools/emu/setup_env.py                       # once: creates tools/emu/.venv with pinned packages
cmake -S tools/a2dp_decode -B build/a2dp_decode     # once: the decoder tool
cmake --build build/a2dp_decode --config Release
cargo build --release -p bluecaster-core-exe
python tools/emu/run_test.py                        # every codec and negotiation case
```

Як це працює:

- `emu_sink.py` запускає два віртуальні контролери [Bumble](https://github.com/google/bumble), з'єднані всередині процесу. BlueCaster підключається до одного з них як H4 через TCP (`127.0.0.1`). Другий використовує A2DP-приймач Bumble: він пропонує SBC (з підтримкою SBC XQ), AAC, aptX, aptX HD, aptX LL, LDAC, LHDC V5, FastStream, Opus, Opus 05 і aptX Adaptive та записує свій трафік HCI.
- `run_test.py` запускає `bluecaster-core.exe --cli` для кожного кодека з параметрами для розробки: `--hci-tcp` (віртуальний контролер замість WinUSB), `--test-tone` (1 kHz ліворуч / 1,5 kHz праворуч замість системного звуку), `--duration`, `--hci-capture`. Змінна середовища `BLUECASTER_CONFIG_DIR` спрямовує BlueCaster в окрему папку конфігурації, тож сполучення з віртуальним приймачем ніколи не зачіпає ваших справжніх ключів зв'язку.
- Потім обидва записи HCI перевіряє `bluecaster_decode` (бік приймача — з ключем `--received`).

Кодек проходить тест, якщо виконано всі умови:

- узгоджено саме той кодек, який запитано
- `bluecaster_decode` не знаходить проблем ні на боці надсилання, ні на боці приймання
- приймач отримав кожен медіапакет і кожен кадр, надісланий BlueCaster, і надіслано щонайменше 90 % запитаної тривалості звуку
- декодований звук однаковий на обох боках і збігається з тестовим тоном (крім LDAC, LHDC V5 і aptX Adaptive, для яких немає декодера з відкритим кодом)

| Параметр | Опис |
|:-------|:------------|
| `--codecs sbc aac ...` | Перевірити лише ці кодеки (`sbc sbcxq aac aptx aptxhd aptxll ldac lhdcv5 faststream opus opus05`) або випадки узгодження (частоти дискретизації, низькі bitpool, конфігурації кадрів SBC, aptX Adaptive, ...; `-h` виводить їх список); за замовчуванням: усі |
| `--le-audio [case ...]` | Натомість виконати випадки LE Audio (за замовчуванням: усі) |
| `--auracast [case ...]` | Натомість виконати випадки Auracast (за замовчуванням: усі) |
| `--scan [case ...]` | Натомість виконати випадки пошуку пристроїв (`eir`, `name-request`) |
| `--link [case ...]` | Натомість виконати випадки з'єднання (`disconnect`, `device-link`, `device-a2dp`, `device-stream`, `restart-on-link`) |
| `--duration <s>` | Тривалість передавання для кожного кодека, с (за замовчуванням 5) |
| `--max-packet <bytes>` | Передається BlueCaster (максимальний розмір медіапакета, див. [Використання](usage)) |
| `--bluecaster <exe>` | Ядро для перевірки (за замовчуванням `target/release/bluecaster-core.exe`) |
| `--decoder <exe>` | Інструмент декодування (за замовчуванням `<build-dir>/a2dp_decode/<config>/bluecaster_decode.exe`) |
| `--build-dir <dir>`, `--config <cfg>` | Де зібрано інструмент декодування (за замовчуванням `build`, `Release`) |
| `--base-port <port>` | Перший порт TCP (за замовчуванням 9101; випадки LE Audio використовують +100); змініть його, щоб запускати кілька тестів одночасно |
| `--out <dir>` | Вихідна папка (за замовчуванням `tools/emu/out`) |

Для кожного кодека в `tools/emu/out/<codec>/` зберігаються записи (`bluecaster.pklg`, `sink.pklg`), журнали, звіти `bluecaster_decode` і декодовані файли WAV. Код виходу дорівнює 0, якщо всі кодеки пройшли тест.

{: .note }
Віртуальне з'єднання не має радіо: воно перевіряє кодування, пакетизацію, сигналізацію AVDTP / L2CAP і сумісність із незалежним стеком Bluetooth (Bumble), але не умови радіоканалу, таймінги на справжньому обладнанні чи те, як справжні навушники відтворюють звук. Пакети Bumble — лише засоби тестування й не входять до BlueCaster (див. THIRD_PARTY_LICENSES.md).

`emu_vhci.py` робить те саме для стеку Bluetooth Linux: у Linux він під'єднує до BlueZ віртуальний адаптер через `/dev/vhci`, і вимірювальний приймач `tools/linux_sink` використовує його як справжній. Так приймач можна тестувати без обладнання, за потреби із втраченими або затриманими медіапакетами (`--drop`, `--stall`). Див. `tools/linux_sink/README.md`.

## Залежності {#dependencies}

Бібліотеки C підключено як підмодулі git; Rust-крейти завантажує Cargo (версії зафіксовано в `Cargo.lock`; `cargo deny check` перевіряє їхні ліцензії).

| Бібліотека | Спосіб | Зафіксована версія | Ліцензія |
|:--------|:-------|:---------------|:--------|
| BTstack | підмодуль git | v1.8.3 | BTstack License (у стилі BSD-3-Clause з умовою щодо некомерційного використання) |
| Кодек SBC з Bluedroid | кодувальник перенесено на Rust; декодер (тести, інструменти) — з копії в BTstack | як у BTstack | Apache-2.0 |
| rijndael | входить до BTstack (`3rd-party/rijndael`) | як у BTstack | Суспільне надбання |
| liblc3 (lc3-google) | входить до BTstack (`3rd-party/lc3-google`) | як у BTstack | Apache-2.0 |
| micro-ecc | входить до BTstack (`3rd-party/micro-ecc`) | як у BTstack | BSD-2-Clause |
| libldac (AOSP) | підмодуль git | android-15.0.0_r36-4-geeee1a3 | Apache-2.0 |
| libopenaptx | підмодуль git | 0.2.0 (не оновлювати) | LGPL-2.1+ |
| fdk-aac | підмодуль git | v3.0.0 | FDK AAC License |
| Кодувальник LHDC V5 | підмодуль git | як зафіксовано | Apache-2.0 |
| libopus | підмодуль git | v1.6.1 | BSD-3-Clause |
| GPUI, Rust-крейти | Cargo | `Cargo.lock` | MIT / Apache-2.0 та інші (`licenses/third-party/rust-crates.txt` у випуску) |

{: .warning }
Оскільки BTstack скомпоновано з програмою, будь-яку зібрану вами копію BlueCaster можна використовувати й поширювати лише в особистих некомерційних цілях, хоча сам вихідний код BlueCaster поширюється за подвійною ліцензією Apache-2.0 або MIT.

Повні відомості про ліцензії див. у [THIRD_PARTY_LICENSES.md](https://github.com/SeiyaFunaokaJP/BlueCaster/blob/main/THIRD_PARTY_LICENSES.md).

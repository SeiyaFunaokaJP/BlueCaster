---
title: Geprüfte Hardware
layout: default
parent: Startseite
nav_order: 2.5
---

# Geprüfte Hardware
{: .no_toc }

Adapter und Kopfhörer, die bereits mit BlueCaster im Einsatz waren – aus den Tests des Maintainers und aus Nutzerberichten in den Issues. ✅ funktioniert · ⚠️ funktioniert mit einer Einschränkung oder teilweise · ❌ funktioniert nicht · ❓ nicht bestätigt.

Haben Sie BlueCaster mit anderer Hardware ausprobiert? Berichten Sie bitte in einem [Issue](https://github.com/SeiyaFunaokaJP/BlueCaster/issues) davon (bei Codecs in [#11](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/11)) und fügen Sie den Text aus **Diagnose > Diagnose kopieren** ein. Er nennt Adapter, Codec und Bitrate.

## Inhalt
{: .no_toc .text-delta }

1. TOC
{:toc}

## Bluetooth-Adapter

| Adapter | Chip | USB-ID | Art | Ergebnis | Hinweise | Quelle |
|:--|:--|:--|:--|:--|:--|:--|
| TP-Link UB500 | Realtek RTL8761BU | 2357:0604 | USB-Dongle | ✅ Am besten getestet | Firmware `rtl8761bu`. Kein LE Audio (keine isochronen Kanäle) | Maintainer; [#84](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/84), [#87](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/87) |
| Generischer RTL8761B-Dongle | Realtek RTL8761BU | 0BDA:A729 | USB-Dongle | ✅ | Firmware `rtl8761bu`. Seit 2.0.2 automatisch erkannt (davor: RTL8761BU als Chip wählen). Kein LE Audio | [#66](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/66), [#86](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/86) |
| Eingebauter Realtek RTL8852BE eines Notebooks | Realtek RTL8852BU | 0BDA:4853 | Eingebaut | ✅ | Firmware `rtl8852bu`, seit 2.0.2-beta.1 erkannt. Solange BlueCaster ihn verwendet, hat Windows kein eigenes Bluetooth | [#81](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/81) |
| ASUS BT400 | Broadcom BCM20702 | 0B05:17CB | USB-Dongle | ✅ | Läuft mit der Firmware im ROM, keine Dateien nötig. Gemeldet unter v1 (1.0.12–1.0.17) | [#18](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/18), [#27](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/27), [#30](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/30) |
| Dongle „Realtek Bluetooth 6.0“ | Realtek RTL8761B | 0BDA:A760 | USB-Dongle | ✅ | Noch nicht automatisch erkannt (LMP-Subversion 0x8761). Kein LE Audio | [#208](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/208) |
| CSR8510-A10-Dongle | CSR8510 | 0A12:0001 | USB-Dongle | ✅ | Keine Firmware-Dateien nötig | [#32](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/32) |
| Intel AX211 (eingebaute WLAN- / Bluetooth-Karte) | Intel AX211 (TLV-Bootloader) | 8087:0033 | Eingebaut | ✅ | Firmware aus `intel/` von linux-firmware, vom Bootloader aus in 1,5–1,7 s geladen: `ibt-0180-0041` (Variante 0x1B) oder `ibt-1040-0041` (Variante 0x19). Welche Datei es ist, steht im Protokoll. Kopplung, AAC und LDAC funktionieren. Solange BlueCaster sie verwendet, hat Windows kein eigenes Bluetooth | [#261](https://github.com/SeiyaFunaokaJP/BlueCaster/pull/261), [#237](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/237) |
| Intel AX200 (eingebaute WLAN- / Bluetooth-Karte) | Intel AX200 | 8087:0029 | Eingebaut | ✅ | Im Bericht lief die Firmware bereits, daher wurde keine Datei geladen. LHDC V5 1000 kbps / 192 kHz funktioniert. Solange BlueCaster sie verwendet, hat Windows kein eigenes Bluetooth | [#237](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/237) |
| MediaTek MT7925 (eingebaute WLAN- / Bluetooth-Karte) | MediaTek MT7925 | 0E8D:7925 | Eingebaut | ✅ | Firmware aus dem Windows-Treiber von MediaTek gelesen. A2DP funktioniert. LE Audio (experimentell) lief nur in einigen Tests und funktioniert noch nicht zuverlässig | [#86](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/86) |
| MediaTek MT7921 (eingebaute WLAN- / Bluetooth-Karte) | MediaTek MT7921 | 13D3:3606 | Eingebaut | ✅ | – | [#432](https://github.com/SeiyaFunaokaJP/BlueCaster/pull/432) |
| Adapter „MA530“ | Nicht angegeben | – | USB-Dongle | ✅ | Gemeldet unter v1.0.9 | [#3](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/3), [#9](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/9) |
| UGREEN-Dongles Bluetooth 5.4 und 6.0 | Nicht angegeben | – | USB-Dongle | ✅ | Gemeldet unter 2.0.1 | [#11](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/11#issuecomment-6563908711) |

Zu Qualcomm-Adaptern gibt es noch keine Berichte. Was Sie von ihnen erwarten können, steht unter [Empfohlene Adapter](setup#adapters).

## Kopfhörer

| Kopfhörer | Adapter | Codecs | Hinweise | Version | Quelle |
|:--|:--|:--|:--|:--|:--|
| Sony WH-1000XM4 | TP-Link UB500 | ✅ LDAC 990 kbps (bis 96 kHz) · ✅ AAC · ✅ SBC · ✅ SBC XQ 552 kbps | Kein aptX bei diesem Modell. Bei eingeschaltetem Multipoint ist LDAC nicht verfügbar | 1.0 – 2.0 | Maintainer |
| Sony WH-1000XM4 | ASUS BT400 | ✅ LDAC 990 kbps, 48 kHz / 24 Bit | Windows 10 22H2 | 1.0.12, 1.0.15 | [#18](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/18), [#27](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/27) |
| Sony WH-1000XM6 | Eingebauter RTL8852BU | ✅ LDAC 990 kbps, 48 kHz / 24 Bit (5 Minuten, 0 Pakete verloren) | LE Audio an den Kopfhörern ausgeschaltet. Für die erste Verbindung ist der Kopplungsmodus nötig | 2.0.1, 2.0.2-beta.1 | [#81](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/81) |
| EarFun Air Pro 4+ | RTL8761B (0BDA:A729) | ✅ LDAC · ✅ AAC · ✅ SBC · ✅ SBC XQ · ✅ aptX · ✅ aptX Adaptive | | 2.0.2-beta.1, 2.0.2-beta.6 | [#86](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/86) |
| Grado GW100x | „MA530“ | ✅ aptX | Bietet auch aptX Adaptive an; 1.0.9 hatte keinen aptX-Adaptive-Encoder und sendete klassisches aptX (aptX Adaptive daran noch nicht getestet) | 1.0.9 | [#9](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/9) |
| LHDC-V5-Kopfhörer (Modell nicht angegeben) | RTL8761B | ✅ LHDC V5 1000 kbps / 192 kHz | In der App der Kopfhörer den 192-kHz-Modus wählen. Im 96-kHz-Modus akzeptieren sie nur 400 kbps | 1.1.1-beta.1 | [#50](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/50) |
| OPPO Enco Free4 | Intel AX200 | ✅ LHDC V5 1000 kbps / 192 kHz (10 Minuten, 0 Pakete verloren) | Ohne Hi-Res bieten sie höchstens 48 kHz und 400 kbps an, selbst wenn Hi-Res in ihrer App eingeschaltet ist. BlueCaster fordert Hi-Res an und verbindet sich neu. Danach bieten sie bis zu 192 kHz ohne Bitratengrenze an. Die einmalige Anforderung aus 2.0.2-beta.18 ist an diesem Modell noch nicht bestätigt | 2.0.2-beta.17 | [#237](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/237) |
| OPPO Enco X4 | Intel AX211 | ⚠️ LHDC V5 1000 kbps | Mit eingeschaltetem Hi-Res in ihrer App boten sie bis zu 192 kHz ohne Bitratengrenze an, BlueCaster musste also nichts anfordern. Bei 1000 kbps gingen auf dieser Verbindung Pakete verloren (20 von 79 Kanälen nutzbar, wahrscheinlich wegen WLAN in der Nähe). Standard 500 kbps ist über eine längere Sitzung noch nicht bestätigt | 2.0.2-beta.18 | [#237](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/237) |
| JBL Tune 510BT | TP-Link UB500 | ✅ SBC · ⚠️ AAC (stumm) | Die AAC-Korrektur ([#89](https://github.com/SeiyaFunaokaJP/BlueCaster/pull/89)) ist an diesem Modell noch nicht bestätigt | 2.0.1 | [#43](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/43), [#84](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/84), [#87](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/87) |
| Huawei FreeBuds Pro 4 | ASUS BT400 | ✅ AAC 256 kbps · ❓ LDAC | Unter 1.0.16 boten sie nur SBC und AAC an. Ist LDAC gewählt (automatische Auswahl aus), schaltet BlueCaster sie jetzt in den Modus „Klangqualität zuerst“ und verbindet sich neu. Wird LDAC danach immer noch nicht angeboten, stellt BlueCaster den Modus zurück. An diesen Ohrhörern noch nicht bestätigt. L2HC ist proprietär | 1.0.16 | [#30](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/30) |
| Huawei FreeBuds 5i | RTL8761B (0BDA:A760) | ✅ LDAC 990 / 660 kbps · ✅ AAC 256 kbps · ✅ SBC | Boten LDAC nur bei manchen Verbindungen an. Bieten außerdem einen eigenen Huawei-Codec (0x027D/0x4C36) an, den BlueCaster nicht unterstützt | 2.0.1 | [#208](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/208) |
| Huawei FreeBuds Neo | RTL8761B (0BDA:A760) | ✅ LDAC 990 kbps · ✅ AAC 256 kbps · ✅ SBC | Unter 2.0.1 boten sie nur SBC und AAC an. Unter 2.0.2-beta.13 funktioniert LDAC, sobald LDAC im Profil gewählt ist (automatische Auswahl aus). In einem stark belegten 2,4-GHz-WLAN setzte der Ton aus, bis AFH auf Manuell mit dem Kanal des Routers gestellt war | 2.0.2-beta.13 | [#208](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/208), [#30](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/30) |
| EDIFIER W820NB (Dual Gold Label) | CSR8510 A10 | ✅ SBC · ❓ LDAC | LDAC schlug unter 1.0.16 fehl und wurde in 1.0.17 behoben. Seitdem nicht erneut bestätigt | 1.0.16 | [#32](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/32) |
| ROSESELSA Ceramics Ultra | UGREEN Bluetooth 5.4 / 6.0 | ✅ LDAC · ⚠️ LHDC V5 · ✅ AAC · ✅ SBC | LDAC lief mit der stabilsten Verbindungsqualität und Bitrate. LHDC V5 kann sporadisch aussetzen | 2.0.1 | [#11](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/11#issuecomment-6563908711) |

## Testausrüstung

| Was | Aufbau | Codecs | Ergebnis |
|:--|:--|:--|:--|
| Messempfänger [`tools/linux_sink`](usage#receiver) | TP-Link UB500 → Ubuntu 26.04 (Intel AX201) über Funk | SBC, AAC, aptX, aptX HD, aptX LL, LDAC | ✅ Ohne Verluste empfangen |
| Virtuelle Verbindung `tools/emu` (Bumble) | Kein Funk | Jeder Codec und LE Audio | ✅ End-to-End-Test bestanden |

## Noch nicht geprüft

- **Codecs an echten Kopfhörern**: aptX HD, aptX Low Latency, Opus, Opus 05, FastStream
- **LE Audio** (experimentell, nur im Entwicklermodus): funktioniert mit dem Emulator, auf echten Adaptern aber noch nicht zuverlässig (getestet am MediaTek MT7925)
- **Windows auf ARM64** und **Windows 10 mit v2** (v1 lief unter Windows 10 22H2: [#18](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/18), [#30](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/30))

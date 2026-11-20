---
title: Проверенное оборудование
layout: default
parent: Главная
nav_order: 2.5
---

# Проверенное оборудование
{: .no_toc }

Адаптеры и наушники, с которыми уже работал BlueCaster. Данные взяты из тестов сопровождающего и сообщений пользователей в issues. ✅ работает · ⚠️ работает с ограничением или частично · ❌ не работает · ❓ не подтверждено.

Пробовали BlueCaster с другим оборудованием? Расскажите об этом в [issue](https://github.com/SeiyaFunaokaJP/BlueCaster/issues) (или в [#11](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/11), если речь о кодеках) и приложите текст из **Диагностика > Копировать диагностику**: в нём указаны адаптер, кодек и битрейт.

## Содержание
{: .no_toc .text-delta }

1. TOC
{:toc}

## Адаптеры Bluetooth

| Адаптер | Чип | USB ID | Тип | Результат | Примечания | Источник |
|:--|:--|:--|:--|:--|:--|:--|
| TP-Link UB500 | Realtek RTL8761BU | 2357:0604 | USB-адаптер | ✅ Проверен лучше всего | Прошивка `rtl8761bu`. Нет LE Audio (нет изохронных каналов) | Сопровождающий; [#84](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/84), [#87](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/87) |
| Безымянный адаптер на RTL8761B | Realtek RTL8761BU | 0BDA:A729 | USB-адаптер | ✅ | Прошивка `rtl8761bu`. Определяется автоматически начиная с 2.0.2 (раньше нужно было выбрать чип RTL8761BU). Нет LE Audio | [#66](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/66), [#86](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/86) |
| Встроенный Realtek RTL8852BE ноутбука | Realtek RTL8852BU | 0BDA:4853 | Встроенный | ✅ | Прошивка `rtl8852bu`, распознаётся начиная с 2.0.2-beta.1. Пока адаптер занят BlueCaster, Bluetooth в Windows недоступен | [#81](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/81) |
| ASUS BT400 | Broadcom BCM20702 | 0B05:17CB | USB-адаптер | ✅ | Работает на прошивке из ПЗУ, файлы не нужны. Сообщения относятся к v1 (1.0.12–1.0.17) | [#18](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/18), [#27](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/27), [#30](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/30) |
| Адаптер «Realtek Bluetooth 6.0» | Realtek RTL8761B | 0BDA:A760 | USB-адаптер | ✅ | Пока не определяется автоматически (LMP subversion 0x8761). Нет LE Audio | [#208](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/208) |
| Адаптер CSR8510 A10 | CSR8510 | 0A12:0001 | USB-адаптер | ✅ | Файлы прошивки не нужны | [#32](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/32) |
| Intel AX211 (встроенная карта Wi-Fi / Bluetooth) | Intel AX211 (загрузчик TLV) | 8087:0033 | Встроенный | ✅ | Прошивка из папки `intel/` пакета linux-firmware загружается через загрузчик за 1,5–1,7 с: `ibt-0180-0041` (вариант 0x1B) или `ibt-1040-0041` (вариант 0x19). Какой файл выбран, видно в журнале. Сопряжение, AAC и LDAC работают. Пока адаптер занят BlueCaster, Bluetooth в Windows недоступен | [#261](https://github.com/SeiyaFunaokaJP/BlueCaster/pull/261), [#237](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/237) |
| Intel AX200 (встроенная карта Wi-Fi / Bluetooth) | Intel AX200 | 8087:0029 | Встроенный | ✅ | В отчёте прошивка уже была запущена, поэтому файл не загружался. LHDC V5 1000 kbps / 192 kHz работает. Пока адаптер занят BlueCaster, Bluetooth в Windows недоступен | [#237](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/237) |
| MediaTek MT7925 (встроенная карта Wi-Fi / Bluetooth) | MediaTek MT7925 | 0E8D:7925 | Встроенный | ✅ | Прошивка берётся из драйвера MediaTek для Windows. A2DP работает. LE Audio (экспериментально): звук был лишь в некоторых тестах, пока работает ненадёжно | [#86](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/86) |
| MediaTek MT7921 (встроенная карта Wi-Fi / Bluetooth) | MediaTek MT7921 | 13D3:3606 | Встроенный | ✅ | – | [#432](https://github.com/SeiyaFunaokaJP/BlueCaster/pull/432) |
| Адаптер «MA530» | Не указан | – | USB-адаптер | ✅ | Сообщение относится к v1.0.9 | [#3](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/3), [#9](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/9) |
| Адаптеры UGREEN Bluetooth 5.4 и 6.0 | Не указан | – | USB-адаптер | ✅ | Сообщение относится к 2.0.1 | [#11](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/11#issuecomment-6563908711) |

Об адаптерах Qualcomm сообщений пока нет. Чего от них ожидать, описано в разделе [Рекомендуемые адаптеры](setup#adapters).

## Наушники

| Наушники | Адаптер | Кодеки | Примечания | Версия | Источник |
|:--|:--|:--|:--|:--|:--|
| Sony WH-1000XM4 | TP-Link UB500 | ✅ LDAC 990 kbps (до 96 kHz) · ✅ AAC · ✅ SBC · ✅ SBC XQ 552 kbps | У этой модели нет aptX. Пока включён multipoint, LDAC недоступен | 1.0 – 2.0 | Сопровождающий |
| Sony WH-1000XM4 | ASUS BT400 | ✅ LDAC 990 kbps, 48 kHz / 24 бит | Windows 10 22H2 | 1.0.12, 1.0.15 | [#18](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/18), [#27](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/27) |
| Sony WH-1000XM6 | Встроенный RTL8852BU | ✅ LDAC 990 kbps, 48 kHz / 24 бит (5 минут, 0 потерянных пакетов) | LE Audio на наушниках выключено. Для первого подключения нужен режим сопряжения | 2.0.1, 2.0.2-beta.1 | [#81](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/81) |
| EarFun Air Pro 4+ | RTL8761B (0BDA:A729) | ✅ LDAC · ✅ AAC · ✅ SBC · ✅ SBC XQ · ✅ aptX · ✅ aptX Adaptive | | 2.0.2-beta.1, 2.0.2-beta.6 | [#86](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/86) |
| Grado GW100x | «MA530» | ✅ aptX | Поддерживают и aptX Adaptive, но в 1.0.9 ещё не было кодировщика aptX Adaptive, поэтому передавался классический aptX (aptX Adaptive на них пока не проверен) | 1.0.9 | [#9](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/9) |
| Наушники с LHDC V5 (модель не указана) | RTL8761B | ✅ LHDC V5 1000 kbps / 192 kHz | Переключите приложение наушников в режим 192 kHz; в режиме 96 kHz они принимают только 400 kbps | 1.1.1-beta.1 | [#50](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/50) |
| OPPO Enco Free4 | Intel AX200 | ✅ LHDC V5 1000 kbps / 192 kHz (10 минут, 0 потерянных пакетов) | Без Hi-Res они предлагают не больше 48 kHz и 400 kbps, даже если Hi-Res включён в их приложении. BlueCaster запрашивает у них Hi-Res и подключается заново; после этого они предлагают до 192 kHz без ограничения битрейта. Однократный запрос из 2.0.2-beta.18 на них пока не подтверждён | 2.0.2-beta.17 | [#237](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/237) |
| OPPO Enco X4 | Intel AX211 | ⚠️ LHDC V5 1000 kbps | Когда Hi-Res включён в их приложении, они сразу предлагали до 192 kHz без ограничения битрейта, и запрос от BlueCaster не понадобился. На 1000 kbps в этом соединении терялись пакеты (пригодными были 20 из 79 каналов, вероятно, из-за Wi-Fi поблизости). Стандартные 500 kbps в долгом сеансе пока не проверены | 2.0.2-beta.18 | [#237](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/237) |
| JBL Tune 510BT | TP-Link UB500 | ✅ SBC · ⚠️ AAC (тишина) | Исправление AAC ([#89](https://github.com/SeiyaFunaokaJP/BlueCaster/pull/89)) на этой модели пока не подтверждено | 2.0.1 | [#43](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/43), [#84](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/84), [#87](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/87) |
| Huawei FreeBuds Pro 4 | ASUS BT400 | ✅ AAC 256 kbps · ❓ LDAC | В 1.0.16 они предлагали только SBC и AAC. Если выбран LDAC (автовыбор выключен), BlueCaster теперь включает на них режим «приоритет качества звука» и подключается заново. Если LDAC так и не появляется, BlueCaster возвращает прежний режим. На этих наушниках это пока не подтверждено. L2HC — закрытый кодек | 1.0.16 | [#30](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/30) |
| Huawei FreeBuds 5i | RTL8761B (0BDA:A760) | ✅ LDAC 990 / 660 kbps · ✅ AAC 256 kbps · ✅ SBC | Предлагали LDAC не при каждом подключении. Также предлагают кодек Huawei (0x027D/0x4C36), который BlueCaster не поддерживает | 2.0.1 | [#208](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/208) |
| Huawei FreeBuds Neo | RTL8761B (0BDA:A760) | ✅ LDAC 990 kbps · ✅ AAC 256 kbps · ✅ SBC | В 2.0.1 они предлагали только SBC и AAC. В 2.0.2-beta.13 LDAC работает, если выбрать LDAC в профиле (автовыбор выключен). При загруженном Wi-Fi 2,4 GHz звук пропадал, пока для AFH не выбрали «Вручную» с каналом роутера | 2.0.2-beta.13 | [#208](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/208), [#30](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/30) |
| EDIFIER W820NB (с двумя золотыми знаками) | CSR8510 A10 | ✅ SBC · ❓ LDAC | В 1.0.16 LDAC не работал, в 1.0.17 это исправлено, но с тех пор не перепроверялось | 1.0.16 | [#32](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/32) |
| ROSESELSA Ceramics Ultra | UGREEN Bluetooth 5.4 / 6.0 | ✅ LDAC · ⚠️ LHDC V5 · ✅ AAC · ✅ SBC | С LDAC качество связи и битрейт были самыми стабильными. LHDC V5 иногда прерывается без видимой причины | 2.0.1 | [#11](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/11#issuecomment-6563908711) |

## Тестовое оборудование

| Что | Конфигурация | Кодеки | Результат |
|:--|:--|:--|:--|
| Измерительный приёмник [`tools/linux_sink`](usage#receiver) | TP-Link UB500 → Ubuntu 26.04 (Intel AX201) по радио | SBC, AAC, aptX, aptX HD, aptX LL, LDAC | ✅ Принято без потерь |
| Виртуальное соединение `tools/emu` (Bumble) | Без радио | Все кодеки и LE Audio | ✅ Сквозной тест проходит |

## Ещё не проверено

- **Кодеки на реальных наушниках**: aptX HD, aptX Low Latency, Opus, Opus 05, FastStream
- **LE Audio** (экспериментально, только в режиме разработчика): работает с эмулятором, но на реальных адаптерах пока ненадёжно (проверено на MediaTek MT7925)
- **Windows на ARM64** и **Windows 10 с v2** (v1 работала в Windows 10 22H2: [#18](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/18), [#30](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/30))

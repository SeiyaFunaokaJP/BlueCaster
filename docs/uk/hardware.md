---
title: Перевірене обладнання
layout: default
parent: Головна
nav_order: 2.5
---

# Перевірене обладнання
{: .no_toc }

Адаптери й навушники, з якими вже працював BlueCaster: за тестами супровідника та звітами користувачів в issues. ✅ працює · ⚠️ працює з обмеженням або частково · ❌ не працює · ❓ не підтверджено.

Пробували BlueCaster з іншим обладнанням? Напишіть нам в [issue](https://github.com/SeiyaFunaokaJP/BlueCaster/issues) (або в [#11](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/11), якщо йдеться про кодеки) і додайте текст із **Діагностика > Копіювати діагностику** — у ньому вказано адаптер, кодек і бітрейт.

## Зміст
{: .no_toc .text-delta }

1. TOC
{:toc}

## Bluetooth-адаптери

| Адаптер | Мікросхема | USB ID | Тип | Результат | Примітки | Джерело |
|:--|:--|:--|:--|:--|:--|:--|
| TP-Link UB500 | Realtek RTL8761BU | 2357:0604 | USB-донгл | ✅ Перевірено найкраще | Мікропрограма `rtl8761bu`. Без LE Audio (немає ізохронних каналів) | Супровідник; [#84](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/84), [#87](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/87) |
| Типовий донгл RTL8761B | Realtek RTL8761BU | 0BDA:A729 | USB-донгл | ✅ | Мікропрограма `rtl8761bu`. Визначається автоматично з версії 2.0.2 (раніше: виберіть RTL8761BU як мікросхему). Без LE Audio | [#66](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/66), [#86](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/86) |
| Вбудований у ноутбук Realtek RTL8852BE | Realtek RTL8852BU | 0BDA:4853 | Вбудований | ✅ | Мікропрограма `rtl8852bu`, розпізнається з 2.0.2-beta.1. Поки адаптером користується BlueCaster, у Windows немає власного Bluetooth | [#81](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/81) |
| ASUS BT400 | Broadcom BCM20702 | 0B05:17CB | USB-донгл | ✅ | Працює на мікропрограмі з ПЗП, файли не потрібні. Повідомлялося для v1 (1.0.12–1.0.17) | [#18](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/18), [#27](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/27), [#30](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/30) |
| Донгл «Realtek Bluetooth 6.0» | Realtek RTL8761B | 0BDA:A760 | USB-донгл | ✅ | Поки що не визначається автоматично (LMP subversion 0x8761). Без LE Audio | [#208](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/208) |
| Донгл CSR8510 A10 | CSR8510 | 0A12:0001 | USB-донгл | ✅ | Файли мікропрограми не потрібні | [#32](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/32) |
| Intel AX211 (вбудована карта Wi-Fi / Bluetooth) | Intel AX211 (завантажувач TLV) | 8087:0033 | Вбудований | ✅ | Мікропрограма з linux-firmware (`intel/`) завантажується в режимі завантажувача за 1,5–1,7 с: `ibt-0180-0041` (варіант 0x1B) або `ibt-1040-0041` (варіант 0x19), назву файлу видно в журналі. Сполучення, AAC і LDAC працюють. Поки адаптером користується BlueCaster, у Windows немає власного Bluetooth | [#261](https://github.com/SeiyaFunaokaJP/BlueCaster/pull/261), [#237](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/237) |
| Intel AX200 (вбудована карта Wi-Fi / Bluetooth) | Intel AX200 | 8087:0029 | Вбудований | ✅ | У звіті мікропрограма вже працювала, тож завантажувати файл не довелося. LHDC V5 1000 kbps / 192 kHz працює. Поки адаптером користується BlueCaster, у Windows немає власного Bluetooth | [#237](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/237) |
| MediaTek MT7925 (вбудована карта Wi-Fi / Bluetooth) | MediaTek MT7925 | 0E8D:7925 | Вбудований | ✅ | Мікропрограму прочитано з драйвера MediaTek для Windows. A2DP працює. LE Audio (експериментально): звук відтворювався лише в деяких тестах, поки що працює ненадійно | [#86](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/86) |
| MediaTek MT7921 (вбудована карта Wi-Fi / Bluetooth) | MediaTek MT7921 | 13D3:3606 | Вбудований | ✅ | – | [#432](https://github.com/SeiyaFunaokaJP/BlueCaster/pull/432) |
| Адаптер «MA530» | Не вказано | – | USB-донгл | ✅ | Повідомлялося для v1.0.9 | [#3](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/3), [#9](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/9) |
| Донгли UGREEN Bluetooth 5.4 і 6.0 | Не вказано | – | USB-донгл | ✅ | Повідомлялося для 2.0.1 | [#11](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/11#issuecomment-6563908711) |

Про адаптери Qualcomm звітів поки немає; чого від них очікувати, описано в розділі [Рекомендовані адаптери](setup#adapters).

## Навушники

| Навушники | Адаптер | Кодеки | Примітки | Версія | Джерело |
|:--|:--|:--|:--|:--|:--|
| Sony WH-1000XM4 | TP-Link UB500 | ✅ LDAC 990 kbps (до 96 kHz) · ✅ AAC · ✅ SBC · ✅ SBC XQ 552 kbps | Ця модель не підтримує aptX. LDAC недоступний, поки на навушниках увімкнено multipoint | 1.0 – 2.0 | Супровідник |
| Sony WH-1000XM4 | ASUS BT400 | ✅ LDAC 990 kbps, 48 kHz / 24 біт | Windows 10 22H2 | 1.0.12, 1.0.15 | [#18](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/18), [#27](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/27) |
| Sony WH-1000XM6 | Вбудований RTL8852BU | ✅ LDAC 990 kbps, 48 kHz / 24 біт (5 хвилин, 0 втрачених пакетів) | LE Audio на навушниках вимкнено. Для першого підключення потрібен режим сполучення | 2.0.1, 2.0.2-beta.1 | [#81](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/81) |
| EarFun Air Pro 4+ | RTL8761B (0BDA:A729) | ✅ LDAC · ✅ AAC · ✅ SBC · ✅ SBC XQ · ✅ aptX · ✅ aptX Adaptive | | 2.0.2-beta.1, 2.0.2-beta.6 | [#86](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/86) |
| Grado GW100x | «MA530» | ✅ aptX | Підтримують і aptX Adaptive, але версія 1.0.9 ще не мала його кодувальника, тож передавався класичний aptX (aptX Adaptive на цій моделі ще не пробували) | 1.0.9 | [#9](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/9) |
| Навушники з LHDC V5 (модель не вказано) | RTL8761B | ✅ LHDC V5 1000 kbps / 192 kHz | Увімкніть у застосунку навушників режим 192 kHz; у режимі 96 kHz вони приймають лише 400 kbps | 1.1.1-beta.1 | [#50](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/50) |
| OPPO Enco Free4 | Intel AX200 | ✅ LHDC V5 1000 kbps / 192 kHz (10 хвилин, 0 втрачених пакетів) | Без запиту Hi-Res навушники пропонують щонайбільше 48 kHz і 400 kbps, навіть якщо Hi-Res увімкнено в їхньому застосунку. BlueCaster запитує Hi-Res і перепідключається; після цього доступно до 192 kHz без обмеження бітрейту. Одноразовий запит із 2.0.2-beta.18 на цій моделі ще не підтверджено | 2.0.2-beta.17 | [#237](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/237) |
| OPPO Enco X4 | Intel AX211 | ⚠️ LHDC V5 1000 kbps | Коли Hi-Res увімкнено в їхньому застосунку, навушники пропонували до 192 kHz без обмеження бітрейту, тож запит від BlueCaster не знадобився. На 1000 kbps у цьому з'єднанні губилися пакети (придатними були 20 із 79 каналів, імовірно через Wi-Fi поблизу); Standard 500 kbps під час тривалішого прослуховування ще не підтверджено | 2.0.2-beta.18 | [#237](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/237) |
| JBL Tune 510BT | TP-Link UB500 | ✅ SBC · ⚠️ AAC (без звуку) | Виправлення AAC ([#89](https://github.com/SeiyaFunaokaJP/BlueCaster/pull/89)) на цій моделі ще не підтверджено | 2.0.1 | [#43](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/43), [#84](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/84), [#87](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/87) |
| Huawei FreeBuds Pro 4 | ASUS BT400 | ✅ AAC 256 kbps · ❓ LDAC | У 1.0.16 навушники пропонували лише SBC і AAC. Якщо вибрано LDAC (автовибір вимкнено), BlueCaster тепер перемикає їх у режим «спершу якість звуку» й перепідключається, а якщо LDAC так і не з'являється, повертає попередній режим; на цих навушниках це ще не підтверджено. L2HC — пропрієтарний кодек | 1.0.16 | [#30](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/30) |
| Huawei FreeBuds 5i | RTL8761B (0BDA:A760) | ✅ LDAC 990 / 660 kbps · ✅ AAC 256 kbps · ✅ SBC | LDAC пропонували не під час кожного підключення. Також пропонують власний кодек Huawei (0x027D/0x4C36), який BlueCaster не підтримує | 2.0.1 | [#208](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/208) |
| Huawei FreeBuds Neo | RTL8761B (0BDA:A760) | ✅ LDAC 990 kbps · ✅ AAC 256 kbps · ✅ SBC | У 2.0.1 навушники пропонували лише SBC і AAC. У 2.0.2-beta.13 LDAC працює, якщо вибрати його в профілі (автовибір вимкнено). У завантаженій мережі Wi-Fi 2,4 GHz звук переривався, доки для AFH не вибрали «Вручну» й канал маршрутизатора | 2.0.2-beta.13 | [#208](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/208), [#30](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/30) |
| EDIFIER W820NB (dual gold label) | CSR8510 A10 | ✅ SBC · ❓ LDAC | У 1.0.16 LDAC не працював, у 1.0.17 це виправили; відтоді не підтверджено | 1.0.16 | [#32](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/32) |
| ROSESELSA Ceramics Ultra | UGREEN Bluetooth 5.4 / 6.0 | ✅ LDAC · ⚠️ LHDC V5 · ✅ AAC · ✅ SBC | З LDAC якість зв'язку й бітрейт були найстабільнішими. LHDC V5 іноді переривається без видимої причини | 2.0.1 | [#11](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/11#issuecomment-6563908711) |

## Тестове обладнання

| Що | Конфігурація | Кодеки | Результат |
|:--|:--|:--|:--|
| Вимірювальний приймач [`tools/linux_sink`](usage#receiver) | TP-Link UB500 → Ubuntu 26.04 (Intel AX201) по радіоканалу | SBC, AAC, aptX, aptX HD, aptX LL, LDAC | ✅ Прийнято без втрат |
| Віртуальне з'єднання `tools/emu` (Bumble) | Без радіоканалу | Усі кодеки та LE Audio | ✅ Наскрізний тест проходить |

## Ще не перевірено

- **Кодеки на справжніх навушниках**: aptX HD, aptX Low Latency, Opus, Opus 05, FastStream
- **LE Audio** (експериментально, лише в режимі розробника): працює з емулятором, але на справжніх адаптерах поки що ненадійно (перевірено на MediaTek MT7925)
- **Windows на ARM64** і **Windows 10 з v2** (v1 працювала на Windows 10 22H2: [#18](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/18), [#30](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/30))

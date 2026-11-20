---
title: 已驗證的硬體
layout: default
parent: 首頁
nav_order: 2.5
---

# 已驗證的硬體
{: .no_toc }

以下是曾與 BlueCaster 搭配使用的介面卡與耳機，資料來自維護者的測試，以及使用者在 issue 中的回報。✅ 可用 · ⚠️ 可用但有限制，或部分可用 · ❌ 無法使用 · ❓ 尚未確認。

您在其他硬體上試過 BlueCaster 嗎？請開一個 [issue](https://github.com/SeiyaFunaokaJP/BlueCaster/issues) 告訴我們（編解碼器相關請回報到 [#11](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/11)），並附上 **診斷 > 複製診斷資訊** 複製的文字，其中包含介面卡、編解碼器與位元率。

## 目錄
{: .no_toc .text-delta }

1. TOC
{:toc}

## 藍牙介面卡

| 介面卡 | 晶片 | USB ID | 類型 | 結果 | 備註 | 來源 |
|:--|:--|:--|:--|:--|:--|:--|
| TP-Link UB500 | Realtek RTL8761BU | 2357:0604 | USB 接收器 | ✅ 測試最完整 | 韌體 `rtl8761bu`。不支援 LE Audio（沒有等時通道） | 維護者；[#84](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/84)、[#87](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/87) |
| 通用 RTL8761B 接收器 | Realtek RTL8761BU | 0BDA:A729 | USB 接收器 | ✅ | 韌體 `rtl8761bu`。自 2.0.2 起可自動偵測（之前的版本請將晶片選為 RTL8761BU）。不支援 LE Audio | [#66](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/66)、[#86](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/86) |
| 筆電內建的 Realtek RTL8852BE | Realtek RTL8852BU | 0BDA:4853 | 內建 | ✅ | 韌體 `rtl8852bu`，自 2.0.2-beta.1 起可辨識。BlueCaster 使用這張網卡時，Windows 無法使用藍牙 | [#81](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/81) |
| ASUS BT400 | Broadcom BCM20702 | 0B05:17CB | USB 接收器 | ✅ | 使用內建 ROM 韌體，不需要任何檔案。在 v1（1.0.12–1.0.17）上回報 | [#18](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/18)、[#27](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/27)、[#30](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/30) |
| 「Realtek Bluetooth 6.0」接收器 | Realtek RTL8761B | 0BDA:A760 | USB 接收器 | ✅ | 尚無法自動偵測（LMP subversion 0x8761）。不支援 LE Audio | [#208](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/208) |
| CSR8510 A10 接收器 | CSR8510 | 0A12:0001 | USB 接收器 | ✅ | 不需要韌體檔案 | [#32](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/32) |
| Intel AX211（內建 Wi-Fi / 藍牙網卡） | Intel AX211（TLV bootloader） | 8087:0033 | 內建 | ✅ | 韌體取自 linux-firmware `intel/`，從 bootloader 載入約需 1.5–1.7 秒：`ibt-0180-0041`（variant 0x1B）或 `ibt-1040-0041`（variant 0x19），記錄中會顯示檔案名稱。配對、AAC 與 LDAC 可用。BlueCaster 使用這張網卡時，Windows 無法使用藍牙 | [#261](https://github.com/SeiyaFunaokaJP/BlueCaster/pull/261)、[#237](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/237) |
| Intel AX200（內建 Wi-Fi / 藍牙網卡） | Intel AX200 | 8087:0029 | 內建 | ✅ | 回報時韌體已在執行，因此沒有載入檔案。LHDC V5 1000 kbps / 192 kHz 可用。BlueCaster 使用這張網卡時，Windows 無法使用藍牙 | [#237](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/237) |
| MediaTek MT7925（內建 Wi-Fi / 藍牙網卡） | MediaTek MT7925 | 0E8D:7925 | 內建 | ✅ | 韌體取自 MediaTek 的 Windows 驅動程式。A2DP 可用。LE Audio（實驗性）僅在部分測試中播放成功，目前尚無法穩定運作 | [#86](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/86) |
| MediaTek MT7921（內建 Wi-Fi / 藍牙網卡） | MediaTek MT7921 | 13D3:3606 | 內建 | ✅ | – | [#432](https://github.com/SeiyaFunaokaJP/BlueCaster/pull/432) |
| 「MA530」介面卡 | 未提供 | – | USB 接收器 | ✅ | 在 v1.0.9 上回報 | [#3](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/3)、[#9](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/9) |
| UGREEN 藍牙 5.4 與 6.0 接收器 | 未提供 | – | USB 接收器 | ✅ | 在 2.0.1 上回報 | [#11](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/11#issuecomment-6563908711) |

Qualcomm 介面卡目前還沒有回報。可能的情況請參閱 [建議的介面卡](setup#adapters)。

## 耳機

| 耳機 | 介面卡 | 編解碼器 | 備註 | 版本 | 來源 |
|:--|:--|:--|:--|:--|:--|
| Sony WH-1000XM4 | TP-Link UB500 | ✅ LDAC 990 kbps（最高 96 kHz） · ✅ AAC · ✅ SBC · ✅ SBC XQ 552 kbps | 此型號不支援 aptX。開啟多點連線時無法使用 LDAC | 1.0 – 2.0 | 維護者 |
| Sony WH-1000XM4 | ASUS BT400 | ✅ LDAC 990 kbps、48 kHz / 24 位元 | Windows 10 22H2 | 1.0.12、1.0.15 | [#18](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/18)、[#27](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/27) |
| Sony WH-1000XM6 | 內建 RTL8852BU | ✅ LDAC 990 kbps、48 kHz / 24 位元（5 分鐘，遺失 0 個封包） | 耳機的 LE Audio 已關閉。第一次連線時，耳機需進入配對模式 | 2.0.1、2.0.2-beta.1 | [#81](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/81) |
| EarFun Air Pro 4+ | RTL8761B (0BDA:A729) | ✅ LDAC · ✅ AAC · ✅ SBC · ✅ SBC XQ · ✅ aptX · ✅ aptX Adaptive | | 2.0.2-beta.1、2.0.2-beta.6 | [#86](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/86) |
| Grado GW100x | 「MA530」 | ✅ aptX | 也支援 aptX Adaptive。1.0.9 沒有 aptX Adaptive 編碼器，因此傳送傳統 aptX（尚未在這副耳機上試過 aptX Adaptive） | 1.0.9 | [#9](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/9) |
| LHDC V5 耳機（未提供型號） | RTL8761B | ✅ LHDC V5 1000 kbps / 192 kHz | 請在耳機的 App 中設為 192 kHz 模式。在 96 kHz 模式下，耳機只接受 400 kbps | 1.1.1-beta.1 | [#50](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/50) |
| OPPO Enco Free4 | Intel AX200 | ✅ LHDC V5 1000 kbps / 192 kHz（10 分鐘，遺失 0 個封包） | 未啟用 Hi-Res 時，即使在耳機的 App 中開啟 Hi-Res，耳機最多也只提供 48 kHz 與 400 kbps。BlueCaster 會向耳機要求 Hi-Res 並重新連線，之後耳機會提供最高 192 kHz，且沒有位元率限制。2.0.2-beta.18 的一次性要求尚未在這副耳機上確認 | 2.0.2-beta.17 | [#237](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/237) |
| OPPO Enco X4 | Intel AX211 | ⚠️ LHDC V5 1000 kbps | 在耳機的 App 中開啟 Hi-Res 後，耳機提供最高 192 kHz 且沒有位元率限制，因此 BlueCaster 不需要另外要求。在 1000 kbps 下，這條連結會掉封包（79 個通道只有 20 個可用，可能是附近有 Wi-Fi）。標準 500 kbps 尚未經過長時間確認 | 2.0.2-beta.18 | [#237](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/237) |
| JBL Tune 510BT | TP-Link UB500 | ✅ SBC · ⚠️ AAC（無聲） | AAC 的修正（[#89](https://github.com/SeiyaFunaokaJP/BlueCaster/pull/89)）尚未在這個型號上確認 | 2.0.1 | [#43](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/43)、[#84](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/84)、[#87](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/87) |
| Huawei FreeBuds Pro 4 | ASUS BT400 | ✅ AAC 256 kbps · ❓ LDAC | 在 1.0.16 上只提供 SBC 與 AAC。現在選擇 LDAC（並關閉自動選擇）時，BlueCaster 會把耳機切換到「音質優先」模式並重新連線；若仍未提供 LDAC，就切換回原本的模式。尚未在這副耳機上確認。L2HC 為專有格式 | 1.0.16 | [#30](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/30) |
| Huawei FreeBuds 5i | RTL8761B (0BDA:A760) | ✅ LDAC 990 / 660 kbps · ✅ AAC 256 kbps · ✅ SBC | 只有部分連線會提供 LDAC。另外也提供 BlueCaster 不支援的華為編解碼器（0x027D/0x4C36） | 2.0.1 | [#208](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/208) |
| Huawei FreeBuds Neo | RTL8761B (0BDA:A760) | ✅ LDAC 990 kbps · ✅ AAC 256 kbps · ✅ SBC | 在 2.0.1 上只提供 SBC 與 AAC。在 2.0.2-beta.13 上，只要在設定檔中選擇 LDAC（並關閉自動選擇），LDAC 即可使用。在 2.4 GHz Wi-Fi 壅塞的環境下會斷音，將 AFH 設為「手動」並指定路由器的通道後即改善 | 2.0.2-beta.13 | [#208](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/208)、[#30](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/30) |
| EDIFIER W820NB（雙金標） | CSR8510 A10 | ✅ SBC · ❓ LDAC | LDAC 在 1.0.16 上失敗，已在 1.0.17 修正，之後尚未再確認 | 1.0.16 | [#32](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/32) |
| ROSESELSA Ceramics Ultra | UGREEN 藍牙 5.4 / 6.0 | ✅ LDAC · ⚠️ LHDC V5 · ✅ AAC · ✅ SBC | LDAC 的連線品質與位元率最穩定。LHDC V5 偶爾會斷音 | 2.0.1 | [#11](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/11#issuecomment-6563908711) |

## 測試設備

| 項目 | 配置 | 編解碼器 | 結果 |
|:--|:--|:--|:--|
| 測量用接收器 [`tools/linux_sink`](usage#receiver) | TP-Link UB500 → Ubuntu 26.04（Intel AX201），透過無線傳輸 | SBC、AAC、aptX、aptX HD、aptX LL、LDAC | ✅ 接收無遺失 |
| 虛擬連結 `tools/emu`（Bumble） | 無無線電 | 所有編解碼器，以及 LE Audio | ✅ 端對端測試通過 |

## 尚未驗證

- **實際耳機上的編解碼器**：aptX HD、aptX Low Latency、Opus、Opus 05、FastStream
- **LE Audio**（實驗性，僅限開發人員模式）：在模擬器上可用，但在實體介面卡上尚無法穩定運作（在 MediaTek MT7925 上測試）
- **ARM64 版 Windows**，以及 **搭配 v2 的 Windows 10**（v1 曾在 Windows 10 22H2 上執行：[#18](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/18)、[#30](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/30)）

---
title: 搭配使用的工具
layout: default
parent: 首頁
nav_order: 2.6
---

# 搭配使用的工具
{: .no_toc }

以下是可與 BlueCaster 搭配使用的第三方應用程式與工具，由作者或使用者在 issue 中回報。

{: .warning }
> 這些工具 **不屬於 BlueCaster**，BlueCaster 專案並未製作、測試或支援它們。列在這裡，只是因為有人回報可以搭配使用。若工具無法運作、在任一方更新後失效，或造成任何問題，我們概不負責。請參閱各專案的授權與文件；工具本身的問題，請先向該工具的專案回報。

## 目錄
{: .no_toc .text-delta }

1. TOC
{:toc}

## 應用程式與工具

| 工具 | 與 BlueCaster 搭配時的功能 | 需求 | 授權 | 回報者 |
|:--|:--|:--|:--|:--|
| [EncoDeck](https://github.com/TIANLI0/EncoDeck) | 控制 OPPO、Sony、HUAWEI、小米等品牌的耳機（電量、降噪、EQ、手勢）。以第二個用戶端連線到核心的管道：顯示串流狀態與編解碼器、啟動及停止設定檔、設定 AVRCP 音量，並協助指派 WinUSB。耳機設定透過 RFCOMM 轉送，經由 BlueCaster 本身的藍牙連結傳送 | EncoDeck 1.11.0 以上。RFCOMM 轉送：BlueCaster 2.0.2-beta.10 以上（目前只在模擬器中試過） | GPL-3.0 | 工具作者，[#194](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/194) |

## 新增工具

您製作或找到了可與 BlueCaster 搭配使用的工具嗎？請開一個 [issue](https://github.com/SeiyaFunaokaJP/BlueCaster/issues)，告訴我們工具名稱、連結、授權、與 BlueCaster 搭配時的功能，以及您試過的版本。列在這裡並不代表評測或背書。

## 給工具作者

工具與核心透過具名管道通訊，這條管道與 BlueCaster 視窗使用的相同，每位使用者各有一條。通訊協定、供工具使用的方法、RFCOMM 轉送，以及相容性規則（功能檢查、3.0.0 的管道更名）請參閱 **[整合 (IPC)](integration)**。在 2.0.2 搶先版期間，通訊協定與核心仍可能變更。

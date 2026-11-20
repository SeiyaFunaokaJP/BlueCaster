---
title: 連携するツール
layout: default
parent: ホーム
nav_order: 2.6
---

# 連携するツール
{: .no_toc }

BlueCaster と組み合わせて使えるサードパーティのアプリやツールです。作者や利用者から issue で報告されたものを載せています。

{: .warning }
> これらは **BlueCaster の一部ではなく**、BlueCaster プロジェクトが開発、テスト、サポートしているものではありません。BlueCaster と組み合わせて動作するという報告があったので、載せているだけです。ツールが動かない場合、どちらかのプログラムの更新で動かなくなった場合、その他の問題が起きた場合も、責任は負いかねます。各プロジェクトのライセンスとドキュメントを確認し、ツールの問題はまずそのツールのプロジェクトに報告してください。

## 目次
{: .no_toc .text-delta }

1. TOC
{:toc}

## アプリとツール {#apps-and-tools}

| ツール | BlueCaster と組み合わせてできること | 必要なもの | ライセンス | 報告者 |
|:--|:--|:--|:--|:--|
| [EncoDeck](https://github.com/TIANLI0/EncoDeck) | OPPO、Sony、HUAWEI、Xiaomi などのイヤホンやヘッドホンの操作（バッテリー、ノイズコントロール、EQ、ジェスチャー）。コアのパイプに 2 つ目のクライアントとして接続し、ストリームの状態とコーデックの表示、プロファイルの開始と停止、AVRCP の音量の設定、WinUSB の割り当ての補助を行います。イヤホンの設定は、RFCOMM パススルーを通じて BlueCaster 自身の Bluetooth リンクで送られます | EncoDeck 1.11.0 以降。RFCOMM パススルー: BlueCaster 2.0.2-beta.10 以降（今のところエミュレーターでのみ確認） | GPL-3.0 | 作者、[#194](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/194) |

## ツールを追加するには {#adding-a-tool}

BlueCaster と組み合わせて使えるツールを作った、または見つけた場合は、名前、リンク、ライセンス、BlueCaster と組み合わせてできること、試したバージョンを添えて [issue](https://github.com/SeiyaFunaokaJP/BlueCaster/issues) で教えてください。ここに載っていても、レビューや推薦を意味するものではありません。

## ツールの作者の方へ {#for-tool-authors}

ツールは、BlueCaster のウィンドウと同じユーザーごとの名前付きパイプでコアと通信します。プロトコル、ツール向けのメソッド、RFCOMM パススルー、互換性のルール（機能の確認、3.0.0 でのパイプ名の変更）は **[連携（IPC）](integration)** で説明しています。2.0.2 のベータ期間中は、プロトコルとコアが変わる可能性があります。

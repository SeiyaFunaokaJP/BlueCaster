---
title: 함께 쓰는 도구
layout: default
parent: 홈
nav_order: 2.6
---

# 함께 쓰는 도구
{: .no_toc }

BlueCaster와 함께 쓸 수 있는 타사 앱과 도구입니다. 도구 제작자나 사용자가 이슈로 알려 준 것을 모았습니다.

{: .warning }
> 이 도구들은 **BlueCaster에 포함되지 않으며**, BlueCaster 프로젝트가 만들거나 테스트하거나 지원하지 않습니다. 누군가 BlueCaster와 함께 작동한다고 알려 주었기에 실었을 뿐입니다. 도구가 작동하지 않거나, 어느 한쪽을 업데이트한 뒤 작동하지 않게 되거나, 문제를 일으켜도 책임지지 않습니다. 각 프로젝트의 라이선스와 문서를 확인하시고, 도구에 문제가 있으면 먼저 해당 도구의 프로젝트에 알려 주세요.

## 목차
{: .no_toc .text-delta }

1. TOC
{:toc}

## 앱과 도구

| 도구 | BlueCaster와 함께 하는 일 | 필요 조건 | 라이선스 | 알려 준 사람 |
|:--|:--|:--|:--|:--|
| [EncoDeck](https://github.com/TIANLI0/EncoDeck) | OPPO, Sony, HUAWEI, Xiaomi 등의 이어폰과 헤드폰을 제어합니다(배터리, 노이즈 컨트롤, EQ, 제스처). 두 번째 클라이언트로 코어의 파이프에 연결해 스트림 상태와 코덱을 보여 주고, 프로필을 시작·중지하고, AVRCP 볼륨을 설정하고, WinUSB 할당을 돕습니다. 이어폰 설정은 RFCOMM 패스스루를 거쳐 BlueCaster의 Bluetooth 연결로 전달됩니다 | EncoDeck 1.11.0 이상. RFCOMM 패스스루: BlueCaster 2.0.2-beta.10 이상(아직 에뮬레이터에서만 시험함) | GPL-3.0 | 제작자, [#194](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/194) |

## 도구 추가하기

BlueCaster와 함께 작동하는 도구를 만들었거나 찾으셨나요? 도구 이름, 링크, 라이선스, BlueCaster와 함께 하는 일, 시험한 버전을 [이슈](https://github.com/SeiyaFunaokaJP/BlueCaster/issues)로 알려 주세요. 여기에 실렸다고 해서 검토했거나 추천한다는 뜻은 아닙니다.

## 도구 제작자를 위해

도구는 BlueCaster 창과 같은 사용자별 named pipe로 코어와 통신합니다. 프로토콜, 도구용 메서드, RFCOMM 패스스루, 호환성 규칙(기능 확인, 3.0.0의 파이프 이름 변경)은 **[연동 (IPC)](integration)**에서 설명합니다. 2.0.2 베타 기간에는 프로토콜과 코어가 아직 바뀔 수 있습니다.

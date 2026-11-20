---
title: Funktioniert mit
layout: default
parent: Startseite
nav_order: 2.6
---

# Funktioniert mit
{: .no_toc }

Apps und Werkzeuge anderer Anbieter, die laut ihren Autoren oder Nutzern in den Issues mit BlueCaster funktionieren.

{: .warning }
> Diese Werkzeuge sind **nicht Teil von BlueCaster**. Das BlueCaster-Projekt entwickelt, testet und unterstützt sie nicht. Sie stehen hier nur, weil jemand gemeldet hat, dass sie mit BlueCaster funktionieren. Wir übernehmen keine Verantwortung, wenn ein Werkzeug nicht funktioniert, nach einem Update eines der beiden Programme nicht mehr funktioniert oder Probleme verursacht. Beachten Sie bitte Lizenz und Dokumentation des jeweiligen Projekts und melden Sie Probleme mit einem Werkzeug zuerst dort.

## Inhalt
{: .no_toc .text-delta }

1. TOC
{:toc}

## Apps und Werkzeuge

| Werkzeug | Was es mit BlueCaster tut | Voraussetzungen | Lizenz | Gemeldet von |
|:--|:--|:--|:--|:--|
| [EncoDeck](https://github.com/TIANLI0/EncoDeck) | Steuerung von Ohrhörern und Kopfhörern (Akku, Geräuschunterdrückung, EQ, Gesten) für OPPO, Sony, HUAWEI, Xiaomi und andere. Verbindet sich als zweiter Client mit der Pipe des Kerns: zeigt Streamstatus und Codec an, startet und stoppt Profile, stellt die AVRCP-Lautstärke ein und hilft bei der WinUSB-Zuweisung. Die Einstellungen der Ohrhörer laufen per RFCOMM-Passthrough über die Bluetooth-Verbindung von BlueCaster | EncoDeck 1.11.0 oder neuer. RFCOMM-Passthrough: BlueCaster 2.0.2-beta.10 oder neuer (bisher nur im Emulator ausprobiert) | GPL-3.0 | Sein Autor, [#194](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/194) |

## Ein Werkzeug hinzufügen

Haben Sie ein Werkzeug entwickelt oder gefunden, das mit BlueCaster funktioniert? Schreiben Sie uns in einem [Issue](https://github.com/SeiyaFunaokaJP/BlueCaster/issues) den Namen, einen Link, die Lizenz, was es mit BlueCaster macht und welche Versionen Sie ausprobiert haben. Ein Eintrag hier ist keine Bewertung und keine Empfehlung.

## Für Autoren von Werkzeugen

Werkzeuge kommunizieren mit dem Kern über dieselbe benutzerbezogene Named Pipe wie das BlueCaster-Fenster. Das Protokoll, die für Werkzeuge gedachten Methoden, das RFCOMM-Passthrough und die Kompatibilitätsregeln (Feature-Prüfung, die Umbenennung der Pipe in 3.0.0) stehen unter **[Integration (IPC)](integration)**. Protokoll und Kern können sich während der 2.0.2-Betas noch ändern.

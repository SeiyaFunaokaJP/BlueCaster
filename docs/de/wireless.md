---
title: Aussetzer und Verbindungsprobleme
layout: default
parent: Startseite
nav_order: 2.4
---

# Aussetzer und Verbindungsprobleme
{: .no_toc }

Der Ton setzt aus, stottert oder bricht ab, eine Verbindung kommt nicht zustande, oder die Bitrate ist niedriger als erwartet? Diese Seite fasst zusammen, welche Ursachen hinter bisherigen Berichten zu solchen Problemen steckten. So können Sie die häufigen Ursachen prüfen, bevor Sie ein Problem melden. Passt keine davon, finden Sie hier, was Sie mitschicken sollten.

## Inhalt
{: .no_toc .text-delta }

1. TOC
{:toc}

## Zuerst prüfen {#checklist}

Bluetooth teilt sich das 2,4-GHz-Band mit WLAN, kabellosen Mäusen und Tastaturen, Headset-Dongles und allen anderen Bluetooth-Geräten im Raum. Ein USB-Adapter hat eine kleine Antenne, Ohrhörer eine noch kleinere, und zwischen ihnen und dem PC ist Ihr Kopf. Die meisten Berichte über eine „schlechte Verbindung“ gingen bisher auf eine der folgenden Ursachen zurück. Jede lässt sich in etwa einer Minute prüfen. Im Formular für Fehlerberichte gibt es zu jeder ein Kontrollkästchen. Haken Sie bitte an, was Sie ausprobiert haben.

{: .note }
Prüfen Sie einen Punkt nach dem anderen und hören Sie dazwischen jeweils einige Minuten zu. Hören die Aussetzer auf, haben Sie die Ursache gefunden.

### 1. Die anderen Bluetooth-Adapter in diesem PC ausschalten

Das Windows-eigene Bluetooth (der integrierte Adapter) sendet weiter, während BlueCaster über den USB-Adapter daneben streamt. Beide teilen sich ein Band und oft auch denselben Antennenbereich. Außerdem verbindet sich die Kopfhörer-App auf dem PC über diesen Adapter, sodass die Kopfhörer eine zweite Verbindung haben (siehe nächster Punkt). Deaktivieren Sie den integrierten Adapter im Geräte-Manager, solange BlueCaster läuft, oder zumindest für den Test. Erkennt BlueCaster einen weiteren aktiven Adapter, zeigt es auf Start einen Hinweis an.

Ein langjähriger Tester nannte einen zweiten aktiven Adapter als eine von zwei Ursachen für instabile Verbindungen und Probleme beim Codec-Wechsel.

### 2. Die Kopfhörer nur mit einem Gerät verbinden

Mit der Doppelverbindung (Multipoint) bleiben die Kopfhörer gleichzeitig mit einem Smartphone und dem PC verbunden. Jede Verbindung belegt Sendezeit. Manche Kopfhörer bieten bei eingeschaltetem Multipoint außerdem weniger Codecs an, verweigern die Freisprechverbindung (dann fehlt der Akkustand) oder reservieren einen Codec-Endpunkt für das andere Gerät. Auch die Begleit-App der Kopfhörer zählt: Sie hält eine eigene Verbindung offen, auf dem Smartphone oder über das Bluetooth dieses PCs, selbst wenn Sie sie nur für Akkustand oder Equalizer verwenden.

Für den Test: Schalten Sie die Doppelverbindung in der Kopfhörer-App aus, schalten Sie Bluetooth am Smartphone aus, schließen Sie die Kopfhörer-App auf dem Smartphone und auf dem PC, und verbinden Sie dann erneut aus BlueCaster.

Die Doppelverbindung ist die zweite Ursache, die dieser Tester nannte. Codec-Wechsel liefen deutlich reibungsloser, sobald die Begleit-App getrennt war. Ein Paar Kopfhörer lehnte die erste Verbindung ab, solange es noch mit einem Smartphone mit eingeschaltetem LE Audio verbunden war. Mehrere Ohrhörer boten BlueCaster weniger Codecs oder eine niedrigere Bitrate an, solange ein Smartphone oder ihre App beteiligt war. Sony-Kopfhörer schalten LDAC bei eingeschaltetem Multipoint ab.

### 3. Den Adapter umstecken

- Verwenden Sie einen **USB 2.0-Anschluss** oder ein kurzes USB-Verlängerungskabel. USB-3-Anschlüsse und -Kabel strahlen Störungen im 2,4-GHz-Band ab, genau dort, wo der Adapter steckt.
- Halten Sie ihn **fern von anderen Funkempfängern**: Dongles von Gaming-Mäusen und -Tastaturen, Dongles kabelloser Headsets, WLAN-Antennen. Ein Empfänger im Nachbaranschluss ist eine häufige Ursache für genau dieses Fehlerbild.
- Halten Sie ihn **in Sichtweite der Kopfhörer**. Steht der PC unter dem Schreibtisch und sind Sie und der Tisch dazwischen, geht viel Signal verloren. Ein USB-Verlängerungskabel, mit dem der Adapter auf dem Schreibtisch liegt, hilft mehr als alles andere.

In einem Fall steckte der Empfänger einer Gaming-Maus neben dem Adapter, und der PC hatte keinen WLAN-Adapter, nach dem sich AFH hätte richten können. Die Verbindung schaffte nur etwa 750 kbps, und der Adapter markierte bis zu 46 der 79 Kanäle als schlecht. Ohrhörer, die einen Meter vom Adapter entfernt waren, hatten trotzdem ein schwaches Signal. Nahe der Bitratengrenze reichte das für einen Ausfall alle paar Minuten.

### 4. 2,4-GHz-WLAN von der Verbindung fernhalten

Verbinden Sie diesen PC und nach Möglichkeit auch das Smartphone und andere Geräte in der Nähe mit einem 5- oder 6-GHz-WLAN. Muss 2,4 GHz eingeschaltet bleiben, weist BlueCaster den Adapter an, den WLAN-Kanal dieses PCs zu meiden (AFH, Automatisch). BlueCaster kann diesen Kanal nicht erkennen, wenn der PC keinen WLAN-Adapter hat, per Kabel verbunden ist oder ein anderes Band nutzt als das 2,4-GHz-Netz des Routers. Ermitteln Sie in diesem Fall den Kanal des Routers (auf seiner Verwaltungsseite oder mit einer WLAN-Analyse-App auf dem Smartphone). Wechselt der Kanal, legen Sie ihn dort fest. Tragen Sie ihn dann unter **Adapter > WLAN-Kanäle meiden > Manuell** ein, zum Beispiel `6` oder `11`. Siehe [Verbindungsqualität und AFH](usage#link-quality).

LDAC 990 kbps verlor in einem stark belegten 2,4-GHz-Netz etwa 1.700 Pakete in 5 Minuten. Nachdem der Router auf einen festen Kanal eingestellt und AFH auf Manuell mit diesem Kanal gesetzt war, waren es nur noch 33 in 15 Minuten.

### 5. Der Verbindung Reserven lassen

Eine Bluetooth-Verbindung überträgt unter besten Bedingungen etwas mehr als 1 Mbps. LDAC 990 kbps, LHDC V5 mit 900 und 1000 kbps und SBC XQ mit hohem Bitpool lassen kaum Platz für Wiederholungen. True-Wireless-Ohrhörer leiten das Audio von einem Ohrhörer zum anderen weiter, was ebenfalls Sendezeit kostet. Ab 900 kbps wird es für sie daher selbst bei störungsfreier Verbindung schwierig.

- Probieren Sie **ABR** (adaptive Bitrate; LDAC, LHDC V5, aptX Adaptive, adaptiver Bitpool bei SBC) oder eine Stufe niedriger: LDAC 660, LHDC V5 500, SBC XQ mit Bitpool 29 bis 36.
- Prüfen Sie, ob die Aussetzer auch mit **SBC oder AAC** auftreten. Wenn ja, liegt es nicht an der Bitrate.
- Zur Gegenprobe mit den Kopfhörern: Koppeln Sie sie mit einem Smartphone, stellen Sie dort denselben Codec und dieselbe Bitrate ein und hören Sie an derselben Stelle zu. Setzt der Ton auch dort aus, liegt die Grenze bei den Kopfhörern.

Auf einer Verbindung, die 750 kbps schaffte, setzte alles über 500 kbps aus, während ABR ohne Aussetzer lief. SBC XQ über Bitpool 36 verstummte auf günstigen Ohrhörern, deren SBC-Fähigkeiten bis 53 reichten. Auf denselben Ohrhörern setzte LHDC V5 sporadisch aus, während LDAC stabil lief. Eine saubere Verbindung schafft hohe Bitraten durchaus: Der integrierte Adapter eines Notebooks streamte LDAC 990 kbps 5 Minuten lang ohne ein einziges verlorenes Paket.

### 6. BlueCaster aktualisieren

Mehrere Ruckler, die nach Funkproblemen klangen, waren Fehler in der App. 2.0.1 sendete Audio schneller als in Echtzeit, wenn ein virtuelles Kabel es verspätet lieferte. Das klang wie abwechselnd eine Sekunde Ton und eine Sekunde Stille. Nach Trennen > Verbinden stotterte der Stream, bis die App neu gestartet wurde. Und ein veralteter Bluetooth-Stack verweigerte die Verbindung nach dem Energiesparmodus, einer langen Sitzung oder vielen Neustarts. Alle diese Fehler sind im aktuellen Release behoben ([Updates](usage#updates)). Ist das Problem nach dem Update verschwunden, müssen Sie es nicht melden.

## Die Verbindungsqualität lesen {#link-quality}

**Diagnose > Verbindungsqualität** (und das Diagramm auf Start) zeigt, ob das Problem beim Funk, beim PC oder bei der App liegt. Machen Sie einen Screenshot, während das Problem auftritt, und hängen Sie ihn an Ihren Bericht an.

| Was Sie sehen | Was es bedeutet | Deutet auf |
|:--|:--|:--|
| **Signal zum Idealwert** deutlich unter 0 (etwa -25 dB oder weniger) | Das Signal der Kopfhörer kommt weit unterhalb des idealen Bereichs am Adapter an. Der Wert ist kein absoluter Pegel: 0 ist ideal, -30 liegt 30 dB darunter. Bei -30 kann der Ton also noch spielen, aber ohne jede Reserve | Funk: Entfernung, Ihr Kopf und Körper, Position des Adapters |
| **Sendewarteschlange** voll, **Vor dem Senden verworfen** steigt, Bitrate unter der des Codecs | Pakete haben zu lange auf den Funk gewartet. Die Verbindung kann diese Bitrate gerade nicht übertragen | Funk oder Störungen, oder eine zu hohe Bitrate |
| **Frequenzsprungverfahren (AFH)** springt oder bleibt bei 79, während Pakete verloren gehen | Der Adapter markiert Kanäle mit Fehlern als schlecht. Bei 79 kann er gute und schlechte Kanäle nicht mehr unterscheiden | Störungen: WLAN, andere Empfänger, ein zweiter Bluetooth-Adapter |
| **Bei Aufnahme verworfen** steigt, Start meldet **PC ausgelastet** | Der PC hat das Audio nicht rechtzeitig geliefert | Der PC: Auslastung, das Aufnahmegerät, ein virtuelles Kabel |
| Alles flach und leer, trotzdem Stille oder Stottern | Die Verbindung ist in Ordnung, und auf dem PC geht nichts verloren | Die App oder der Decoder der Kopfhörer: Melden Sie es mit dem Diagnosepaket |

Was auf der Empfängerseite passiert, kann der PC nicht sehen. Um zu messen, was tatsächlich ankommt, ersetzt der [Peer-Empfängertest](usage#receiver) die Kopfhörer durch einen Linux-PC.

## Was die Kopfhörer entscheiden {#headphones}

Manche Berichte wie „ich bekomme nur 400 kbps“ oder „LDAC fehlt“ haben nichts mit der Verbindung zu tun. Die Kopfhörer teilen BlueCaster mit, welche Codecs und Bitraten sie akzeptieren, und BlueCaster hält sich daran, genau wie Android. Die Protokollzeile `remote supports ...` zeigt, was sie angeboten haben.

- **Huawei FreeBuds** bieten einem PC LDAC erst an, wenn in ihrer App AI Life „Klangqualität bevorzugen“ eingestellt und die Mehrfachverbindung ausgeschaltet ist. Sonst bieten sie nur SBC und AAC an. Seit 2.0.2 fordert BlueCaster diesen Modus automatisch an, wenn LDAC gewählt und die automatische Auswahl ausgeschaltet ist.
- **LHDC-V5-Kopfhörer** geben eine maximale Bitrate an. Ein Paar meldete im 96-kHz-Modus seiner App 400 kbps und im 192-kHz-Modus 1000 kbps. OPPO Enco Free4 und Enco X4 melden einem PC unabhängig von der App-Einstellung 400 kbps und 48 kHz. Streamt LHDC V5 bei ausgeschaltetem ABR, fordert 2.0.2 bei OPPO-Ohrhörern Hi-Res über deren eigenes Protokoll an und verbindet sich dafür einmal neu (an echten Ohrhörern noch nicht bestätigt).
- **ABR** hält LHDC V5 bei höchstens 400 kbps, wie unter Android.
- Ein Codec, den die Kopfhörer **für ein anderes Gerät reservieren**, fehlt, solange dieses Gerät verbunden ist ([Codec fehlt, während die Kopfhörer mit einem anderen Gerät verbunden sind](usage#sep-in-use)).

So sehen Sie, was die Kopfhörer anbieten: [Prüfen, was Ihre Kopfhörer anbieten](usage#codec-fallback).

## Verbinden schlägt fehl {#connecting}

Die Fehlermeldung auf Start sagt, was passiert ist. Die Tabelle unter [Zeitüberschreitungen beim Verbinden](usage#connection-timeouts) erklärt jede Meldung. Die Fälle, die mit dem Funk zu tun haben:

- **Das Gerät hat nicht geantwortet** (Page Timeout): Das Gerät ist ausgeschaltet, außer Reichweite oder bei der ersten Verbindung nicht im Kopplungsmodus. Oder der Adapter läuft noch mit seiner ROM-Firmware. Ein Realtek-Adapter ohne Firmware findet bei der Suche nichts und läuft bei jeder Verbindung in eine Zeitüberschreitung. Oder er lässt sich öffnen, streamt dann aber nie. In einem Fall war die Firmware-Datei eine gespeicherte Webseite. Auf der Seite Adapter sehen Sie, ob die Firmware geladen ist.
- **Die Verbindung kam zustande, übertrug dann aber nichts**: Der Funkempfang ist schlecht, oder die Kopfhörer sind mit einem anderen Gerät beschäftigt. Versuchen Sie es erneut oder stecken Sie den Adapter um.
- **Authentifizierung fehlgeschlagen**, oder im Protokoll steht `signaling connection failed (0x66)`: Die Kopfhörer haben noch einen Schlüssel aus einer früheren Kopplung, meist mit dem Windows-Bluetooth dieses PCs. `(0x67)`: Sie sind mit einem anderen Gerät beschäftigt, zum Beispiel mit einem Smartphone mit eingeschaltetem LE Audio. Trennen Sie die anderen Geräte, schalten Sie LE Audio in der Kopfhörer-App aus, versetzen Sie die Kopfhörer in den Kopplungsmodus und verbinden Sie einmal. Spätere Verbindungen verwenden den gespeicherten Schlüssel.
- Die Kopfhörer **können sich nach dem Trennen nicht mit dem Smartphone verbinden**: Seit 2.0.2 gibt Trennen die Verbindung frei. Wenn Sie die Kopfhörer durch Aus- und Einschalten zwischen PC und Smartphone wechseln, schalten Sie **Automatisch wieder verbinden** aus.

## Was hinter den Berichten steckte {#reports}

Alle bisherigen Berichte zur Funkverbindung, gruppiert nach der Ursache, die sich am Ende herausstellte.

| Ursache | Berichte | Beispiele |
|:--|:--|:--|
| **Umgebung**: Funk, Position, ein anderes Gerät | Etwa ein Drittel | 2,4-GHz-WLAN auf denselben Kanälen, ein Gaming-Maus-Empfänger neben dem Adapter, ein zweiter aktiver Bluetooth-Adapter, Doppelverbindung und eine laufende Kopfhörer-App, eine Bitrate ohne Reserve auf True-Wireless-Ohrhörern, ein schwaches Signal kleiner Ohrhörer, eine Firmware-Datei, die eine gespeicherte Webseite war |
| **Kopfhörer**: was sie einem PC anbieten | Etwa ein Viertel | LDAC erst im Klangqualitätsmodus von Huawei, LHDC V5 durch den Modus der Kopfhörer auf 400 kbps begrenzt, Hi-Res über das Protokoll von OPPO eingestellt, ein für das Smartphone reservierter Codec |
| **App**: ein Fehler in BlueCaster, alle behoben | Knapp die Hälfte | Mit einem virtuellen Kabel schneller als in Echtzeit gesendetes Audio, Stottern nach Trennen > Verbinden, ein veralteter Bluetooth-Stack nach dem Energiesparmodus oder vielen Neustarts, eine nach dem Trennen gehaltene Verbindung, Kopfhörer, die zuerst verbinden und für einen falschen Schlüssel gehalten wurden, ein Codec-Cache, der LDAC verbarg, nicht erkannte Adapter (ein Chip, eine USB-Klasse, eine noch nicht aktive Treiberbindung) |

Was auffällt:

- **Stottern und Aussetzer verteilen sich etwa gleichmäßig** auf die App (Timing-Fehler in 2.0.1 und den ersten 2.0.2-Betas) und die Umgebung. Die beiden gemessenen Störungsfälle zeigen dasselbe Bild: Die gesendete Bitrate fällt auf 550 bis 800 kbps, die Sendewarteschlange läuft voll, und der Adapter markiert 79 von 79 Kanälen.
- **Verbindungsprobleme lagen meist an der App**, fast alle im selben Ablauf: Die Kopfhörer bauen nach dem Einschalten selbst die Verbindung auf. Der Bluetooth-Emulator kann diesen Ablauf nicht nachstellen, daher beruhen diese Korrekturen auf den Protokollen der Nutzer.
- **Bei „Codec fehlt“ und „Bitrate hängt fest“ entschieden die Kopfhörer**, was sie einem PC anbieten, meist gesteuert durch ihre Begleit-App.
- Manche Probleme **sahen nach Funk aus, waren es aber nicht**: Stille mit AAC bei einem Modell (das Frame-Format), die Windows-Ausgabe, die nach dem Ausschalten der Kopfhörer auf dem virtuellen Kabel blieb, einige Sekunden Ton aus den Lautsprechern, während ein Codec-Wechsel neu verbindet, Ton auch auf kabelgebundenen Kopfhörern (so funktioniert System-Loopback).

## Melden {#reporting}

Haben Sie die Ursache mit den Prüfungen oben nicht gefunden, öffnen Sie einen [Fehlerbericht](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/new?template=bug_report.yml) und haken Sie an, was Sie ausprobiert haben. Folgendes hilft, einen Bericht zur Funkverbindung schnell zu klären:

1. Stellen Sie unter **Diagnose > Problem melden** die **Ausführliche Protokollierung** auf **Detailliert (debug)** und stellen Sie dann das Problem nach.
2. Das **Diagnosepaket** (**Diagnosepaket > Speichern…** mit eingeschaltetem **HCI-Mitschnitte einschließen**, dann **Auf GitHub melden**). Es enthält die Zeilen zum Streamzustand (Sendewarteschlange, verworfene Pakete, Signal, AFH-Kanäle im Zeitverlauf) und den HCI-Mitschnitt einer fehlgeschlagenen Verbindung. Daran ist zu sehen, wo die Pakete hängen geblieben sind.
3. Ein **Screenshot der Verbindungsqualität**, während das Problem auftritt.
4. Ob es auch mit **SBC oder AAC** und bei einer **niedrigeren Bitrate** auftritt.
5. Ihre Antworten auf die Fragen im Formular zu Doppelverbindung, weiterem Bluetooth-Adapter und WLAN.

---
title: Coupures et problèmes de connexion
layout: default
parent: Accueil
nav_order: 2.4
---

# Coupures et problèmes de connexion
{: .no_toc }

Le son se coupe, saccade ou s'arrête, la connexion échoue, ou le débit est plus bas que prévu ? Cette page résume les causes trouvées lors des rapports précédents, pour que vous puissiez vérifier les plus courantes avant de signaler le problème. Si aucune ne correspond, elle indique ce qu'il faut nous envoyer.

## Table des matières
{: .no_toc .text-delta }

1. TOC
{:toc}

## À vérifier d'abord {#checklist}

Le Bluetooth partage la bande des 2,4 GHz avec le Wi-Fi, les souris et claviers sans fil, les dongles de casques et toutes les autres radios Bluetooth de la pièce. Un adaptateur USB a une petite antenne, et des écouteurs une encore plus petite, avec votre tête entre eux et le PC. La plupart des rapports de « mauvaise connexion » reçus jusqu'ici venaient de l'une des causes ci-dessous, et chacune se vérifie en une minute environ. Le formulaire de rapport de bogue a une case pour chacune : cochez celles que vous avez essayées.

{: .note }
Testez-les une par une et écoutez quelques minutes entre deux : si les coupures cessent, vous avez trouvé la cause.

### 1. Désactivez les autres adaptateurs Bluetooth de ce PC

Le Bluetooth de Windows (l'adaptateur intégré) continue d'émettre pendant que BlueCaster diffuse par l'adaptateur USB voisin. Les deux partagent la même bande, et souvent la même zone d'antenne. L'application du casque sur le PC se connecte aussi par l'adaptateur intégré, ce qui donne au casque une seconde liaison (voir le point suivant). Désactivez l'adaptateur intégré dans le Gestionnaire de périphériques pendant que BlueCaster fonctionne, ou au moins le temps du test. BlueCaster affiche un avertissement sur l'Accueil quand il détecte un autre adaptateur actif.

Un testeur de longue date a désigné un second adaptateur actif comme l'une des deux causes de connexions instables et de changements de codec intempestifs.

### 2. Ne laissez au casque qu'une seule connexion

La double connexion (multipoint) permet au casque de rester connecté à la fois à un téléphone et au PC. Chaque connexion consomme du temps d'antenne. Tant que le multipoint est activé, certains casques proposent aussi moins de codecs, refusent la liaison mains libres (donc pas de niveau de batterie) ou réservent un endpoint de codec à l'autre appareil. L'application du casque compte aussi : elle garde sa propre liaison ouverte, sur le téléphone ou sur le Bluetooth de ce PC, même si vous ne l'utilisez que pour la batterie ou l'égaliseur.

Pour le test : désactivez la double connexion dans l'application du casque, désactivez le Bluetooth du téléphone, fermez l'application du casque sur le téléphone et sur le PC, puis reconnectez-vous depuis BlueCaster.

La double connexion est l'autre cause citée par ce testeur : les changements de codec sont devenus bien plus fluides une fois l'application du casque déconnectée. Un casque a refusé une première connexion tant qu'il restait connecté à un téléphone avec LE Audio activé. Plusieurs écouteurs ont proposé à BlueCaster moins de codecs ou un débit plus bas tant qu'un téléphone ou leur application était connecté. Les casques Sony désactivent le LDAC tant que le multipoint est activé.

### 3. Déplacez l'adaptateur

- Utilisez un **port USB 2.0**, ou un court câble de rallonge USB. Les ports et câbles USB 3 rayonnent du bruit dans la bande des 2,4 GHz, exactement là où se trouve l'adaptateur.
- Tenez-le **à l'écart des autres récepteurs sans fil** : dongles de souris et de clavier gaming, dongles de casques sans fil, antennes Wi-Fi. Un récepteur branché sur le port voisin provoque souvent exactement ce type de coupures.
- Gardez-le **en vue du casque**. Un PC posé sous le bureau, avec vous et le bureau entre lui et le casque, perd beaucoup de signal. Une rallonge USB qui remonte l'adaptateur sur le bureau aide plus que tout le reste.

Sur un PC sans adaptateur Wi-Fi pour guider l'AFH, un récepteur de souris gaming placé à côté de l'adaptateur limitait la liaison à environ 750 kbps, et l'adaptateur marquait jusqu'à 46 des 79 canaux comme mauvais. Des écouteurs à un mètre de l'adaptateur avaient encore un signal faible, assez pour provoquer une coupure totale toutes les quelques minutes près de la limite de débit.

### 4. Éloignez le Wi-Fi 2,4 GHz de la liaison

Mettez ce PC, et si possible le téléphone et les autres appareils proches, en Wi-Fi 5 ou 6 GHz. Si le 2,4 GHz doit rester activé, BlueCaster indique à l'adaptateur d'éviter le canal qu'utilise le Wi-Fi de ce PC (AFH, Auto). BlueCaster ne peut pas connaître ce canal quand le PC n'a pas d'adaptateur Wi-Fi, est en filaire ou utilise une autre bande que le réseau 2,4 GHz du routeur. Dans ce cas, trouvez le canal du routeur (sur sa page d'administration, ou avec une application d'analyse Wi-Fi sur le téléphone), fixez-le dans le routeur s'il change, et saisissez-le sous **Adaptateur > Éviter les canaux Wi-Fi > Manuel**, par exemple `6` ou `11`. Voir [Qualité de liaison et AFH](usage#link-quality).

LDAC 990 kbps a perdu environ 1 700 paquets en 5 minutes sur un réseau 2,4 GHz chargé, et seulement 33 en 15 minutes une fois le routeur fixé sur un seul canal et l'AFH réglé sur Manuel avec ce canal.

### 5. Laissez de la marge à la liaison

Une liaison Bluetooth transporte un peu plus de 1 Mbps dans le meilleur des cas. LDAC 990 kbps, LHDC V5 900 et 1000 kbps et SBC XQ à un bitpool élevé ne laissent presque rien pour les retransmissions. Les écouteurs true wireless relaient l'audio d'un écouteur à l'autre, ce qui consomme aussi du temps d'antenne : 900 kbps et plus leur posent donc problème, même sur une liaison propre.

- Essayez l'**ABR** (débit adaptatif ; LDAC, LHDC V5, aptX Adaptive, bitpool adaptatif pour SBC), ou un cran plus bas : LDAC 660, LHDC V5 500, SBC XQ bitpool 29 à 36.
- Vérifiez si les coupures persistent en **SBC ou AAC**. Si c'est le cas, le débit n'est pas en cause.
- Pour tester le casque : associez-le à un téléphone, réglez-y le même codec et le même débit, et écoutez au même endroit. Si le son se coupe là aussi, la limite vient du casque.

Sur une liaison limitée à 750 kbps, tout débit supérieur à 500 kbps provoquait des coupures, alors que l'ABR n'en avait aucune. Au-delà du bitpool 36, SBC XQ devenait muet sur des écouteurs d'entrée de gamme dont la capacité SBC autorisait 53. Sur les mêmes écouteurs, LHDC V5 se coupait de façon aléatoire alors que LDAC restait stable. Une liaison propre supporte ces débits : l'adaptateur intégré d'un portable a diffusé du LDAC 990 kbps pendant 5 minutes sans perdre un seul paquet.

### 6. Mettez BlueCaster à jour

Plusieurs saccades qui semblaient venir de la radio étaient en fait des bogues de l'application. La 2.0.1 envoyait l'audio plus vite que le temps réel quand un câble virtuel le livrait en retard : on entendait alors une seconde de son, puis une seconde de silence, en alternance. Après Déconnecter > Connecter, le flux saccadait jusqu'au redémarrage de l'application. Une pile Bluetooth restée dans un état périmé refusait de se connecter après une veille, une longue session ou de nombreux redémarrages. Tout cela est corrigé dans la version actuelle ([Mises à jour](usage#updates)). Si le problème a disparu après la mise à jour, aucun rapport n'est nécessaire.

## Lire la Qualité de liaison {#link-quality}

**Diagnostic > Qualité de liaison** (et le graphique de l'Accueil) indique si le problème vient de la radio, du PC ou de l'application. Prenez une capture d'écran pendant que le problème se produit et joignez-la à un rapport.

| Ce que vous voyez | Ce que cela signifie | Cause probable |
|:--|:--|:--|
| **Signal par rapport à l'idéal** bien en dessous de 0 (environ -25 dB ou moins) | Le signal du casque arrive à l'adaptateur bien en dessous de la plage idéale de l'adaptateur. Ce n'est pas un niveau absolu : 0 est l'idéal et -30 se situe 30 dB en dessous ; à -30, le son passe encore, mais sans aucune marge | Radio : distance, votre tête et votre corps, emplacement de l'adaptateur |
| **File d'envoi** pleine, **Paquets abandonnés avant envoi** en hausse, débit inférieur à celui du codec | Les paquets ont attendu la radio trop longtemps ; la liaison ne peut pas transporter ce débit pour l'instant | Radio ou interférences, ou un débit trop élevé |
| **Saut de fréquence (AFH)** qui varie fortement, ou bloqué à 79 alors que des paquets sont perdus | L'adaptateur marque comme mauvais les canaux où il constate des erreurs ; à 79, il a renoncé à les distinguer | Interférences : Wi-Fi, autres récepteurs, un second adaptateur Bluetooth |
| **Audio perdu à la capture** en hausse, l'Accueil indique **PC occupé** | Le PC n'a pas livré l'audio à temps | Le PC : charge, le périphérique de capture, un câble virtuel |
| Tout est plat et vide, mais silence ou saccades | La liaison va bien et rien n'est perdu sur le PC | L'application ou le décodeur du casque : signalez-le avec le paquet de diagnostic |

Le PC ne voit pas ce qui se passe côté réception. Pour mesurer ce qui arrive réellement, le [test avec récepteur pair](usage#receiver) remplace le casque par un PC Linux.

## Ce que le casque décide {#headphones}

Certains rapports du type « je n'ai que 400 kbps » ou « LDAC n'apparaît pas » n'ont rien à voir avec la liaison : c'est le casque qui indique à BlueCaster les codecs et les débits qu'il accepte, et BlueCaster s'y conforme, comme Android. La ligne de journal `remote supports ...` montre ce qu'il a proposé.

- Les **Huawei FreeBuds** ne proposent LDAC à un PC qu'une fois que leur application AI Life a réglé « privilégier la qualité sonore » avec la multiconnexion désactivée ; sinon ils ne listent que SBC et AAC. Depuis la 2.0.2, BlueCaster demande automatiquement ce mode quand LDAC est choisi avec la sélection automatique désactivée.
- Les **casques LHDC V5** annoncent un débit maximal. En mode 96 kHz de leur application, une paire listait 400 kbps ; en mode 192 kHz, 1000 kbps. Les OPPO Enco Free4 et Enco X4 annoncent 400 kbps et 48 kHz à un PC, quel que soit le réglage de leur application. La 2.0.2 demande le Hi-Res aux écouteurs OPPO par leur propre protocole quand LHDC V5 diffuse avec l'ABR désactivé, et se reconnecte une fois pour l'appliquer (pas encore confirmé sur de vrais écouteurs).
- L'**ABR** maintient LHDC V5 à 400 kbps ou moins, comme sur Android.
- Un codec que le casque **réserve à un autre appareil** reste absent tant que cet appareil est connecté ([Codec absent quand le casque est connecté à un autre appareil](usage#sep-in-use)).

Comment voir ce qu'il propose : [Vérifier ce que propose votre casque](usage#codec-fallback).

## La connexion échoue {#connecting}

L'erreur affichée sur l'Accueil indique ce qui s'est passé ; le tableau de [Délais de connexion dépassés](usage#connection-timeouts) explique chaque message. Les cas liés à la radio :

- **L'appareil n'a pas répondu** (Page Timeout) : l'appareil est éteint, hors de portée ou n'est pas en mode d'association pour une première connexion, ou l'adaptateur fonctionne encore avec son firmware ROM. Un adaptateur Realtek sans son firmware ne trouve rien lors d'une recherche et dépasse le délai à chaque connexion, ou bien s'ouvre normalement mais ne diffuse jamais. Dans un cas, le fichier de firmware était en réalité une page web enregistrée. L'écran Adaptateur indique si le firmware est chargé.
- **La connexion s'est établie mais n'a rien transmis** : conditions radio, ou le casque est occupé avec un autre appareil. Réessayez, ou déplacez l'adaptateur.
- **Échec de l'authentification**, ou le journal indique `signaling connection failed (0x66)` : le casque a gardé la clé d'une association précédente, en général avec le Bluetooth Windows de ce même PC. `(0x67)` : il est occupé avec un autre appareil, par exemple un téléphone avec LE Audio activé. Déconnectez les autres appareils, désactivez LE Audio dans l'application du casque, mettez-le en mode d'association et connectez-vous une fois ; les connexions suivantes utilisent la clé enregistrée.
- Le casque **ne peut pas se connecter au téléphone** après Déconnecter : depuis la 2.0.2, Déconnecter libère la liaison. Si vous faites passer le casque du PC au téléphone en l'éteignant puis en le rallumant, désactivez **Reconnecter automatiquement**.

## Ce que les rapports se sont avérés être {#reports}

Tous les rapports liés au sans-fil reçus jusqu'ici, regroupés selon leur cause réelle.

| Cause | Rapports | Exemples |
|:--|:--|:--|
| **Environnement** : la radio, l'emplacement, un autre appareil | Environ un tiers | Wi-Fi 2,4 GHz sur les mêmes canaux, un récepteur de souris gaming à côté de l'adaptateur, un second adaptateur Bluetooth actif, la double connexion et une application du casque en cours d'exécution, un débit sans marge sur des écouteurs true wireless, un signal faible de petits écouteurs, un fichier de firmware qui était une page web enregistrée |
| **Casque** : ce qu'il propose à un PC | Environ un quart | LDAC seulement après le mode qualité sonore de Huawei, LHDC V5 limité à 400 kbps par le mode propre du casque, le Hi-Res réglé par le protocole d'OPPO, un codec réservé au téléphone |
| **Application** : un bogue dans BlueCaster, tous corrigés | Un peu moins de la moitié | Audio envoyé plus vite que le temps réel avec un câble virtuel, saccades après Déconnecter > Connecter, une pile Bluetooth périmée après une veille ou de nombreux redémarrages, la liaison conservée après Déconnecter, un casque qui se connecte en premier pris pour une mauvaise clé, un cache de codecs qui masquait LDAC, des adaptateurs non reconnus (une puce, une classe USB, une liaison de pilote pas encore active) |

Ce qui ressort :

- **Saccades et coupures se répartissent à peu près à égalité** entre l'application (bogues de synchronisation dans la 2.0.1 et les premières bêtas de la 2.0.2) et l'environnement. Les deux cas d'interférences mesurés montrent le même tableau : le débit envoyé tombe à 550 à 800 kbps, la file d'envoi se remplit, et la carte des canaux de l'adaptateur passe à 79 sur 79.
- **Les problèmes de connexion venaient surtout de l'application**, et presque tous sur un même chemin : le casque qui ouvre lui-même la connexion après avoir été allumé. L'émulateur Bluetooth ne peut pas reproduire ce cas : ces correctifs reposent donc sur les journaux des utilisateurs.
- **« Codec absent » et « débit bloqué », c'était le casque** qui décidait de ce qu'il propose à un PC, en général sous l'influence de son application.
- Certains problèmes **ressemblaient à de la radio mais n'en étaient pas** : silence en AAC sur un modèle (le format des trames), la sortie Windows laissée sur le câble virtuel après l'extinction du casque, quelques secondes de son sur les haut-parleurs pendant qu'un changement de codec reconnecte, du son aussi sur un casque filaire (le fonctionnement du Bouclage système).

## Signaler {#reporting}

Si les vérifications ci-dessus n'ont pas permis de trouver la cause, ouvrez un [rapport de bogue](https://github.com/SeiyaFunaokaJP/BlueCaster/issues/new?template=bug_report.yml) et cochez ce que vous avez essayé. Ces éléments permettent de résoudre rapidement un problème sans fil :

1. Dans **Diagnostic > Signaler un problème**, réglez **Journalisation détaillée** sur **Détaillée (debug)**, puis reproduisez le problème.
2. Le **paquet de diagnostic** (**Paquet de diagnostic > Enregistrer…** avec **Inclure les captures HCI** activé, puis **Signaler sur GitHub**). Il contient les lignes de santé du flux (file d'envoi, paquets abandonnés, signal, canaux AFH au fil du temps) et la capture HCI d'une connexion échouée, qui montrent où les paquets se sont arrêtés.
3. Une **capture d'écran de Qualité de liaison** prise pendant le problème.
4. Si le problème se produit aussi en **SBC ou AAC**, et à un **débit plus bas**.
5. Vos réponses aux questions du formulaire sur la double connexion, un autre adaptateur Bluetooth et le Wi-Fi.

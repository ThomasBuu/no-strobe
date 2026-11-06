# Strobes lents (lampes de police) : applis de la branche `ios`

Contexte : la police utilise des lampes torches en mode strobe, à fréquence quasi fixe (~15 Hz mesurés sur une
vidéo réelle), pour empêcher de filmer. Ce n'est pas le cœur de `no-strobe` (LED PWM de concert, 100 Hz–5 kHz), mais
c'est le chemin « strobe » que le README classe expérimental. Cette branche ajoute trois applis autonomes (sans le
cœur Rust) qui traitent ce cas, et note ce que les deux approches peuvent s'apporter.

## Contenu

| Dossier | Quoi | Plateforme |
|---|---|---|
| `platform/ios/app/AntiStrobe` | filme à 240 i/s, détecte le strobe, ne garde que les images prises lampe éteinte (30 i/s) + version brute | iOS 18 |
| `platform/ios/app/Flasher` | « Strobe » : fait clignoter la torche de l'iPhone (1–30 Hz, rapport cyclique réglable) pour les tests | iOS 18 |
| `platform/ios/app/check` | vérification hors appareil du tri des images (simulation) | macOS (`swiftc`) |
| `platform/android-strobe` | caméra 30 i/s calée en phase sur le strobe : une image par cycle, toujours lampe éteinte | Android 10+ |
| `tools/android-strobe-flasher` | fait clignoter le flash d'un téléphone Android pour les tests | Android 10+ |

## Deux approches selon le téléphone

**iPhone (240 i/s) : sur-échantillonner puis trier.** iOS ne permet pas de déclencher chaque image à un instant
précis, mais filme à 240 i/s. Avec une pose courte (1/1000 s par défaut) et l'exposition verrouillée, à peu près la
moitié des images tombent lampe éteinte.
- Détection : luminance de 8 bandes horizontales par image (le rolling shutter fait qu'un flash peut n'éclairer
  qu'une partie), DFT de la luminance moyenne sur 2 s (2–60 Hz), avec un passe-haut contre les mouvements de caméra et
  une fenêtre de Hann.
- Tri par phase : chaque image est placée dans le cycle du strobe ; une image est gardée si la luminance moyenne de sa
  phase est dans le bas du profil et si elle n'est pas plus claire que la dernière image propre.
- Sortie à cadence fixe : pour chaque créneau de 1/30 s, l'image propre la plus proche (`FramePacer`).
- Deux fichiers par enregistrement : la version traitée et la brute (toutes les images à 240 i/s) pour ne rien perdre.

**Android sans haute cadence : se caler sur le strobe.** Beaucoup de téléphones plafonnent à 30 i/s en caméra
arrière. `StrobeLock` (Kotlin pur, testé hors appareil) :
- Mesure : chaque bande de l'image est lue à un instant connu (`SENSOR_TIMESTAMP` + `SENSOR_ROLLING_SHUTTER_SKEW`),
  donc une image donne 16 échantillons datés. Ça lève l'ambiguïté d'une caméra à 30 i/s : 14,3 Hz et 15,7 Hz s'y
  ressemblent image par image, mais la lumière ne progresse pas dans le même sens d'une bande à l'autre.
- Accrochage : une image par cycle qui glisse lentement dans le cycle jusqu'à des images entièrement sombres.
- Suivi : les images elles-mêmes donnent l'erreur (« early-late gate ») : lumière sur les premières bandes = trop tôt,
  sur les dernières = trop tard ; recentrage d'un coup au milieu de la plage éteinte, fréquence affinée par la dérive.
  Une image « sonde » en pleine phase allumée toutes les 30 images vérifie que le strobe est toujours là.
- Limite physique : une image entière (lecture + pose) doit tenir dans la plage éteinte. Avec 25 ms de lecture et un
  strobe allumé la moitié du temps, ça marche jusqu'à ~19 Hz ; au-delà il faut un capteur plus rapide.

## Ce qui est vérifié (et ce qui ne l'est pas)

- iOS AntiStrobe : compile ; tri vérifié en simulation (`check/`, 5–30 Hz, caméra qui bouge, sans strobe) ; testé sur
  iPhone 16 Pro Max avec un vrai flash à 5 Hz (premier résultat bon, version actuelle pas encore re-testée sur appareil).
- iOS Flasher : installé et lancé sur iPhone 16 Pro Max.
- `StrobeLock` : 10 tests de simulation (rolling shutter, latence du pipeline 2 à 6 images, bruit, 6–40 Hz, flashs
  courts, 10 graines à 14,3 et 15,7 Hz) : aucune image avec flash gardée, 75 % des cycles ou plus donnent une image
  propre en régime établi, calage en moins de 6 s.
- Android sur Unihertz Jelly Star (MT6789) : l'appli tourne, mesure la lumière, enregistre les deux vidéos avec le son.
  **Mais le calage y est impossible** : ce capteur ignore `SENSOR_FRAME_DURATION` (les images restent à 33,3 ms quelle
  que soit la durée demandée, même avec `CONTROL_MODE_OFF` et `CONTROL_CAPTURE_INTENT_MANUAL`) ; seules les plages de
  cadence fixes (10/15/20/30 i/s) sont respectées. Sa lecture dure aussi 31,2 ms, ce qui laisserait moins de 1 ms de
  marge à 14,3 Hz. L'appli le détecte et l'affiche. Aucun téléphone Android qui respecte la durée d'image n'a été testé.
- Android : les deux encodeurs copient l'image YUV pixel par pixel pour la chrominance ; pendant l'enregistrement la
  cadence tombe à ~15 i/s sur le Jelly. À optimiser.

## Ce que `no-strobe` peut reprendre d'ici

1. **Clé de rolling shutter.** `Camera2Sync.kt` lit `android.sensor.info.rolling_shutter_sustain_duration`, qui n'est
   pas une clé de l'API Android publique (à vérifier sur vos appareils) : d'où `sustain = 0` et le mode « aveugle » sur
   le Xperia et le Pixel. La clé standard est `CaptureResult.SENSOR_ROLLING_SHUTTER_SKEW` (par image, API 21) :
   31,2 ms sur le Jelly Star. Elle donnerait directement `line_rate = hauteur / skew` au chemin PWM.
2. **Strobes au-delà de fps/2.** Le chemin strobe (`controller.rs`) cherche sur la luminance moyenne des images, donc
   jusqu'à `0,4 × fps` (12 Hz à 30 i/s) : un strobe de police à 14–16 Hz lui échappe à 30 i/s. Les bandes datées par le
   rolling shutter (comme dans `StrobeLock`) prolongent la détection bien au-delà.
3. **Mode Gap calé en phase.** `solver.rs` choisit une pose dans la plage éteinte mais laisse la fenêtre dériver dans
   le cycle (`drift_us_per_frame` n'est qu'un score). Sans contrôle de l'instant de prise de vue, la fenêtre retombe
   régulièrement sur le flash. Le suivi early-late de `StrobeLock` corrige ça, quand le capteur respecte
   `SENSOR_FRAME_DURATION`.
4. **Vérifier que la durée d'image est appliquée** (point 3 de la roadmap) : `StrobeCamera.kt` compare la durée
   demandée à l'écart réel entre `SENSOR_TIMESTAMP` successifs, pas à la valeur renvoyée dans le résultat (le Jelly
   Star renvoie la valeur demandée tout en restant à 30 i/s).

## Ce que ces applis peuvent reprendre de `no-strobe`

1. **Mode Flash** (`solver.rs`) : capturer exactement un flash par image. La nuit, quand la lampe de police est la seule
   lumière, les images « lampe éteinte » sont noires ; un flash constant par image donne une vidéo exploitable.
2. **Restauration après coup** (`restorer.rs`, `no-strobe-post`) : gain par image et par ligne, applicable aux versions
   brutes enregistrées par les deux applis.
3. **Haute cadence Android** : `main` enregistre déjà en haute cadence à la fréquence que le scintillement permet.
   Le même chemin rendrait possible l'approche iPhone (sur-échantillonner puis trier) pour les strobes lents, sans
   contrôle de phase, sur les téléphones qui ignorent `SENSOR_FRAME_DURATION`.
4. **Front montant pour la phase** (`detector.rs`) : un point par cycle au lieu de chaque image au-dessus du seuil.

## Construire

```sh
# iOS (macOS, Xcode, xcodegen) ; choisir son équipe de signature dans Xcode (pas de DEVELOPMENT_TEAM dans project.yml)
cd platform/ios/app && xcodegen generate && open AntiStrobe.xcodeproj
cd platform/ios/app/check && swiftc ../AntiStrobe/StrobeAnalyzer.swift main.swift -o /tmp/check && /tmp/check

# Android (sdk.dir dans local.properties de chaque projet)
cd platform/android-strobe && ./gradlew testDebugUnitTest assembleDebug
cd tools/android-strobe-flasher && ./gradlew assembleDebug
```

Avertissement : un strobe peut déclencher des crises chez les personnes épileptiques photosensibles. Les applis de test
font clignoter une lumière ; à utiliser avec précaution.

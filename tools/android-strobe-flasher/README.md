# Strobe Flasher

Appli Android de banc d'essai : fait clignoter la torche (et option écran blanc) à 1–30 Hz, rapport cyclique 10–90 %, pour simuler une lampe stroboscopique devant l'appli iOS `strobe/ios/`. Affiche la fréquence demandée et la fréquence mesurée (latence de `setTorchMode`).

⚠ Risque pour les personnes atteintes d'épilepsie photosensible.

```bash
./gradlew assembleDebug && adb install -r app/build/outputs/apk/debug/app-debug.apk
```

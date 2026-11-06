# Contribuer / Contributing

[🇫🇷 Français](#-guide-de-contribution-français) | [🇬🇧 English](#-contributing-guide-english)

---

## 🇫🇷 Guide de Contribution (Français)

Le projet filme des concerts : l'application mesure le scintillement des projecteurs et écrans LED et ajuste le temps d'exposition du capteur caméra sur un multiple entier de périodes de scintillement, afin que chaque image et chaque ligne du capteur reçoivent exactement la même quantité de lumière.

Le mode principal supporté est le **scintillement PWM de 100 Hz à 5 kHz** (projecteurs LED, murs d'écrans de scène). Le mode stroboscopique (1–25 Hz) est expérimental et secondaire.

---

### Environnement de développement

```sh
cargo test --workspace                                   # Le cœur algorithmique Rust (agnostique)
cd platform/android
JAVA_HOME=/path/to/jdk gradle assembleDebug              # L'application Android (APK)
```

Pour compiler les bibliothèques natives Android avec `cargo-ndk` :

```sh
export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/<version>
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -o platform/android/app/src/main/jniLibs \
    build -p ns-ffi --release
```

Le dossier `jniLibs` est généré lors du build et n'est pas versionné dans git.
L'outil desktop de post-traitement nécessite `ffmpeg` et `ffprobe` dans votre PATH (`cargo build --release -p ns-post`).

---

### Gestion des branches

- `main` — La branche stable de référence. L'intégration continue (CI) compile l'APK et publie automatiquement les versions *Nightly*.
- `test-for-android` — `main` avec les traces de débogage et sondes matérielles. **Tout ce qui se trouve dans `crates/ns-core/examples/` appartient à cette branche et ne doit jamais aller sur `main`.**

Gardez les branches dans le même historique linéaire. Ne faites pas de rebase de l'une sur l'autre.

---

### Règles pour les commits

- **Signez chaque commit** avec votre clé GPG (`git commit -S`, `commit.gpgsign = true`). Les commits non signés sont rejetés lors de la revue.
- **Des commits propres et professionnels.** Aucun trailer automatisé ou lien externe non pertinent. Des messages clairs, soignés et utiles.
- **Format du message** :
  - *Sujet* : à l'impératif, décrivant une seule chose, moins de 70 caractères.
  - *Corps* : expliquez **pourquoi** le changement est fait, ce qui a été mesuré, et ce qui échouerait sans cela. Un diff qui indique seulement "fix bug" ne peut pas être relu efficacement.
- **Un commit par idée** : Ne mélangez pas refactorisation cosmétique et changement de comportement dans un même commit.
- **Jamais de force push (`git push -f`)** sur les branches partagées.

---

### Avant de soumettre une Pull Request

```sh
cargo test --workspace
cd platform/android && ./gradlew testDebugUnitTest
```

Les deux suites de tests doivent passer à 100 %.
Ajoutez des tests unitaires pour tout changement : le cœur mathématique est testé sur des profils synthétiques (`crates/ns-core/tests/`), et la logique Android dans `platform/android/app/src/test/`.
Conservez le moteur indépendant : `crates/ns-core` ne doit comporter aucune dépendance liée à Android ou iOS. Le code spécifique aux plateformes réside dans `platform/`.

---

### Test sur un smartphone réel

L'émulateur Android ne supporte pas l'exposition manuelle (`MANUAL_SENSOR`). Le comportement de la boucle fermée doit donc être vérifié sur du vrai matériel :

```sh
adb -s <serial> install -r -g platform/android/app/build/outputs/apk/debug/app-debug.apk
adb -s <serial> shell am force-stop dev.nostrobe
adb -s <serial> shell am start -n dev.nostrobe/.MainActivity --es sim_hz 500
adb -s <serial> logcat -d -s nostrobe:V
```

---

### Sécurité et respect des lieux

- L'application réduit le scintillement **dans la vidéo enregistrée**, pas dans la salle de concert. Conservez l'avertissement d'épilepsie photosensible dans l'interface.
- `no-strobe-post serve` exécute `ffmpeg`. Ne l'exposez jamais directement sur Internet sans réseau sécurisé.
- Respectez les droits à l'image et les règles de captation des salles et des artistes.

---

<br/>

---

## 🇬🇧 Contributing Guide (English)

The project films concerts: the app measures LED flicker and sets the sensor exposure to a whole number of flicker cycles (`exposure = k/f`), so frames and sensor rows integrate the exact same light: no horizontal banding, no brightness pumping.

The supported mode is **PWM flicker from 100 Hz to 5 kHz** (LED projectors, stage screens). The strobe mode (1–25 Hz) is experimental and secondary.

---

### Getting set up

```sh
cargo test --workspace                                   # Platform-free core engine
cd platform/android
JAVA_HOME=/path/to/jdk gradle assembleDebug              # Android APK
```

Build phone native libraries using `cargo-ndk`:

```sh
export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/<version>
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -o platform/android/app/src/main/jniLibs \
    build -p ns-ffi --release
```

`jniLibs` is generated build output and is not tracked in git. The desktop post-processing tool requires `ffmpeg` and `ffprobe` in PATH (`cargo build --release -p ns-post`).

---

### Branches

- `main` — Production-ready code. CI builds the APK and publishes nightly releases.
- `test-for-android` — `main` plus hardware probes and debug traces. **Anything under `crates/ns-core/examples/` belongs strictly here, never on `main`.**

---

### Commits

- **Sign every commit** with your GPG key (`git commit -S`, `commit.gpgsign = true`).
- **Clean and professional commits.** No third-party automated trailers, no irrelevant external links. Clear, professional commit messages.
- **Message format**:
  - *Subject*: Imperative, single focus, under 70 characters.
  - *Body*: Explain **why** the change was made, what was measured, and what breaks without it.
- **One commit per concept.** Keep functional logic separate from styling or formatting.
- **Never force push** to shared branches.

---

### Before opening a Pull Request

```sh
cargo test --workspace
cd platform/android && ./gradlew testDebugUnitTest
```

Both test suites must pass 100%. Add unit tests for any new or modified behaviour.

---

### Real device testing

The Android emulator lacks `MANUAL_SENSOR` capabilities. Closed-loop flicker tracking must be tested on physical hardware:

```sh
adb -s <serial> install -r -g platform/android/app/build/outputs/apk/debug/app-debug.apk
adb -s <serial> shell am force-stop dev.nostrobe
adb -s <serial> shell am start -n dev.nostrobe/.MainActivity --es sim_hz 500
adb -s <serial> logcat -d -s nostrobe:V
```

# no-strobe

[🇫🇷 Français](#-version-française) | [🇬🇧 English](#-english-version)

---

## 🇫🇷 Version Française

Application caméra pour Android et iOS conçue pour filmer des concerts sans scintillement ni bandes parasites (*banding*) causées par les éclairages LED et écrans de scène. Elle mesure la fréquence de modulation PWM de la source lumineuse et cale l'exposition du capteur sur un multiple entier exact de la période de scintillement (`exposition = k/f`), garantissant que chaque trame et chaque ligne du capteur reçoivent la même quantité de lumière.

**Périmètre : scintillement PWM / LED (100 Hz – 5 kHz).** C'est le mode opérationnel principal.

| Mode | Source lumineuse | Fréquence | Statut |
|---|---|---|---|
| **PWM** | Projecteurs LED, panneaux de scène, écrans | 100 Hz – 5 kHz | **Cible principale**, temps réel sur Android et iOS |
| **Strobe** | Éclairs délibérés lents | 1 – 25 Hz | Expérimental (voir *Modes stroboscopiques*) |

Un outil de post-traitement sur ordinateur permet également de corriger les vidéos enregistrées sans synchronisation (voir *Post-traitement*).

Pour installer l'application sur votre smartphone sans passer par les stores, consultez le [Guide d'installation pas à pas](INSTALL.md).

---

### Pourquoi la synchronisation d'exposition fonctionne

- **Le scintillement temporel** provient du fait que des trames successives intègrent un nombre variable de cycles lumineux. Si la fenêtre d'exposition est un multiple entier exact de la période $T = 1/f$, chaque trame intègre exactement la même énergie lumineuse : le pompage de luminosité disparaît, quelle que soit la phase.
- **Le banding de rolling shutter** provient du décalage de lecture temporelle entre les lignes du capteur. Chaque ligne est exposée pendant `exposition + sustain`, où `sustain` correspond au temps de lecture ligne (`SENSOR_INFO_ROLLING_SHUTTER_SUSTAIN_DURATION` sur Android). Si cette fenêtre traverse plusieurs cycles lumineux, le haut et le bas de l'image voient des phases différentes, créant des barres horizontales. Choisir `exposition = k/f` annule ces barres.
- **Convergence spatiale sub-seconde (< 500 ms)** : Au lieu d'un balayage empirique lent de 60 secondes, l'autocorrélation spatiale robuste (filtrage passe-haut laplacien, seuillage MAD et corrélation de Pearson) extrait la fréquence fondamentale $f$ du PWM en une seule trame (33 ms).
- **Filtrage prédictif (PLL logicielle)** : Dans Camera2, une commande d'exposition envoyée à la trame $N$ n'est appliquée par le capteur qu'à la trame $N+2$ ou $N+3$ (profondeur de pipeline). Une boucle à verrouillage de phase (PLL du 2nd ordre) suit la phase du scintillement et compense ce retard avec 2 à 3 trames d'avance.

---

### Modes stroboscopiques (Expérimental)

> Considérez ce mode comme expérimental et secondaire par rapport au flicker LED. La synchronisation d'exposition ne peut pas éliminer un stroboscope très lent lorsque `exposition = k·T` dépasse la durée de la trame (ex. 10 Hz à 30 fps).

| Mode | Exposition | Effet produit |
|---|---|---|
| `Integrate` | $k \cdot T$ | Énergie stroboscopique uniforme entre les trames, image lumineuse |
| `Gap` | intervalle sombre − sustain | Éclairs stroboscopiques totalement occultés |
| `Flash` | fenêtre d'un éclair | Chaque trame capture exactement un seul flash |

---

### Mode PWM (Temps Réel)

La luminance moyenne par image ne permet pas d'observer les fréquences supérieures à $f_{ps}/2$ (à 30 fps, tout ce qui dépasse 15 Hz subit un repliement de spectre). Le mode PWM analyse donc le **profil de luminance ligne par ligne** :
1. `LineProfileExtractor` extrait le profil moyen de luminance par ligne depuis le plan YUV.
2. `robust_period_rows` applique un filtre laplacien spatial $\Delta^2$, centre le signal, et calcule la corrélation de Pearson pour détecter la période $T$ en nombre de lignes en moins d'une trame.
3. Le contrôleur bascule instantanément à `k/f`, calcule la compensation ISO pour maintenir l'exposition globale, et engage la PLL pour maintenir le calage de phase.
4. Si le temps de lecture ligne est inconnu (iOS), le contrôleur effectue une descente de gradient sur l'amplitude de banding résiduelle.

---

### Arborescence du projet

```
Cargo.toml             Workspace Rust : ns-core, ns-ffi, ns-post
crates/ns-core/        Cœur algorithmique pur sans dépendance externe
  src/lib.rs           Point d'entrée et API publique
  src/camera.rs        Limites du capteur (plages d'exposition, sustain, fps, iso)
  src/pll.rs           Boucle à verrouillage de phase (PLL 2e ordre) et prédiction pipeline
  src/pwm.rs           Filtrage laplacien, autocorrélation robuste, simulation de profils
  src/pwm_controller.rs Contrôleur PWM rapide (< 500 ms) + suivi PLL
  src/controller.rs    Contrôleur stroboscopique (Search → Locked)
  src/restorer.rs      Post-traitement : gain temporel par trame, gain spatial par ligne
crates/ns-ffi/         Interface C (en-tête cbindgen) et passerelle JNI pour Android
crates/ns-post/        Outil no-strobe-post : CLI, serveur HTTP offload, pipeline ffmpeg
platform/android/      Application Android (Gradle, Camera2, Material You / M3)
platform/ios/          Application iOS (SwiftUI, Liquid Glass, AVFoundation)
tools/                 Scripts de test, banc d'essai et simulation de scintillement
```

---

### Compilation et Installation

Pour les utilisateurs finaux, téléchargez directement le fichier `app-debug.apk` depuis les [Releases](https://github.com/redoskance/no-strobe/releases) et suivez le [Guide d'installation](INSTALL.md).

#### Compilation native (Android NDK)
```sh
cargo install cargo-ndk
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/<version>
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -o platform/android/app/src/main/jniLibs \
    build -p ns-ffi --release
```

#### Assemblage de l'APK Android
```sh
cd platform/android
./gradlew assembleDebug
# Génère platform/android/app/build/outputs/apk/debug/app-debug.apk
```

---

### Post-traitement sur ordinateur (`no-strobe-post`)

Pour corriger une vidéo enregistrée avec une caméra conventionnelle :

```sh
cargo build --release -p ns-post
target/release/no-strobe-post fix concert.mp4 --line-rate 40000 -o concert-corrige.mp4
```

Vous pouvez également lancer le serveur local pour décharger le calcul depuis le smartphone :
```sh
export NO_STROBE_TOKEN=mon-secret
target/release/no-strobe-post serve --listen 0.0.0.0:8765
```

---

### Contribution

Consultez [`CONTRIBUTING.md`](CONTRIBUTING.md) pour les conventions de code, la signature GPG des commits et les directives de test.

---

<br/>

---

## 🇬🇧 English Version

Camera app (Android + iOS) that films concerts without LED flicker. It measures the flicker
of LED projectors and stage screens, then sets the sensor exposure to a whole number of
flicker cycles (`exposure = k/f`), so every frame and every sensor row integrates the same
amount of light: no banding, no brightness pumping.

**Scope: LED / PWM flicker, 100 Hz–5 kHz.** This is the supported mode.

| Mode | Light source | Frequency | Status |
|---|---|---|---|
| **PWM** | LED projectors, stage screens | 100 Hz–5 kHz | **main target**, real-time on Android and iOS |
| **Strobe** | deliberate slow flashes | 1–25 Hz | experimental, outside the project scope (see *Strobe modes*) |

A desktop post-processing tool fixes clips recorded without sync (see *Post-processing*).

For direct APK installation on your smartphone, see the step-by-step [Installation Guide](INSTALL.md).

---

### Why exposure sync works

- **Flicker** comes from frames integrating a different number of cycles. If the exposure
  window is an exact integer multiple of the period `T`, every frame integrates the same
  amount of light → no flicker, and the phase no longer matters.
- **Rolling-shutter banding** comes from rows being read out at different times. A row is
  exposed over `exposure + sustain`, where `sustain` is the sensor readout duration
  (`SENSOR_INFO_ROLLING_SHUTTER_SUSTAIN_DURATION` on Android, unpublished on iOS). If that
  window spans several cycles, top and bottom of the frame see different phases → horizontal
  bands. Exposure = `k/f` removes them.
- **Sub-second spatial convergence (< 500 ms)**: Rather than taking 60 seconds with empirical stepping, robust spatial autocorrelation (2nd-difference Laplacian pre-filter, MAD Winsorization, and mean-subtracted Pearson correlation) locks onto the PWM fundamental frequency $f$ in a single frame (33 ms).
- **Predictive pipeline filtering (Software PLL)**: In Camera2, an exposure command issued at frame $N$ is only applied by sensor hardware at frame $N+2$ or $N+3$ (pipeline depth). A 2nd-order software PLL tracks the flicker phase and compensates this latency with 2 to 3 frames of lead prediction.

---

### Strobe modes (Experimental)

> Experimental and outside the project scope (LED flicker only). Kept because the code exists
> and is tested, but it is not the target of the app. Exposure sync alone cannot flatten a
> slow strobe when `exposure = k·T` exceeds the frame period (e.g. 10 Hz at 30 fps).

| Mode | Exposure | Effect |
|---|---|---|
| `Integrate` | `k·T` | strobe energy uniform across frames, image stays bright |
| `Gap` | dark interval − sustain | strobe fully removed from the image |
| `Flash` | one flash window | every frame captures exactly one flash |

---

### PWM mode (Real-Time)

Frame-mean luma cannot resolve above `fps/2` — at 30 fps it aliases everything past 15 Hz. So
the PWM path reads the **per-row luminance profile** of a frame: the rolling shutter samples
light at the row readout rate (20–100 kHz), so a 100 Hz–5 kHz flicker shows up as a vertical
banding pattern.

1. `LineProfileExtractor` builds a per-row mean luma array from the Y plane.
2. `robust_period_rows` applies a discrete Laplacian filter $\Delta^2$, centers the profile, and computes Pearson cross-correlation to estimate $T$ in rows in a single frame.
3. The controller switches directly to `k/f`, compensates ISO to maintain overall frame exposure, and activates the PLL.
4. When the sensor's row readout rate is unknown (iOS), the controller hill-climbs on banding amplitude directly.

---

### Repository Layout

```
Cargo.toml             workspace: ns-core, ns-ffi, ns-post
crates/ns-core/        platform-free algorithm (no external deps)
  src/lib.rs           module list, public API
  src/camera.rs        sensor limits (exposure range, rolling-shutter sustain, fps, iso)
  src/pll.rs           software PLL (2nd order loop filter) & pipeline lead predictor
  src/pwm.rs           discrete Laplacian filter, robust autocorrelation, synthetic profiles
  src/pwm_controller.rs fast PWM controller (< 500 ms lock) & PLL integration
  src/controller.rs    strobe path: Search → Locked
  src/restorer.rs      post-processing: per-frame gain, per-row gain
crates/ns-ffi/         C API (cbindgen header) + JNI for Android
crates/ns-post/        no-strobe-post: fix.rs (streaming ffmpeg), serve.rs (HTTP server), main.rs (CLI)
platform/android/      Android app (Gradle, Camera2, Material 3 UI)
platform/ios/          iOS app (SwiftUI, Liquid Glass UI, AVFoundation)
tools/                 test scripts, flicker generator, test probes
```

---

### Building and Installation

Users can download the latest `app-debug.apk` directly from GitHub [Releases](https://github.com/redoskance/no-strobe/releases) and follow the [Installation Guide](INSTALL.md).

#### Build native JNI libraries
```sh
cargo install cargo-ndk
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/<version>
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -o platform/android/app/src/main/jniLibs \
    build -p ns-ffi --release
```

#### Assemble Android APK
```sh
cd platform/android
./gradlew assembleDebug
# Produces platform/android/app/build/outputs/apk/debug/app-debug.apk
```

---

### Desktop Post-Processing (`no-strobe-post`)

To deflicker clips recorded on standard cameras:

```sh
cargo build --release -p ns-post
target/release/no-strobe-post fix clip.mp4 --line-rate 40000 -o clip-fixed.mp4
```

To offload processing from your smartphone:
```sh
export NO_STROBE_TOKEN=some-secret
target/release/no-strobe-post serve --listen 0.0.0.0:8765
```

---

### Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for branch guidelines, GPG signing requirements, and testing guidelines.

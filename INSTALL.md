# Guide d'Installation / Installation Guide

[🇫🇷 Français](#-guide-dinstallation-français) | [🇬🇧 English](#-installation-guide-english)

---

## 🇫🇷 Guide d'Installation (Français)

Bienvenue sur **No Strobe** ! Ce guide vous explique pas à pas comment installer l'application sur votre téléphone, même si vous n'avez jamais installé d'application en dehors du Google Play Store ou de l'App Store.

---

### Méthode 1 : Téléchargement direct de l'APK (Recommandé pour Android)

C'est la méthode la plus simple et rapide. Vous n'avez pas besoin d'ordinateur, tout se fait directement depuis votre smartphone Android.

#### Étape 1 : Télécharger l'application
1. Ouvrez votre navigateur sur votre téléphone (Chrome, Firefox, etc.).
2. Rendez-vous sur la page des téléchargements officiels : [Releases GitHub](https://github.com/redoskance/no-strobe/releases).
3. Dans la section **Assets** de la dernière version (ou de la *Nightly Release*), appuyez sur le fichier nommé **`app-debug.apk`**.
4. Le téléchargement commence. Votre navigateur peut afficher un message d'avertissement du type : *"Ce type de fichier peut endommager votre appareil"*. C'est un avertissement de sécurité standard d'Android pour tout fichier téléchargé hors du Play Store. Appuyez sur **Télécharger quand même**.

#### Étape 2 : Autoriser l'installation de sources inconnues
Par défaut, Android protège votre appareil en bloquant les applications qui ne viennent pas du Play Store. Pour installer No Strobe :
1. Une fois le téléchargement terminé, appuyez sur la notification de téléchargement ou ouvrez votre application **Fichiers** / **Téléchargements** et touchez `app-debug.apk`.
2. Si une boîte de dialogue s'affiche disant : *"Pour votre sécurité, votre téléphone n'est pas autorisé à installer des applications inconnues depuis cette source"* :
   - Appuyez sur **Paramètres** (ou *Settings*).
   - Activez l'option **Autoriser cette source** (ou cochez votre navigateur / gestionnaire de fichiers).
   - Revenez en arrière avec le bouton retour de votre téléphone.

#### Étape 3 : Installer l'application
1. Appuyez sur **Installer**.
2. **Avertissement Google Play Protect** : Un écran bleu ou jaune peut apparaître indiquant *"Application bloquée par Play Protect"* ou *"Développeur inconnu"*.
   - Appuyez sur **Plus de détails** (ou *Détails*).
   - Appuyez sur **Installer quand même**.
   *(Cet avertissement est normal : No Strobe est un projet open-source indépendant qui n'a pas encore payé les certificats commerciaux de Google).*
3. Dès que l'installation est terminée, appuyez sur **Ouvrir**.

#### Étape 4 : Autorisations nécessaires
Au premier lancement, l'application vous demandera :
- **Appareil photo / Caméra** : Indispensable pour filmer et mesurer la fréquence du scintillement LED en temps réel. Appuyez sur **Lorsque vous utilisez l'application**.
- **Microphone** (optionnel) : Pour enregistrer le son de vos concerts en même temps que la vidéo.

---

### Méthode 2 : Installation via un ordinateur avec ADB

Si vous avez déjà configuré les outils Android sur votre ordinateur :
1. Branchez votre smartphone en USB et activez le **Débogage USB** dans les *Options pour les développeurs*.
2. Téléchargez `app-debug.apk` sur votre ordinateur.
3. Ouvrez un terminal et exécutez :
   ```sh
   adb install -r -g app-debug.apk
   ```
4. L'application est immédiatement installée et les permissions sont accordées.

---

### Méthode 3 : Compiler depuis les sources

Pour les développeurs qui souhaitent compiler l'application eux-mêmes :

#### Prérequis
- **Rust** (dernière version stable) : `rustup default stable`
- **Android NDK** (version 25 ou supérieure) : via Android Studio ou standalone
- **cargo-ndk** : `cargo install cargo-ndk`
- **JDK 17 ou 21**
- **Gradle**

#### Compilation
```sh
# 1. Cloner le dépôt
git clone https://github.com/redoskance/no-strobe.git
cd no-strobe

# 2. Vérifier et tester le moteur Rust
cargo test --workspace

# 3. Compiler les bibliothèques natives Android
export ANDROID_NDK_HOME=/chemin/vers/votre/ndk
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -o platform/android/app/src/main/jniLibs build -p ns-ffi --release

# 4. Assembler l'APK Android
cd platform/android
./gradlew assembleDebug
# L'APK compilé se trouve dans platform/android/app/build/outputs/apk/debug/app-debug.apk
```

---

### Installation sur iPhone (iOS)

Sur iOS, les applications hors App Store nécessitent une signature personnelle :
1. **Via AltStore / Sideloadly** : Téléchargez le package IPA depuis les Releases GitHub et installez-le avec votre identifiant Apple gratuit.
2. **Via Xcode** : Ouvrez `platform/ios/NoStrobe.xcodeproj` dans Xcode, sélectionnez votre iPhone et votre équipe personnelle, puis cliquez sur *Run*.

---

### En cas de problème / FAQ

- **L'application s'arrête ou indique que le capteur manuel n'est pas supporté** :
  No Strobe a besoin de l'accès manuel au temps d'exposition (`MANUAL_SENSOR`). Certains smartphones d'entrée de gamme ou constructeurs verrouillent cette fonctionnalité dans leur pilote Camera2.
- **Le scintillement persiste** :
  Assurez-vous d'orienter le capteur vers l'écran ou le projecteur LED pendant une seconde pour que l'algorithme synchronise l'exposition avec la fréquence PWM.

---

<br/>

---

## 🇬🇧 Installation Guide (English)

Welcome to **No Strobe**! This step-by-step guide walks you through installing the application on your mobile device, even if you have never installed an app outside the Google Play Store or Apple App Store before.

---

### Method 1: Direct APK Download (Recommended for Android)

This is the easiest and fastest way. No computer is needed; everything is done directly on your Android phone.

#### Step 1: Download the application
1. Open your mobile browser (Chrome, Firefox, etc.).
2. Go to the official releases page: [GitHub Releases](https://github.com/redoskance/no-strobe/releases).
3. In the **Assets** section of the latest release (or *Nightly Release*), tap on the file named **`app-debug.apk`**.
4. The download will start. Your browser may display a warning: *"File might be harmful"*. This is standard Android security for files downloaded outside the Play Store. Tap **Download anyway**.

#### Step 2: Allow installation from unknown sources
By default, Android blocks apps that do not come from the Play Store. To allow installation:
1. Once downloaded, tap the download notification or open your **Files** / **Downloads** app and tap `app-debug.apk`.
2. If a popup appears saying: *"For your security, your phone is not allowed to install unknown apps from this source"*:
   - Tap **Settings**.
   - Toggle **Allow from this source** to ON.
   - Press the Back button to return to the installer.

#### Step 3: Install the application
1. Tap **Install**.
2. **Google Play Protect Warning**: If a blue/yellow prompt appears stating *"Blocked by Play Protect"* or *"Unknown developer"*:
   - Tap **More details**.
   - Tap **Install anyway**.
   *(This warning is expected: No Strobe is an independent open-source project and does not carry commercial Google signatures).*
3. Once installation completes, tap **Open**.

#### Step 4: Grant required permissions
On first launch, grant:
- **Camera**: Essential for capturing video and measuring PWM flicker in real-time. Select **While using the app**.
- **Microphone** (optional): To capture concert audio alongside video.

---

### Method 2: Installation via ADB (Command Line)

If you have Android developer tools on your PC:
1. Connect your phone via USB with **USB Debugging** enabled.
2. Download `app-debug.apk` to your PC.
3. Open a terminal and run:
   ```sh
   adb install -r -g app-debug.apk
   ```

---

### Method 3: Building from Source

For developers who wish to compile from source:

#### Prerequisites
- **Rust** (stable): `rustup default stable`
- **Android NDK** (version 25+): via Android Studio or standalone
- **cargo-ndk**: `cargo install cargo-ndk`
- **JDK 17 or 21**
- **Gradle**

#### Build Steps
```sh
# 1. Clone repository
git clone https://github.com/redoskance/no-strobe.git
cd no-strobe

# 2. Test Rust core
cargo test --workspace

# 3. Build native shared libraries
export ANDROID_NDK_HOME=/path/to/your/ndk
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -o platform/android/app/src/main/jniLibs build -p ns-ffi --release

# 4. Assemble Android APK
cd platform/android
./gradlew assembleDebug
# Generated APK: platform/android/app/build/outputs/apk/debug/app-debug.apk
```

---

### iPhone (iOS) Installation

On iOS, installing applications outside the App Store requires personal signing:
1. **Via AltStore / Sideloadly**: Download the `.ipa` package from GitHub Releases and install with your personal Apple ID.
2. **Via Xcode**: Open `platform/ios/NoStrobe.xcodeproj` in Xcode, select your connected iPhone and personal team, and click *Run*.

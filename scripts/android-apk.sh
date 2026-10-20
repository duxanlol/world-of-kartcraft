#!/usr/bin/env bash
# The Android package: `libbenilla.so` (crates/benilla-android) in an APK whose NativeActivity
# loads it, no Java code, signed with the local key, and with `--install` put on the phone
# adb sees (`ANDROID_SERIAL` picks one). The player build (no `dev`), release profile, arm64
# only, the server built in. `target/android/kartcraft.apk` is the file to give out: a player
# copies their own game files in (the app says where; nothing of the install is in it).
#
# Needs the NDK (ANDROID_NDK_HOME, r27), the SDK (ANDROID_HOME: build-tools;34.0.0,
# platforms;android-34, platform-tools), a JDK for apksigner and keytool, and the Rust target
# `aarch64-linux-android`.
#
# `--data` copies the install (`WoW/Data`) and the kart data (`kart-data`) into the app's own
# folder on the phone over adb (crates/benilla-app/src/android.rs), which the app must have made
# by running once.
#
#   scripts/android-apk.sh [--install] [--data] [--run]
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

: "${ANDROID_HOME:=/c/Android/sdk}"
: "${ANDROID_NDK_HOME:=/c/Android/android-ndk-r27d}"
export ANDROID_NDK_HOME ANDROID_NDK_ROOT="$ANDROID_NDK_HOME"
PACKAGE=com.worldofkartcraft.game
API=30
BUILD_TOOLS="$ANDROID_HOME/build-tools/34.0.0"
PLATFORM="$ANDROID_HOME/platforms/android-34/android.jar"
ADB="$ANDROID_HOME/platform-tools/adb"

case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN*) host=windows-x86_64 cmd=.cmd bat=.bat ;;
    Darwin) host=darwin-x86_64 cmd= bat= ;;
    *) host=linux-x86_64 cmd= bat= ;;
esac
llvm="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$host/bin"
# cargo hands these to cmd.exe on Windows, which takes no `/c/...` path.
command -v cygpath >/dev/null && llvm="$(cygpath -m "$llvm")"
export CC_aarch64_linux_android="$llvm/aarch64-linux-android$API-clang$cmd"
export CXX_aarch64_linux_android="$llvm/aarch64-linux-android$API-clang++$cmd"
export AR_aarch64_linux_android="$llvm/llvm-ar"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CC_aarch64_linux_android"

# The channel's presets, as publish.ps1 reads them: `.setup/publish.cfg`, and for
# ANDROID_CHANNEL=staging `.setup/publish-staging.cfg` over it (the staging server and its
# update folder). Built in: the server it plays on, and where it updates itself from, the update
# folder's `android/` (`dist/android-upload.sh`).
channel="${ANDROID_CHANNEL:-}"
cfgs=(.setup/publish.cfg)
[ "$channel" = staging ] && cfgs=(.setup/publish-staging.cfg .setup/publish.cfg)
[ -z "$channel" ] || [ -f "${cfgs[0]}" ] || { echo "${cfgs[0]} is missing: it names the $channel server"; exit 1; }
preset() { cat "${cfgs[@]}" 2>/dev/null | sed -n "s/^$1=//p" | head -1 | tr -d '\r'; }
KARTCRAFT_SERVER="$(preset SERVER)"
updates="$(preset UPDATE_URL)"
KARTCRAFT_UPDATE_URL="${updates:+${updates%/}/android}"
# Staging sleeps when nobody plays (benilla-config/staging/idle): its build wakes it first.
KARTCRAFT_WAKE_URL=""
[ "$channel" = staging ] && KARTCRAFT_WAKE_URL="https://worldofkartcraft.com/staging/wake"
# Its own version code, which an update must beat: the commit count, growing with every commit.
code="$(git rev-list --count HEAD)"
KARTCRAFT_VERSION_CODE="$code"
export KARTCRAFT_SERVER KARTCRAFT_UPDATE_URL KARTCRAFT_WAKE_URL KARTCRAFT_VERSION_CODE
echo "channel ${channel:-production}: server ${KARTCRAFT_SERVER:-none}, updates ${KARTCRAFT_UPDATE_URL:-none}"
cargo build -p benilla-android --target aarch64-linux-android --release --no-default-features

out="$root/target/android"
stage="$out/stage"
rm -rf "$stage"
mkdir -p "$stage/lib/arm64-v8a" "$out/res/mipmap"
cp target/aarch64-linux-android/release/libbenilla.so "$stage/lib/arm64-v8a/"
"$llvm/llvm-strip$( [ "$host" = windows-x86_64 ] && echo .exe)" --strip-debug "$stage/lib/arm64-v8a/libbenilla.so"

version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
version="$version-$(git rev-parse --short HEAD)"
cat >"$out/AndroidManifest.xml" <<EOF
<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android"
    package="$PACKAGE" android:versionCode="$code" android:versionName="$version">
    <uses-sdk android:minSdkVersion="$API" android:targetSdkVersion="34" />
    <uses-permission android:name="android.permission.INTERNET" />
    <!-- "All files access": the player's own game files, copied into Internal storage. -->
    <uses-permission android:name="android.permission.MANAGE_EXTERNAL_STORAGE" />
    <!-- The app's own update, handed to the system installer (`Updater.java`). -->
    <uses-permission android:name="android.permission.REQUEST_INSTALL_PACKAGES" />
    <uses-feature android:name="android.hardware.vulkan.level" android:required="true" />
    <uses-feature android:name="android.hardware.touchscreen" android:required="false" />
    <application android:label="World of Kartcraft" android:icon="@mipmap/icon"
        android:hasCode="true" android:extractNativeLibs="true" android:appCategory="game"
        android:isGame="true">
        <!-- A keyboard, mouse or pad plugged in changes the configuration: the client keeps
             running through it instead of being restarted. -->
        <activity android:name="android.app.NativeActivity" android:exported="true"
            android:launchMode="singleTask" android:screenOrientation="sensorLandscape"
            android:theme="@android:style/Theme.NoTitleBar.Fullscreen"
            android:configChanges="orientation|screenSize|screenLayout|smallestScreenSize|keyboard|keyboardHidden|navigation|uiMode|density|touchscreen">
            <meta-data android:name="android.app.lib_name" android:value="benilla" />
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
            </intent-filter>
        </activity>
        <!-- The installer's answer to an update: shows the player its confirmation. -->
        <receiver android:name="com.worldofkartcraft.game.InstallReceiver" android:exported="false" />
    </application>
</manifest>
EOF
cp dist/icon.png "$out/res/mipmap/icon.png"

# The APK's only Java, the updater (crates/benilla-android/java), compiled against the platform
# and turned into `classes.dex` by the SDK's d8.
rm -rf "$out/classes"
mkdir -p "$out/classes"
javac --release 11 -Xlint:all -Werror -cp "$PLATFORM" -d "$out/classes" \
    crates/benilla-android/java/com/worldofkartcraft/game/*.java
"$BUILD_TOOLS/d8$bat" --release --min-api "$API" --lib "$PLATFORM" --output "$stage" \
    $(find "$out/classes" -name '*.class')

(cd "$out" && "$BUILD_TOOLS/aapt2" compile -o compiled.zip --dir res)
"$BUILD_TOOLS/aapt2" link -o "$out/unsigned.apk" -I "$PLATFORM" \
    --manifest "$out/AndroidManifest.xml" "$out/compiled.zip"
(cd "$stage" && "$BUILD_TOOLS/aapt" add -0 so "$out/unsigned.apk" classes.dex lib/arm64-v8a/libbenilla.so >/dev/null)
"$BUILD_TOOLS/zipalign" -f -p 4 "$out/unsigned.apk" "$out/aligned.apk"

# The signing key lives in the state folder: machine-local, never committed. Every APK given out
# must be signed with this same key, or a phone refuses it as an update; keep a copy of it.
# ANDROID_KEYSTORE names it elsewhere (the release workflow's, from a secret), and
# ANDROID_KEYSTORE_PASS its password; only the state folder's is made when missing.
key="${ANDROID_KEYSTORE:-$root/benilla-config/android/debug.keystore}"
export ANDROID_KEYSTORE_PASS="${ANDROID_KEYSTORE_PASS:-android}"
if [ ! -f "$key" ]; then
    [ -z "${ANDROID_KEYSTORE:-}" ] || { echo "no keystore at $ANDROID_KEYSTORE"; exit 1; }
    mkdir -p "$(dirname "$key")"
    keytool -genkeypair -keystore "$key" -storepass android -keypass android -alias debug \
        -keyalg RSA -keysize 2048 -validity 10000 -dname "CN=World of Kartcraft debug" >/dev/null
fi
"$BUILD_TOOLS/apksigner$bat" sign --ks "$key" --ks-pass env:ANDROID_KEYSTORE_PASS \
    --key-pass env:ANDROID_KEYSTORE_PASS --out "$out/kartcraft.apk" "$out/aligned.apk"
rm -f "$out/unsigned.apk" "$out/aligned.apk"
# What `dist/android-upload.sh` puts beside the APK on the update host, for the app to compare.
sha="$(sha256sum "$out/kartcraft.apk" | cut -d' ' -f1)"
printf '{\n  "versionCode": %s,\n  "versionName": "%s",\n  "channel": "%s",\n  "apk": "WorldOfKartcraft.apk",\n  "sha256": "%s"\n}\n' \
    "$code" "$version" "${channel:-production}" "$sha" >"$out/version.json"
echo "built $out/kartcraft.apk ($(du -h "$out/kartcraft.apk" | cut -f1)), build $code ($version, ${channel:-production})"

for arg in "$@"; do
    case "$arg" in
        --install) "$ADB" install -r "$out/kartcraft.apk" ;;
        --data)
            # The app made its folder; a pushed file is the shell's, so it is opened to the app.
            dir=/sdcard/Android/data/$PACKAGE/files
            MSYS_NO_PATHCONV=1 "$ADB" shell "test -d $dir" || {
                echo "run the app once first, so Android makes its folder"
                exit 1
            }
            MSYS_NO_PATHCONV=1 "$ADB" push WoW/Data "$dir/WoW/"
            MSYS_NO_PATHCONV=1 "$ADB" push kart-data "$dir/"
            MSYS_NO_PATHCONV=1 "$ADB" shell "chmod -R a+rwX $dir/WoW $dir/kart-data"
            ;;
        --run) "$ADB" shell am start -n "$PACKAGE/android.app.NativeActivity" ;;
    esac
done

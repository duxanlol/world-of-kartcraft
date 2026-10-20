#!/usr/bin/env bash
# The Android package: `libbenilla.so` (crates/benilla-android) in an APK whose NativeActivity
# loads it, no Java code, signed with a local debug key, and with `--install` put on the phone
# adb sees. The player build (no `dev`), release profile, arm64 only.
#
# Needs the NDK (ANDROID_NDK_HOME, r27), the SDK (ANDROID_HOME: build-tools;34.0.0,
# platforms;android-34, platform-tools), a JDK for apksigner and keytool, and the Rust target
# `aarch64-linux-android`.
#
# `--data` copies the install (`WoW/Data`), the kart data (`kart-data`) and `WOW_HOST` (as
# `env.txt`) into the app's folder on the phone (crates/benilla-app/src/android.rs), which the
# app must have made by running once.
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

cargo build -p benilla-android --target aarch64-linux-android --release --no-default-features

out="$root/target/android"
stage="$out/stage"
rm -rf "$stage"
mkdir -p "$stage/lib/arm64-v8a" "$out/res/mipmap"
cp target/aarch64-linux-android/release/libbenilla.so "$stage/lib/arm64-v8a/"
"$llvm/llvm-strip$( [ "$host" = windows-x86_64 ] && echo .exe)" --strip-debug "$stage/lib/arm64-v8a/libbenilla.so"

version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
# A code that grows with every build, so the phone takes a reinstall as an update.
code="$(git rev-list --count HEAD)"
cat >"$out/AndroidManifest.xml" <<EOF
<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android"
    package="$PACKAGE" android:versionCode="$code" android:versionName="$version">
    <uses-sdk android:minSdkVersion="$API" android:targetSdkVersion="34" />
    <uses-permission android:name="android.permission.INTERNET" />
    <uses-feature android:name="android.hardware.vulkan.level" android:required="true" />
    <uses-feature android:name="android.hardware.touchscreen" android:required="false" />
    <application android:label="World of Kartcraft" android:icon="@mipmap/icon"
        android:hasCode="false" android:extractNativeLibs="true" android:appCategory="game"
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
    </application>
</manifest>
EOF
cp dist/icon.png "$out/res/mipmap/icon.png"


(cd "$out" && "$BUILD_TOOLS/aapt2" compile -o compiled.zip --dir res)
"$BUILD_TOOLS/aapt2" link -o "$out/unsigned.apk" -I "$PLATFORM" \
    --manifest "$out/AndroidManifest.xml" "$out/compiled.zip"
(cd "$stage" && "$BUILD_TOOLS/aapt" add -0 so "$out/unsigned.apk" lib/arm64-v8a/libbenilla.so >/dev/null)
"$BUILD_TOOLS/zipalign" -f -p 4 "$out/unsigned.apk" "$out/aligned.apk"

# The debug key lives in the state folder: machine-local, never committed.
key="$root/benilla-config/android/debug.keystore"
if [ ! -f "$key" ]; then
    mkdir -p "$(dirname "$key")"
    keytool -genkeypair -keystore "$key" -storepass android -keypass android -alias debug \
        -keyalg RSA -keysize 2048 -validity 10000 -dname "CN=World of Kartcraft debug" >/dev/null
fi
"$BUILD_TOOLS/apksigner$bat" sign --ks "$key" --ks-pass pass:android --key-pass pass:android \
    --out "$out/kartcraft.apk" "$out/aligned.apk"
rm -f "$out/unsigned.apk" "$out/aligned.apk"
echo "built $out/kartcraft.apk ($(du -h "$out/kartcraft.apk" | cut -f1))"

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
            host="$(sed -n "s/^SERVER=//p" .setup/publish.cfg 2>/dev/null | tr -d '\r')"
            [ -n "$host" ] && MSYS_NO_PATHCONV=1 "$ADB" shell "echo WOW_HOST=$host > $dir/env.txt"
            MSYS_NO_PATHCONV=1 "$ADB" shell "chmod -R a+rwX $dir/WoW $dir/kart-data $dir/env.txt"
            ;;
        --run) "$ADB" shell am start -n "$PACKAGE/android.app.NativeActivity" ;;
    esac
done

#!/usr/bin/env bash
# Native half of the Android TV build (Chromecast with Google TV, 32-bit ARM):
# SDL2 from source, libluma.so from the Rust launcher, and Luma's generated
# assets. Run after `pnpm assets`, with ANDROID_NDK_ROOT set; Gradle then
# packages android/app (see android/README.md and .github/workflows/android.yml).
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
APP="$ROOT/android/app/src/main"
WORK="$ROOT/.build/android"
API=26
ABI=armeabi-v7a
TARGET=armv7-linux-androideabi
SDL_VERSION=2.32.10
SDL_SHA256=5f5993c530f084535c65a6879e9b26ad441169b3e25d789d83287040a9ca5165

: "${ANDROID_NDK_ROOT:?Set ANDROID_NDK_ROOT to an Android NDK (r27 or newer)}"
TOOLS="$ANDROID_NDK_ROOT/toolchains/llvm/prebuilt/linux-x86_64/bin"
CLANG="$TOOLS/armv7a-linux-androideabi$API-clang"
[ -x "$CLANG" ] || { echo "Missing NDK compiler: $CLANG" >&2; exit 1; }
[ -f "$ROOT/.build/assets/version.txt" ] || { echo "Run pnpm assets first." >&2; exit 1; }
mkdir -p "$WORK"

# SDL2: pinned release, checked before use.
SDL_ARCHIVE="$WORK/SDL2-$SDL_VERSION.tar.gz"
if [ ! -f "$SDL_ARCHIVE" ]; then
  curl -fsSL -o "$SDL_ARCHIVE.part" \
    "https://github.com/libsdl-org/SDL/releases/download/release-$SDL_VERSION/SDL2-$SDL_VERSION.tar.gz"
  mv "$SDL_ARCHIVE.part" "$SDL_ARCHIVE"
fi
echo "$SDL_SHA256  $SDL_ARCHIVE" | sha256sum -c -
SDL_SOURCE="$WORK/SDL2-$SDL_VERSION"
rm -rf "$SDL_SOURCE"
tar -xzf "$SDL_ARCHIVE" -C "$WORK"
SDL_BUILD="$WORK/sdl-$ABI"
cmake -S "$SDL_SOURCE" -B "$SDL_BUILD" \
  -DCMAKE_TOOLCHAIN_FILE="$ANDROID_NDK_ROOT/build/cmake/android.toolchain.cmake" \
  -DANDROID_ABI=$ABI -DANDROID_PLATFORM=android-$API -DANDROID_STL=c++_static \
  -DCMAKE_BUILD_TYPE=Release -DSDL_SHARED=ON -DSDL_STATIC=OFF -DSDL_TEST=OFF
cmake --build "$SDL_BUILD" --parallel
SDL_LIB="$SDL_BUILD/libSDL2.so"
"$TOOLS/llvm-readelf" -d "$SDL_LIB" | grep -q 'SONAME.*\[libSDL2\.so\]' \
  || { echo "libSDL2.so has an unexpected SONAME" >&2; exit 1; }

# SDLActivity and its helpers, from the same SDL release.
rm -rf "$APP/java/org/libsdl"
mkdir -p "$APP/java/org/libsdl"
cp -R "$SDL_SOURCE/android-project/app/src/main/java/org/libsdl/app" "$APP/java/org/libsdl/"

# libluma.so: the launcher, linked against that SDL2.
export CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_LINKER="$CLANG"
export CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_RUSTFLAGS="-L native=$SDL_BUILD"
export CC_armv7_linux_androideabi="$CLANG"
export CXX_armv7_linux_androideabi="$CLANG++"
export AR_armv7_linux_androideabi="$TOOLS/llvm-ar"
export ANDROID_NDK_HOME="$ANDROID_NDK_ROOT"
cargo build --locked --release --manifest-path "$ROOT/Cargo.toml" --package luma --lib --target $TARGET
LUMA_LIB="$ROOT/target/$TARGET/release/libluma.so"
"$TOOLS/llvm-nm" -D --defined-only "$LUMA_LIB" | grep -q ' T SDL_main$' \
  || { echo "libluma.so does not export SDL_main" >&2; exit 1; }
NEEDED=$("$TOOLS/llvm-readelf" -d "$LUMA_LIB" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p' | sort | tr '\n' ' ')
echo "libluma.so needs: $NEEDED"
for library in $NEEDED; do
  case "$library" in
    libSDL2.so | libc.so | libm.so | libdl.so | liblog.so | libandroid.so | libGLESv2.so | libEGL.so | libOpenSLES.so) ;;
    *) echo "libluma.so needs $library, which the APK does not ship" >&2; exit 1 ;;
  esac
done
rm -rf "$APP/jniLibs"
mkdir -p "$APP/jniLibs/$ABI"
cp "$SDL_LIB" "$LUMA_LIB" "$APP/jniLibs/$ABI/"

# Assets: everything the TV package ships except video wallpapers and the
# webOS splash screens; LumaActivity copies them out using files.txt.
OUT="$APP/assets/luma"
rm -rf "$APP/assets"
mkdir -p "$OUT"
(
  cd "$ROOT/.build/assets"
  find . -type f \
    ! -path './video-stills/*' ! -path './video-thumbnails/*' \
    ! -path './splash/*' ! -path './app-icons/*' \
    ! -name '*@2x.jpg' ! -name '.DS_Store' \
    ! \( -name '*.png' \( -path './logos/*' -o -path './ui-icons/*' \) \) \
    | sed 's|^\./||' | LC_ALL=C sort > "$OUT/files.txt"
  while IFS= read -r file; do
    mkdir -p "$OUT/$(dirname "$file")"
    cp "$file" "$OUT/$file"
  done < "$OUT/files.txt"
)
echo "Assets: $(wc -l < "$OUT/files.txt") files, $(du -sh "$OUT" | cut -f1)"

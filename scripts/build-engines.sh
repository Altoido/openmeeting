#!/bin/bash
# Builds whisper-cli and llama-server from pinned releases and places them where Tauri expects
# sidecars: src-tauri/binaries/<name>-<target-triple>[.exe]. Works on macOS, Linux and Windows (Git Bash).
set -euo pipefail
WHISPER_TAG="${WHISPER_TAG:-v1.9.5}"; LLAMA_TAG="${LLAMA_TAG:-b12441}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"; WORK="$ROOT/engines-build"; OUT="$ROOT/src-tauri/binaries"
TRIPLE="${TARGET_TRIPLE:-$(rustc -vV | sed -n 's/host: //p')}"
mkdir -p "$WORK" "$OUT"
[ -d "$WORK/whisper.cpp" ] || git clone --depth 1 --branch "$WHISPER_TAG" https://github.com/ggml-org/whisper.cpp "$WORK/whisper.cpp"
[ -d "$WORK/llama.cpp" ] || git clone --depth 1 --branch "$LLAMA_TAG" https://github.com/ggml-org/llama.cpp "$WORK/llama.cpp"
COMMON=(-DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF)
EXE=""
case "$TRIPLE" in
  *apple-darwin*) COMMON+=(-DCMAKE_OSX_DEPLOYMENT_TARGET=13.0 -DGGML_METAL=ON -DGGML_METAL_EMBED_LIBRARY=ON -DGGML_NATIVE=OFF)
                  [[ "$TRIPLE" == x86_64* ]] && COMMON+=(-DCMAKE_OSX_ARCHITECTURES=x86_64 -DGGML_METAL=OFF) || COMMON+=(-DCMAKE_OSX_ARCHITECTURES=arm64) ;;
  *windows*) EXE=".exe"
             COMMON+=(-DGGML_NATIVE=OFF -DGGML_AVX=ON -DGGML_AVX2=ON -DGGML_FMA=ON -DGGML_F16C=ON
                      -DCMAKE_POLICY_DEFAULT_CMP0091=NEW -DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded) ;;
esac
cmake -S "$WORK/whisper.cpp" -B "$WORK/wb" "${COMMON[@]}" -DWHISPER_BUILD_TESTS=OFF -DWHISPER_BUILD_SERVER=OFF -DWHISPER_SDL2=OFF -DWHISPER_CURL=OFF
cmake --build "$WORK/wb" --config Release --target whisper-cli -j 4
cmake -S "$WORK/llama.cpp" -B "$WORK/lb" "${COMMON[@]}" -DLLAMA_BUILD_TESTS=OFF -DLLAMA_BUILD_EXAMPLES=OFF -DLLAMA_BUILD_UI=OFF \
      -DLLAMA_USE_PREBUILT_UI=OFF -DLLAMA_OPENSSL=OFF -DLLAMA_CONNECT=OFF
cmake --build "$WORK/lb" --config Release --target llama-server -j 4
find_bin() { find "$1" -type f \( -name "$2" -o -name "$2.exe" \) -path "*bin*" | head -1; }
cp "$(find_bin "$WORK/wb" whisper-cli)" "$OUT/whisper-cli-$TRIPLE$EXE"
cp "$(find_bin "$WORK/lb" llama-server)" "$OUT/llama-server-$TRIPLE$EXE"
mkdir -p "$ROOT/src-tauri/resources/licenses"
cp "$WORK/whisper.cpp/LICENSE" "$ROOT/src-tauri/resources/licenses/whisper.cpp-LICENSE.txt"
cp "$WORK/llama.cpp/LICENSE" "$ROOT/src-tauri/resources/licenses/llama.cpp-LICENSE.txt"
ls -la "$OUT"

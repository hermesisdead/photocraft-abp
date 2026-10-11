#!/bin/sh
# Builds cassette_tape.wasm with clang (any recent clang with the WebAssembly target + wasm-ld).
set -eu
cd "$(dirname "$0")"
clang --target=wasm32 -O2 -nostdlib -ffreestanding -fno-builtin -fno-math-errno \
  -mno-simd128 -mno-bulk-memory -mno-reference-types -mno-multivalue -mno-nontrapping-fptoint \
  -Wall -Wextra -Wl,--no-entry -Wl,-z,stack-size=65536 -Wl,--initial-memory=262144 -Wl,--max-memory=536870912 -Wl,--strip-all \
  -o cassette_tape.wasm cassette_tape.c
echo "built cassette_tape.wasm ($(wc -c < cassette_tape.wasm) bytes)"

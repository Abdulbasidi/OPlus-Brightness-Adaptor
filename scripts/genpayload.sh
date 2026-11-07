#!/bin/bash
# Regenerate the composer-side payload blob and its Rust embedding.
#
# asm/hwc_payload.s is the source of truth. The script assembles it for
# aarch64, extracts the raw .text bytes and writes both asm/hwc_payload.bin
# and src/hwc_payload.rs.
set -euo pipefail

cd "$(dirname "$0")/.."

NDK=${NDK:-/home/ryanistr/android-ndk-r27c/toolchains/llvm/prebuilt/linux-x86_64/bin}
CLANG="$NDK/clang"
OBJCOPY="$NDK/llvm-objcopy"
OBJDUMP="$NDK/llvm-objdump"
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

"$CLANG" --target=aarch64-linux-android31 -c asm/hwc_payload.s -o "$OUT/payload.o"
"$OBJCOPY" -O binary --only-section=.text "$OUT/payload.o" "$OUT/payload.bin"
cp "$OUT/payload.bin" asm/hwc_payload.bin

size=$(stat -c %s asm/hwc_payload.bin)
if [ "$size" -ne 1032 ]; then
    echo "unexpected payload size: $size" >&2
    exit 1
fi

{
    echo "// Generated from asm/hwc_payload.s by scripts/genpayload.sh - do not edit."
    echo "//"
    echo "// Code page layout:"
    echo "//   0x000  stub A - displayGetBrightnessSupport replacement"
    echo "//   0x200  stub B - displaySetBrightness replacement"
    echo "//   0x400  dptr   - state page address, patched at install time"
    echo ""
    echo "pub const PAYLOAD: &[u8] = &["
    python3 - "$OUT/payload.bin" <<'PY'
import sys
data = open(sys.argv[1], 'rb').read()
for i in range(0, len(data), 12):
    chunk = data[i:i + 12]
    print("    " + " ".join(f"0x{b:02x}," for b in chunk))
PY
    echo "];"
    echo ""
    echo "/// Offset of the displayGetBrightnessSupport replacement."
    echo "pub const STUB_SUPPORT: usize = 0x000;"
    echo "/// Offset of the displaySetBrightness replacement."
    echo "pub const STUB_SET: usize = 0x200;"
    echo "/// Offset of the pointer to the state page."
    echo "pub const DPTR: usize = 0x400;"
    echo ""
    echo "/// Offset of the state struct in the state page."
    echo "pub const STATE: usize = 0x000;"
    echo "/// Size of the state struct: led_fd, min_bri, max_bri, last_val."
    echo "pub const STATE_LEN: usize = 16;"
    echo "/// Offset of the float constants in the state page."
    echo "pub const FLOATS: usize = 0x010;"
    echo "/// Offset of the last write() result in the state page."
    echo "pub const WRESULT: usize = 0x018;"
    echo "/// Offset of the backlight path buffer in the state page."
    echo "pub const PATHBUF: usize = 0x100;"
} > src/hwc_payload.rs

echo "payload: $size bytes"
"$OBJDUMP" -d "$OUT/payload.o" | sed -n '1,12p'

#!/bin/bash
# Regenerate the composer-side payload blob and its Rust embedding.
#
# asm/hwc_payload.s is the source of truth. The script assembles it for
# aarch64, extracts the raw .text bytes and writes both asm/hwc_payload.bin
# and src/hwc_payload.rs.
set -euo pipefail

cd "$(dirname "$0")/.."

TOOLCHAIN=/home/ryanistr/android-ndk-r27c/toolchains/llvm/prebuilt/linux-x86_64/bin
CLANG="$TOOLCHAIN/clang"
OBJCOPY="$TOOLCHAIN/llvm-objcopy"
OBJDUMP="$TOOLCHAIN/llvm-objdump"
OUT="target/scratch/genpayload"
mkdir -p "$OUT"
trap 'rm -rf "$OUT"' EXIT

# --- 1. hwc_payload ---
"$CLANG" --target=aarch64-linux-android31 -c asm/hwc_payload.s -o "$OUT/hwc_payload.o"
"$OBJCOPY" -O binary --only-section=.text "$OUT/hwc_payload.o" "$OUT/hwc_payload.bin"
cp "$OUT/hwc_payload.bin" asm/hwc_payload.bin

size=$(stat -c %s asm/hwc_payload.bin)
if [ "$size" -ne 1032 ]; then
    echo "unexpected hwc payload size: $size" >&2
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
    python3 - "$OUT/hwc_payload.bin" <<'PY'
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
    echo "/// Offset of the dynamic perceptual curve LUT (256 f32 entries) in the state page."
    echo "pub const LUT: usize = 0x200;"
    echo "/// Size of the LUT in bytes (256 * 4)."
    echo "pub const LUT_LEN: usize = 256 * 4;"
} > src/hwc_payload.rs

echo "hwc payload: $size bytes"

# --- 2. gui_payload ---
"$CLANG" --target=aarch64-linux-android31 -c asm/gui_payload.s -o "$OUT/gui_payload.o"
"$OBJCOPY" -O binary --only-section=.text "$OUT/gui_payload.o" "$OUT/gui_payload.bin"
cp "$OUT/gui_payload.bin" asm/gui_payload.bin

gui_size=$(stat -c %s asm/gui_payload.bin)
if [ "$gui_size" -ne 1032 ]; then
    echo "unexpected gui payload size: $gui_size" >&2
    exit 1
fi

{
    echo "// Generated from asm/gui_payload.s by scripts/genpayload.sh - do not edit."
    echo "//"
    echo "// Code page layout:"
    echo "//   0x000  stub A - getDisplayBrightnessSupport replacement"
    echo "//   0x200  stub B - setDisplayBrightness replacement"
    echo "//   0x400  dptr   - state page address, patched at install time"
    echo ""
    echo "pub const PAYLOAD: &[u8] = &["
    python3 - "$OUT/gui_payload.bin" <<'PY'
import sys
data = open(sys.argv[1], 'rb').read()
for i in range(0, len(data), 12):
    chunk = data[i:i + 12]
    print("    " + " ".join(f"0x{b:02x}," for b in chunk))
PY
    echo "];"
    echo ""
    echo "/// Offset of the getDisplayBrightnessSupport replacement."
    echo "pub const STUB_SUPPORT: usize = 0x000;"
    echo "/// Offset of the setDisplayBrightness replacement."
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
    echo "/// Offset of the stub A execution counter in the state page."
    echo "pub const COUNTA: usize = 0x01c;"
    echo "/// Offset of the stub B execution counter in the state page."
    echo "pub const COUNTB: usize = 0x020;"
    echo "/// Offset of the ABI mode (0 = struct, 1 = direct s0, 2 = direct s1)."
    echo "pub const ABI_MODE: usize = 0x024;"
    echo "/// Offset of the support ABI mode (0 = bool in w0, 1 = bool* in x1)."
    echo "pub const SUPPORT_ABI: usize = 0x028;"
    echo "/// Offset of the brightness field inside the struct (default 0x10)."
    echo "pub const STRUCT_OFFSET: usize = 0x02c;"
    echo "/// Offset of the backlight path buffer in the state page."
    echo "pub const PATHBUF: usize = 0x100;"
    echo "/// Offset of the dynamic perceptual curve LUT (256 f32 entries) in the state page."
    echo "pub const LUT: usize = 0x200;"
    echo "/// Size of the LUT in bytes (256 * 4)."
    echo "pub const LUT_LEN: usize = 256 * 4;"
} > src/gui_payload.rs

echo "gui payload: $gui_size bytes"

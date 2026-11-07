// Payload for the framework-side (libgui / SurfaceComposerClient) brightness hook.
//
// The code and the mutable state live in two different mappings:
//
//   * the code page is a private file backed executable mapping of a library
//     system_server already loads (libgui.so).
//   * the state page is a plain anonymous read/write mapping.
//
// The state page address is patched into the pointer slot at 0x400 when the
// hook is installed, so the stub stays position independent.
//
// Code page layout:
//   0x000  stub A - SurfaceComposerClient::getDisplayBrightnessSupport replacement
//   0x200  stub B - SurfaceComposerClient::setDisplayBrightness replacement
//   0x400  dptr   - state page address, patched at install time
//
// State page layout:
//   0x000  state   - { int32 led_fd, min_bri, max_bri, last_val }
//   0x010  floats  - { float 1.0f, float 0.0f }
//   0x018  wresult - last write() result, diagnostics only
//   0x01c  countA  - stub A execution counter
//   0x020  countB  - stub B execution counter
//   0x100  path    - backlight sysfs path, NUL terminated
//
// AAPCS64 for the two SurfaceComposerClient entry points:
//   bool getDisplayBrightnessSupport(const sp<IBinder>& displayToken)
//       x0 = displayToken, returns bool in w0 (1 = true)
//   status_t setDisplayBrightness(const sp<IBinder>& displayToken, const gui::DisplayBrightness& brightness)
//       x0 = displayToken, x1 = &brightness, returns status_t in w0 (0 = NO_ERROR)

    .text
    .balign 4096
page:
    // ---- stub A: SurfaceComposerClient::getDisplayBrightnessSupport ----
    adr     x5, dptr
    ldr     x4, [x5]                // state page
    ldr     w6, [x4, #0x01c]        // call counter
    add     w6, w6, #1
    str     w6, [x4, #0x01c]

    ldr     w7, [x4, #0x028]        // support_abi (0 = bool in w0, 1 = bool* in x1)
    cbz     w7, .Lret_bool

    // support_abi == 1: BpSurfaceComposer (status_t in w0, bool* in x1)
    cbz     x1, .Lret_ok
    mov     w2, #1
    strb    w2, [x1]                // *outSupport = true
.Lret_ok:
    mov     w0, #0                  // status_t NO_ERROR
    ret

.Lret_bool:
    mov     w0, #1                  // true (supported)
    ret

    .balign 0x200
stubB:
    // ---- stub B: SurfaceComposerClient::setDisplayBrightness ----
    adr     x5, dptr
    ldr     x4, [x5]                // state page
    ldr     w7, [x4, #0x024]        // abi_mode: 0 = struct in x1, 1 = float in s0, 2 = float in s1
    cmp     w7, #1
    b.eq    .LhaveDirectS0
    cmp     w7, #2
    b.eq    .LhaveDirectS1

    // abi_mode == 0: struct in x1
    cbz     x1, .LbadB              // null pointer check

    // Check displayBrightness at struct offset
    ldr     w8, [x4, #0x02c]        // struct_offset (default 0x10)
    ldr     s0, [x1, w8, uxtw]
    fcmp    s0, s0                  // NaN check
    b.vs    .LfallbackSdr
    fmov    s2, #0.0
    fcmp    s0, s2
    b.lt    .LfallbackSdr           // if negative, fallback to sdrBrightness
    b       .LhaveCandidate

.LfallbackSdr:
    ldr     s0, [x1, #0x08]         // sdrBrightness
    fcmp    s0, s0                  // NaN check
    b.vs    .LbadB
    fmov    s2, #0.0
    fcmp    s0, s2
    b.lt    .Lzero                  // negative sdrBrightness -> screen off (0)
    b       .LhaveCandidate

.LhaveDirectS1:
    fmov    s0, s1                  // displayBrightness in s1
.LhaveDirectS0:
    fcmp    s0, s0                  // NaN check
    b.vs    .LbadB
    fmov    s2, #0.0

.LhaveCandidate:
    str     s0, [x4, #0x030]        // record raw input float for diagnostic
    fcmp    s0, s2
    b.le    .Lzero                  // <= 0.0f means off (0)

    ldr     s1, [x4, #0x010]        // 1.0f
    fcmp    s0, s1
    b.gt    .LclampMax              // > 1.0f -> clamp to max

    // Apply perceptual curve from precomputed LUT at state_page + 0x200
    ldr     s2, [x4, #0x014]        // 255.0f
    fmul    s2, s0, s2              // x = s0 * 255.0f
    fcvtzs  w13, s2                 // i = floor(x)
    cmp     w13, #255
    b.ge    .LlutMax
    scvtf   s3, w13                 // (float)i
    fsub    s3, s2, s3              // frac = x - (float)i
    add     x14, x4, #0x200         // &lut[0]
    ldr     s4, [x14, w13, uxtw #2] // y0 = lut[i]
    add     w15, w13, #1
    ldr     s5, [x14, w15, uxtw #2] // y1 = lut[i+1]
    fsub    s5, s5, s4              // y1 - y0
    fmadd   s0, s3, s5, s4          // s0 = y0 + frac * (y1 - y0)
    b       .Lscale

.LlutMax:
    fmov    s0, #1.0

.Lscale:
    // Normalised curved float in (0.0, 1.0] -> scale to [min_bri, max_bri]
    ldr     w10, [x4, #0x004]       // min_bri
    ldr     w11, [x4, #0x008]       // max_bri
    sub     w12, w11, w10           // max - min
    scvtf   s1, w12
    fmul    s1, s0, s1              // (max - min) * curved_float
    fcvtzs  w9, s1
    add     w9, w9, w10             // val = min + (max - min) * curved_float
    b       .LhaveLevel

.LclampMax:
    ldr     w9, [x4, #0x008]        // max_bri
    b       .LhaveLevel

.Lzero:
    mov     w9, #0

.LhaveLevel:
    ldr     w6, [x4, #0x020]        // call counter
    add     w6, w6, #1
    str     w6, [x4, #0x020]

    ldr     w12, [x4, #0x00c]       // last_val
    cmp     w9, w12
    b.eq    .Lok
    str     w9, [x4, #0x00c]

    // Format decimal ASCII into stack scratch
    sub     x14, sp, #32
    add     x12, x14, #16
    mov     x16, x9
    mov     w13, #10
.Ldigit:
    udiv    x17, x16, x13
    msub    x18, x17, x13, x16
    add     w18, w18, #48
    sub     x12, x12, #1
    strb    w18, [x12]
    mov     x16, x17
    cbnz    x16, .Ldigit

    add     x2, x14, #16
    sub     x2, x2, x12             // len
    add     x17, x12, x2
    mov     w18, #10                // newline '\n'
    strb    w18, [x17]
    add     x2, x2, #1

    ldr     w0, [x4, #0x000]        // led_fd
    mov     x1, x12                 // buffer
    mov     x8, #64                 // __NR_write
    svc     #0
    str     w0, [x4, #0x018]        // wresult

.Lok:
    mov     w0, #0                  // status_t NO_ERROR
    ret

.LbadB:
    mov     w0, #-22                // -EINVAL
    ret

    .balign 0x400
dptr:
    .quad    0

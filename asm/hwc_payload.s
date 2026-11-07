// Payload for the composer-side brightness hook.
//
// The code and the mutable state live in two different mappings:
//
//   * the code page is a private file backed executable mapping of a library
//     the composer already loads. An anonymous PROT_EXEC mapping needs the
//     `execmem` permission, which the composer domain does not have.
//   * the state page is a plain anonymous read/write mapping.
//
// The state page address is patched into the pointer slot at 0x300 when the
// hook is installed, so the stub stays position independent.
//
// Code page layout:
//   0x000  stub A - displayGetBrightnessSupport replacement
//   0x200  stub B - displaySetBrightness replacement
//   0x300  dptr   - state page address, patched at install time
//
// State page layout:
//   0x000  state   - { int32 led_fd, min_bri, max_bri, last_val }
//   0x010  floats  - { float 1.0f, float 0.0f }
//   0x018  wresult - last write() result, diagnostics only
//   0x100  path    - backlight sysfs path, NUL terminated
//
// AAPCS64 for the two HWC2 entry points:
//   displayGetBrightnessSupport(hwc2_device_t*, hwc2_display_t, bool*)
//       x0 = dev, x1 = display, x2 = bool* outSupport, returns int
//   displaySetBrightness(hwc2_device_t*, hwc2_display_t, float)
//       x0 = dev, x1 = display, s0 = brightness, returns int
//
// Only x9-x18 are touched: those are the caller-saved temporaries and the
// intra-procedure-call scratch registers, so the callee-saved state of the
// replaced function stays intact.

    .text
    .balign 4096
page:
    // ---- stub A: displayGetBrightnessSupport ----
    adr     x5, dptr
    ldr     x4, [x5]                // state page
    cbz     x2, .LbadA
    ldr     w6, [x4, #0x01c]        // call counter
    add     w6, w6, #1
    str     w6, [x4, #0x01c]
    cbnz    x1, .LnotPrimary
    mov     w5, #1
    strb    w5, [x2]
    mov     w0, #0
    ret
.LnotPrimary:
    mov     w5, #0
    strb    w5, [x2]
    mov     w0, #0
    ret
.LbadA:
    mov     w0, #4
    ret

    .balign 0x200
stubB:
    // ---- stub B: displaySetBrightness ----
    adr     x5, dptr
    ldr     x4, [x5]                // state page
    ldr     s1, [x4, #0x010]        // 1.0f
    fmov    s2, #0.0                // 0.0f
    fcmp    s0, s0                  // NaN check
    b.vs    .LbadB
    str     s0, [x4, #0x030]        // record raw input float for diagnostic
    fcmp    s0, s1                  // > 1.0f
    b.gt    .LbadB
    fcmp    s0, s2                  // <= 0.0f means off
    b.le    .Lzero

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
    ldr     w10, [x4, #0x004]       // min_bri
    ldr     w11, [x4, #0x008]       // max_bri
    sub     w12, w11, w10
    scvtf   s1, w12
    fmul    s1, s0, s1
    fcvtzs  w9, s1
    add     w9, w9, w10
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

    // Decimal conversion. The digits are produced least significant first
    // into the top of a 16 byte scratch area, which leaves them in
    // ascending address order with the most significant digit first.
    sub     x14, sp, #32            // scratch base
    add     x12, x14, #16           // digit cursor, walks down
    mov     x16, x9
    mov     w13, #10
.Ldigit:
    udiv    x17, x16, x13
    msub    x18, x17, x13, x16      // remainder
    add     w18, w18, #48
    sub     x12, x12, #1
    strb    w18, [x12]
    mov     x16, x17
    cbnz    x16, .Ldigit

    add     x2, x14, #16
    sub     x2, x2, x12             // length of the digit string
    add     x17, x12, x2            // one past the last digit
    mov     w18, #10
    strb    w18, [x17]              // newline
    add     x2, x2, #1

    ldr     w0, [x4, #0x000]        // led_fd
    mov     x1, x12                 // buffer, most significant digit first
    mov     x8, #64                 // __NR_write
    svc     #0
    str     w0, [x4, #0x018]        // write() result, for diagnostics only
.Lok:
    mov     w0, #0
    ret
.LbadB:
    mov     w0, #4
    ret

    .balign 0x400
dptr:
    .quad    0                      // state page address, patched at install time

# OPlus Brightness Adaptor for Transsion (v6@1.0)

A native Rust AIDL HAL (`vendor.oplus.hardware.displaypanelfeature.IDisplayPanelFeature/default`) and panel backlight adaptor for OxygenOS and ColorOS ports on Transsion (Infinix and Tecno) and MediaTek devices.

## How OPlus Brightness Works: SurfaceFlinger to HWC

### 1. Stock OPlus Framework Architecture
On stock OxygenOS and ColorOS, display brightness dispatch follows a specialized display pipeline:

1. **Framework Calculation:** `DisplayPowerController.animateScreenBrightness` calculates the target brightness in an 11-bit domain (`222` to `8191`).
2. **Forced SurfaceControl Branch:** In standard Android, the framework checks `canSetBrightnessViaHwc` and falls back to `LightsService` (`LogicalLight.setBrightness`) if the composer cannot handle brightness. In OPlus framework smali (`LocalDisplayDevice.smali`), `mForceSurfaceControl` is hardcoded to `true`. This flag overrides display quirks and forces `BacklightAdapter.setBacklight` to route exclusively through `SurfaceControl.setDisplayBrightness`. The `LogicalLight` branch in the framework remains dead code.
3. **SurfaceFlinger Forwarding:** SurfaceFlinger receives the Binder call via `ISurfaceComposer::setDisplayBrightness` and forwards it to the Hardware Composer HAL through `HWC2::Display::setDisplayBrightness`.
4. **Native Vendor HWC Execution:** In native OPlus vendor implementations (such as `hwcomposer.mtk_common.so`), HWC initializes `LedDevice` with paths `/sys/class/leds/lcd-backlight/{brightness,max_brightness,min_brightness}` and links with `PanelChaplin::PanelChaplinWrapper::SetBrightness()`. When SurfaceFlinger delivers the brightness payload, `LedDevice::setBrightness` writes the value directly to the kernel sysfs node.

### 2. Failure Mode on Ported Vendor Trees
When porting OxygenOS or ColorOS to Transsion or generic MediaTek devices:

1. **Stock Vendor HWC:** The device uses the vendor's stock composer (for example, `hwcomposer.mt6893.so`).
2. **Unsupported Operation:** In stock Transsion HWC, `HWCMediator::displayGetBrightnessSupport` explicitly reports unsupported (`*outSupport = false`), and `HWCMediator::displaySetBrightness` returns error `0x8` (`HWC2_ERROR_UNSUPPORTED`). The stock binary contains no `LedDevice` implementation and no sysfs backlight path references.
3. **Silent Drop:** SurfaceFlinger drops the brightness commands because HWC reports no brightness support (`displayBrightness=nullopt`).
4. **Bypassed Light HAL:** Because `mForceSurfaceControl` is hardcoded to `true` in `services.jar`, the framework never falls back to `android.hardware.light.ILights`. The standard Light HAL remains uncalled.

### 3. The Adaptor Mechanism (One-Shot In-Process HWC2 Hook)
Brightness control is restored without patching framework JAR files and without
replacing the vendor HWC binary:

- **Dynamic discovery:** At startup the adaptor scans `/proc/*/maps` for the
  composer HAL process and the `hwcomposer.*.so` (or `libmtkhwc*.so`) it has
  mapped. No MediaTek-specific path or offset is compiled in.
- **Dynamic symbol resolution:** The two HWC2 entry points
  (`displayGetBrightnessSupport`, `displaySetBrightness`) are resolved by
  name from the library's own `.dynsym`/`.symtab`, and their runtime
  addresses are computed from the ELF program headers plus the target's
  mapping table.
- **One-shot in-memory patch:** Two pages are mapped into the composer
  process through `ptrace` remote syscalls. The **code** page is a private
  file-backed `PROT_READ|PROT_EXEC` mapping of a library the composer already
  loads — an anonymous executable mapping needs the `execmem` permission,
  which the composer domain does not have, and `MAP_PRIVATE` keeps every
  write inside the process so the file on disk is never touched. The
  **state** page is a separate anonymous `PROT_READ|PROT_WRITE` mapping, and
  the state page address is patched into a pointer slot in the code page so
  the stubs stay position independent. The backlight node is opened *inside
  the composer* so every write happens under the composer's own SELinux
  label. The two entry points are then replaced with a branch into the code
  page; the branch has a ±128 MB reach, so the free gaps in the target's
  map table are searched for a usable code page address.
- **Panel output:** The patched `displaySetBrightness` receives the normalized
  `0.0..1.0` float from SurfaceFlinger, maps it onto the panel range
  (`min_brightness`..`max_hw_brightness`), and writes the value to
  `/sys/class/leds/lcd-backlight/brightness` itself. Redundant frames are
  skipped by the payload's own last-value comparison.
- **No background work:** The installer runs once and exits. There is no
  property listener, no poll thread and no second writer on the backlight
  node. The `android.hardware.light.ILights` client is not used at all.

The payload is assembled from `asm/hwc_payload.s`; `asm/hwc_payload.bin` and
`src/hwc_payload.rs` are generated from it and kept in sync.

### 4. Verified on device, and the one limit of this port

The hook mechanism was verified on a live device: the symbols are resolved
dynamically, the payload is installed into the running composer, and the
installed stub was driven with a normalized value of `0.5`, which scaled into
the panel range and wrote `256` to `/sys/class/leds/lcd-backlight/brightness`
through the composer's own descriptor.

What was **not** observed on this port is any framework traffic reaching those
entry points. Call counters inside the patched functions stayed at zero across
a SurfaceFlinger restart and a slider change, and a software-breakpoint trace
confirmed it: with `HWCMediator::displayPresent` trapping 953 times as a
control, the composer's `_hidl_getDisplayBrightnessSupport` and
`_hidl_setDisplayBrightness` recorded no hits at all.

The reason is the composer interface on this port. The vendor manifest
declares the composer as **version 2.1**, and
`android.hardware.graphics.composer@2.1.so` has no `DisplayBrightness`
symbols at all — those transactions only exist from V2_3 on. The composer
service is also built on `libhwc2on1adapter.so`, so it presents an HWC1 device
to the framework, and `HWC2On1Adapter::Display` implements no brightness
methods either. SurfaceFlinger itself reports `displayBrightness=nullopt`.

So the framework's brightness path stops at the V2_1 client boundary and
never produces a brightness transaction for the vendor HWC2 to receive. On this
port the hook installs cleanly, is idempotent on a second run, and stays idle;
the only live framework→vendor brightness channel remains the
`debug.tracing.screen_brightness` property. The hook engages on any port whose
composer exposes the V2_3+ brightness API. See `analysis.md` section 9 for the
full transcripts and the control measurements.

---

## Important Notes

- Do not delete stock light HALs. Run this service alongside them.

---

## Features in v6@1.0

- **One-Shot Dynamic Framework/HWC2 Hooks:**
  - **Framework GUI Hook (`system_server` / `libgui.so`):** Injects into `system_server` and intercepts `SurfaceComposerClient::setDisplayBrightness` and `SurfaceComposerClient::getDisplayBrightnessSupport`. Bypasses SurfaceFlinger, HWC1/HWC2 composer versions, and Light HAL. Scales normalized float directly to panel range and writes to `/sys/class/leds/lcd-backlight/brightness` under `system_server`'s native SELinux permissions.
  - **Composer Hook (`hwcomposer.*.so`):** Injects into `android.hardware.graphics.composer` for devices where HWC exposes V2_3+ brightness transactions.
- **Zero Polling & Zero Races:** No property polling, no background loop overhead, no dual-writer contention.
- **Native AIDL `IDisplayPanelFeature` Implementation:** Implements `vendor.oplus.hardware.displaypanelfeature.IDisplayPanelFeature/default` in Rust using `rsbinder`.
- **UDFPS `FP_TYPE` (`0xd3`) Support:** Returns `4` (`BIT_HW_DIMLAYER`) to suppress OPlus `OnScreenFingerprintDimLayer` 92% black overlay on lockscreens.
- **Property Overrides for `fp_type`:** Reads `ro.vendor.display.fp_type` and `persist.sys.rianixia.display.fp_type` with validation against invalid non-positive values.
- **Kernel & Sysfs Auto-Detection:** Automatically detects backlight bounds from `/sys/class/leds/lcd-backlight/` with fallback logic.

---

## Installation

For Android 14 (OS 14) specific instructions, refer to [readme-os14.md](readme-os14.md).

### Step 1: Copy Files to Vendor

File layout:

```text
/vendor/bin/hw/vendor.xia.display.adaptor-V6@1.0-service
/vendor/etc/init/init.xia.display.adaptor.rc
```

Do not add the ODM binary included in old packages. Those were intended for OS 14.

### Step 2: Set Binary File Context

Add the following entry to `file_contexts`:

```text
/vendor/bin/hw/vendor\.xia\.display\.adaptor-V6@1\.0-service    u:object_r:mtk_hal_light_exec:s0
```

### Step 3: Add Debug Property (Optional)

```text
persist.sys.rianixia.display-debug=true
```

---

## SEPolicy

Address missing permissions manually or verify SELinux denials in logcat (`Xia-DisplayAdaptor`). Different kernel trees label sysfs backlight nodes under different SELinux contexts.

### Required for the HWC2 hook

The adaptor reaches into the composer process, so it needs `ptrace` on the
composer domain, and the composer needs to be allowed to write the backlight
node:

```tepolicy
# The adaptor attaches to the composer HAL.
allow hal_allocator_default hal_graphics_composer_default:process ptrace;
allow hal_allocator_default hal_graphics_composer_default:process getattr;
allow hal_allocator_default capability sys_ptrace;

# The composer writes the panel backlight node itself.
allow hal_graphics_composer_default sysfs_backlight_file:file rw_file_perms;
allow hal_graphics_composer_default sysfs_leds_31_0:file rw_file_perms;
```

The init rc grants the capabilities the `ptrace` attach needs:

```text
capabilities SYS_PTRACE DAC_OVERRIDE
```

The shipped vendor policy is precompiled, so these rules have to be added to
the policy source and the policy rebuilt; `vendor/etc/selinux/vendor_sepolicy.cil`
is the file that carries them.

---

## Display Configuration

### IPS LCD Displays

Functions with default configuration. If issues occur, refer to AMOLED properties below.

### AMOLED Displays

Inspect `my_product` and remove properties containing `vrr`, `brightness`, `silky`, or `underscreen`. Then add the following vendor properties:

```text
persist.sys.tran.brightness.gammalinear.convert=1
ro.vendor.transsion.backlight_hal.optimization=1
ro.transsion.backlight.level=-1
ro.transsion.physical.backlight.optimization=1
```

---

## Runtime Configuration

- The adaptor auto-detects `max_brightness` and `min_brightness` from the kernel unless overridden via system properties.
- Set `persist.sys.rianixia.display-debug=true` for verbose logging in logcat (Tag: `Xia-DisplayAdaptor`).

### Core Properties

| Property                                  | Type | Default | Description                                                                                    |
| ----------------------------------------- | ---- | ------- | ---------------------------------------------------------------------------------------------- |
| `persist.sys.rianixia.brightness.mode`    | Int  | `0`     | Scaling algorithm: `0` = Curved (Gamma 2.2), `1` = Linear, `2` = Custom (75% in = 255 out)     |
| `persist.sys.rianixia.oplus.lux_aod`      | Bool | `false` | Enables Lux AOD handling. Prevents 0-brightness writes during Doze State 3.                    |
| `persist.sys.rianixia.display.fp_type`    | Int  | `4`     | Overrides `FP_TYPE` (feature `0xd3`). Value `4` disables `OnScreenFingerprintDimLayer`.        |
| `ro.vendor.display.fp_type`               | Int  | `4`     | Read-only vendor default for `FP_TYPE`. Takes precedence over stale persist properties.        |
| `persist.sys.rianixia.brightness.isfloat` | Bool | `false` | Set to `true` if the ROM writes float brightness strings to `debug.tracing.screen_brightness`. |
| `persist.sys.rianixia.display-debug`      | Bool | `false` | Enables verbose debug logging to logcat (Tag: `Xia-DisplayAdaptor`).                           |

### Hardware Overrides

| Property | Type | Description |
|---|---|---|
| `persist.sys.rianixia.custom.devmax.brightness` | Int | Manually overrides the maximum hardware brightness value. |
| `persist.sys.rianixia.hw_max` | Int | Cached hardware max brightness detected from sysfs. |
| `persist.sys.rianixia.hw_min` | Int | Cached hardware min brightness detected from sysfs. |

### Legacy DisplayPanel Mode (OS 14)

Relevant only if `persist.sys.rianixia.is-displaypanel.support` is `true`.

| Property | Type | Description |
|---|---|---|
| `persist.sys.rianixia-display.min` | Int | Input range minimum (default: `22`). |
| `persist.sys.rianixia-display.max` | Int | Input range maximum (default: `5118`). |

---

## Scaling Modes

- **Curved (Mode 0):** Standard Gamma 2.2 approximation. Aligns with human perceptual brightness curves.
- **Linear (Mode 1):** Direct proportional mapping between framework and hardware limits.
- **Custom (Mode 2):** Bi-linear curve mapping 75% of input to hardware 255 (midpoint on 511 scale) with steeper upper-range progression.

---

## Lux and AOD Handling

When `persist.sys.rianixia.oplus.lux_aod` is enabled:
- Prevents writing `0` to sysfs backlight during Doze State 3 to avoid panel blackouts during AOD transitions.
- Handles edge-case panel reports of raw brightness `2937.773`.

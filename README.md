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

### 3. The Adaptor Mechanism (Kernel Futex Property Notification)
To restore brightness control without patching framework JAR files or replacing the vendor HWC binary:

- **Property Mirroring:** When `DisplayPowerController.animateScreenBrightness` runs, it updates the system property `debug.tracing.screen_brightness` on every animation frame and slider movement.
- **Kernel Futex Listener:** Instead of periodic polling, the adaptor uses Bionic's `__system_property_wait` API (`sys/system_properties.h`). The worker thread sleeps directly on the kernel futex associated with the property serial number.
- **Instant Wakeup:** When Android's `property_service` updates the property, the kernel issues a `FUTEX_WAKE` syscall. The adaptor wakes instantly with sub-millisecond latency and 0.0% idle CPU usage.
- **Hardware Output:** The adaptor normalizes the 11-bit OPlus range (`222..8191`) to the panel hardware range (typically `1..511`), applies the selected gamma curve, and writes directly to `/sys/class/leds/lcd-backlight/brightness`.

---

## Important Notes

- Do not delete stock light HALs. Run this service alongside them.

---

## Features in v6@1.0

- **Kernel Futex Notification:** Replaces 50ms property polling with Bionic's `__system_property_wait`, reducing idle CPU consumption to 0.0%.
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

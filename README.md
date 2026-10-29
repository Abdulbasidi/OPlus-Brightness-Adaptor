### OPlus-Brightness-Adaptor-For-Transsion (v6@1.0)

A unified native Rust AIDL HAL (`vendor.oplus.hardware.displaypanelfeature.IDisplayPanelFeature/default`) and backlight adaptor for OPlus/Oppo/Realme (OxygenOS / ColorOS) ports on Transsion (Infinix / Tecno) and other MediaTek devices.
*Please give credit to this REPO or my @ when using.*

The code is open source because I decided not to gatekeep it. If you know a better logic or method, please share! I'm always listening to feedback—even if my responses are rough. Thank you\~

### Download
**[GitHub Releases page](https://github.com/ryanistr/OPlus-Brightness-Adaptor-For-Transsion/releases)**.

## ⚠️ Important

**DO NOT DELETE your stock Transsion light HALs.**
Just add this alongside them.

---

## What's New in v6@1.0

* **Native AIDL `IDisplayPanelFeature` Implementation**: Implements `vendor.oplus.hardware.displaypanelfeature.IDisplayPanelFeature/default` in pure Rust with `rsbinder`.
* **UDFPS `FP_TYPE` (0xd3) Support**: Handles fingerprint display type queries. Returns `4` (`BIT_HW_DIMLAYER`) to suppress the legacy `OnScreenFingerprintDimLayer` 92% black overlay that causes severe screen dimming on lockscreens.
* **Property Overrides for `fp_type`**: Reads `ro.vendor.display.fp_type` and `persist.sys.rianixia.display.fp_type` with validation against invalid non-positive values.
* **Realtime Brightness Tracing**: Dedicated background thread monitors `debug.tracing.screen_brightness` for immediate reaction to SystemUI and Settings slider movements.
* **Kernel & Sysfs Auto-Detection**: Auto-detects backlight limits from `/sys/class/leds/lcd-backlight/` with fallback logic.

---

## How to Use

> **Note:** Originally made for OS15 ROMs. For OS14 or lower follow this guide instead:
For Android 14 (OS 14) specific instructions, see the [OS14 guide](readme-os14.md).

### Step 1: Copy Files to Vendor

File structure:

```
/vendor/bin/hw/vendor.xia.display.adaptor-V6@1.0-service
/vendor/etc/init/init.xia.display.adaptor.rc
```
DO NOT ADD THE ODM BINARY INCLUDED IN THE ZIP. THOSE ARE FOR OS14.

### Step 2: Set Binary File Context on file_contexts

```bash
/vendor/bin/hw/vendor\.xia\.display\.adaptor-V6@1\.0-service    u:object_r:mtk_hal_light_exec:s0
```

### Step 3: Add Debug Prop (Optional)

```bash
persist.sys.rianixia.display-debug=true
```

---

## SEPolicy

None.
Address manually or use permissive mode. Each device may have different sysfs labels.

---

## Display Types

### IPS LCD Displays

Should work as-is. If not, refer to AMOLED method.

### AMOLED Displays

Make sure to check `my_product` and remove props containing: `vrr`, `brightness`, `silky`, `underscreen`. Then add the following props in vendor properties:

```bash
# Brightness Props for AMOLED
persist.sys.tran.brightness.gammalinear.convert=1
ro.vendor.transsion.backlight_hal.optimization=1
ro.transsion.backlight.level=-1
ro.transsion.physical.backlight.optimization=1
```

## Runtime Notes

* The adaptor auto-detects `max_brightness` and `min_brightness` from the kernel unless overridden via properties.
* Enable `persist.sys.rianixia.display-debug=true` for verbose logging to diagnose scaling and AOD behavior (log tag: `Xia-DisplayAdaptor`).

### Core Configuration

| Property                                  | Type | Default | Description                                                                                  |
| ----------------------------------------- | ---: | ------: | -------------------------------------------------------------------------------------------- |
| `persist.sys.rianixia.brightness.mode`    |  Int |     `0` | Selects the scaling algorithm.                                                               |
|                                           |      |         | `0`: Curved (Gamma 2.2)                                                                      |
|                                           |      |         | `1`: Linear                                                                                  |
|                                           |      |         | `2`: Custom (75% in = 255 out)                                                               |
| `persist.sys.rianixia.oplus.lux_aod`      | Bool | `false` | Enables specific handling for Lux AOD panels.                                                |
|                                           |      |         | Prevents 0-brightness writes during Doze (State 3) and applies fix for raw value `2937.773`. |
| `persist.sys.rianixia.display.fp_type`    |  Int |     `4` | Overrides `FP_TYPE` (feature `0xd3`). Set `4` to disable OPlus `OnScreenFingerprintDimLayer`. |
| `ro.vendor.display.fp_type`               |  Int |     `4` | Read-only vendor default for `FP_TYPE`. Takes precedence over stale persist properties.     |
| `persist.sys.rianixia.brightness.isfloat` | Bool | `false` | Set to `true` if the ROM uses float brightness values in `debug.tracing.screen_brightness`.  |
| `persist.sys.rianixia.display-debug`      | Bool | `false` | Enables verbose debug logging to logcat (Tag: `Xia-DisplayAdaptor`).                         |

### Hardware Overrides

| Property                                        | Type | Description                                                                            |
| ----------------------------------------------- | ---: | -------------------------------------------------------------------------------------- |
| `persist.sys.rianixia.custom.devmax.brightness` |  Int | Manually override the maximum hardware brightness value used for scaling calculations. |
| `persist.sys.rianixia.hw_max`                   |  Int | (Auto-Generated) Cached hardware max brightness. Clear this to re-detect.              |
| `persist.sys.rianixia.hw_min`                   |  Int | (Auto-Generated) Cached hardware min brightness. Clear this to re-detect.              |

### Legacy / DisplayPanel Mode (OS 14)

These properties are only relevant if `persist.sys.rianixia.is-displaypanel.support` is set to `true`.

| Property                           | Type | Description                            |
| ---------------------------------- | ---: | -------------------------------------- |
| `persist.sys.rianixia-display.min` |  Int | Input range minimum (default: `22`).   |
| `persist.sys.rianixia-display.max` |  Int | Input range maximum (default: `5118`). |

---

## Scaling Modes

* **Curved (Mode 0)**: Uses a standard Gamma 2.2 approximation. Best for human perception.
* **Linear (Mode 1)**: Direct 1:1 mapping (normalized) between input and output ranges.
* **Custom (Mode 2)**: A specifically tuned curve where 75% of the input range maps to hardware value `255` (approx. 50% on 511 scale), with steeper scaling thereafter.

---

## Lux / AOD Behavior

* When `persist.sys.rianixia.oplus.lux_aod` is enabled, the adaptor:
  * Prevents writing `0` to the kernel backlight during Doze State 3 to avoid AOD blackouts.
  * Applies a special-case fix for panels reporting raw brightness `2937.773`.

---
# Enjoy
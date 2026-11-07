# OPlus Brightness Adaptor for Transsion (v6@2.0)

Native Rust AIDL service (`vendor.oplus.hardware.displaypanelfeature.IDisplayPanelFeature/default`) and in-process display interceptor for OxygenOS and ColorOS ports on Transsion (Infinix, Tecno) and MediaTek devices.

## Display Brightness Architecture

### 1. Stock OPlus Framework Pipeline
On stock OxygenOS and ColorOS, display brightness dispatch operates through a dedicated pipeline:

1. **Framework Calculation:** `DisplayPowerController` calculates the target brightness in an 11-bit domain (`222` to `8191`) and converts it to a normalized perceptual float `s0` in `[0.0, 1.0]`.
2. **Forced SurfaceControl Routing:** In standard Android, the framework evaluates `canSetBrightnessViaHwc` and falls back to `LightsService` (`LogicalLight.setBrightness`) if the composer lacks brightness capability. In OPlus framework smali (`LocalDisplayDevice.smali`), `mForceSurfaceControl` is hardcoded to `true`. This flag overrides display quirks and forces `BacklightAdapter.setBacklight` to route exclusively through `SurfaceControl.setDisplayBrightness`. The `LogicalLight` branch in the framework remains dead code.
3. **SurfaceFlinger Forwarding:** SurfaceFlinger receives the Binder transaction via `ISurfaceComposer::setDisplayBrightness` and forwards it to the Hardware Composer HAL through `HWC2::Display::setDisplayBrightness`.
4. **Native Vendor HWC Execution:** On native OPlus hardware (such as `hwcomposer.mtk_common.so`), HWC initializes `LedDevice` with kernel paths `/sys/class/leds/lcd-backlight/{brightness,max_brightness,min_brightness}` and links with `PanelChaplin::PanelChaplinWrapper::SetBrightness()`. When SurfaceFlinger delivers the brightness payload, `LedDevice::setBrightness` writes the value directly to the kernel sysfs node.

### 2. Failure Mode on Ported Vendor Trees
When porting OxygenOS or ColorOS to Transsion or generic MediaTek devices:

1. **Stock Vendor HWC:** The device uses the vendor stock composer (for example, `hwcomposer.mt6893.so`).
2. **Unsupported Operation:** In stock Transsion HWC, `HWCMediator::displayGetBrightnessSupport` explicitly reports unsupported (`*outSupport = false`), and `HWCMediator::displaySetBrightness` returns error `0x8` (`HWC2_ERROR_UNSUPPORTED`). The stock binary contains no `LedDevice` implementation and no sysfs backlight path references.
3. **Silent Drop:** SurfaceFlinger drops brightness commands because HWC reports no brightness support (`displayBrightness=nullopt`).
4. **Bypassed Light HAL:** Because `mForceSurfaceControl` is hardcoded to `true` in `services.jar`, the framework never falls back to `android.hardware.light.ILights`. The standard Light HAL remains uncalled.

---

## The Interceptor Mechanism

The adaptor intercepts and handles brightness without patching framework JAR files and without modifying vendor binaries on disk.

### 1. Framework GUI Hook (`system_server` / `libgui.so`)
On modern Android ports, HWC1/HWC2 composer implementations frequently drop or ignore brightness calls. The adaptor hooks `system_server` directly:
- **Dynamic Symbol Scoring:** Resolves mangled symbol variants for `SurfaceComposerClient::setDisplayBrightness` and `SurfaceComposerClient::getDisplayBrightnessSupport` directly from `libgui.so`'s ELF dynamic symbol table (`.dynsym`). Supports both standard struct argument passing (`gui::DisplayBrightness`) and direct float ABIs across AOSP versions.
- **In-Memory Injection:** Maps a code page (`PROT_READ|PROT_EXEC`, backed by a mapped library to satisfy SELinux without `execmem`) and a shared state page (`PROT_READ|PROT_WRITE`) into `system_server` using `ptrace` remote syscalls.
- **Direct Kernel Dispatch:** The injected AArch64 stub intercepts brightness requests, applies the dynamic perceptual curve, and writes the integer value directly to `/sys/class/leds/lcd-backlight/brightness` using a file descriptor opened inside `system_server`. Redundant writes are filtered by last-value checks.
- **Hot-Reloading:** When updated, the supervisor detects existing hooks and refreshes the payload instructions and lookup tables in-place without restarting `system_server`.

### 2. Composer Hook (`hwcomposer.*.so`)
For devices with HWC 2.3+ implementations that expose native composer brightness transactions, the adaptor provides an alternative in-process hook for `displayGetBrightnessSupport` and `displaySetBrightness` inside `android.hardware.graphics.composer`.

### 3. Native AIDL `IDisplayPanelFeature`
The adaptor registers `vendor.oplus.hardware.displaypanelfeature.IDisplayPanelFeature/default` in Rust using `rsbinder`.
- **Integrated Control:** Functions alongside the in-process GUI hook to satisfy OPlus framework panel bindings.
- **Clean Fingerprint Fallback:** All fingerprint and optical FOD feature codes (`0xd3` / `FP_TYPE`, `0x1c` / `FP_PRESS`, `0x16` / `DIMLAY_HBM`, `0x17` / `DIMLAYER_BL_EN`, and `179`) return `StatusCode::UnknownTransaction`. This deliberately delegates fingerprint operations to native MediaTek SurfaceFlinger fallback (`transact 20004` / `NOTIFY_DISP_FINGER_LAYER`), preventing screen flickers or color corruptions on lockscreens.

---

## Dynamic Perceptual Curve for All Display Types

Different display panels vary widely in maximum hardware brightness:
- 8-bit / 9-bit displays (e.g. LCD or standard AMOLED with `hw_max = 511`)
- 10-bit displays (`hw_max = 1023`)
- 11-bit displays (`hw_max = 2047`)
- 12-bit high-nit AMOLED displays (`hw_max = 4096`)

### Why Static Scaling Failed
In stock Android framework, `DisplayPowerController` already shapes the slider position into a perceptual float `s0` in `[0.0, 1.0]`. The framework curve is calibrated for standard displays:
- Slider at 50% outputs `s0 = 0.1004`
- Manual slider maximum outputs `s0 = 0.4999` (leaving `0.50` to `1.00` for Sunlight HBM)

Applying a fixed gamma curve (such as 1.8) on top of this float caused double-compounding on 511-count panels:
- `0.1004^1.8 = 0.0157`, resulting in 9 out of 511 (crushed, way too dim)
- `0.4999^1.8 = 0.2872`, resulting in 147 out of 511 (capped at 28% luminance indoors)

Conversely, pure linear scaling on a 4096-count display blew out the mid-range:
- `0.1004 * 4095 = 411` counts (~200 nits indoors)
- `0.2508 * 4095 = 1027` counts (>500 nits indoors at 35% slider)

### The Continuous Auto-Gamma Formulation
The adaptor dynamically computes gamma based on the panel bit-depth:

```text
bits = log2(hw_max)
if bits <= 9.0:
    gamma = 1.00
else:
    gamma = clamp(1.0 + (bits - 9.0) * 0.17, 1.0, 1.8)
```

Values across panel types:
- `hw_max <= 511` (8-bit / 9-bit): `gamma = 1.00`
  Linear passthrough of the framework perceptual float. Indoor 50% slider produces 52 counts, manual max produces 255 counts, and HBM produces 511 counts.
- `hw_max = 1023` (10-bit): `gamma = 1.17`
- `hw_max = 2047` (11-bit): `gamma = 1.34`
- `hw_max = 4096` (12-bit): `gamma = 1.51`
  Softens the expanded dynamic range. At 35% slider, output scales to ~130-240 counts instead of 600-1400 counts, preventing mid-range blowout.

### Hardware Lookup Table (LUT) Execution
Inside `system_server` and `composer`, floating-point library functions (`powf`, `log2`) cannot be linked safely. The adaptor precomputes a 256-entry float lookup table in the shared state page during initialization:

$$\text{lut}[i] = \left(\frac{i}{255.0}\right)^{\gamma(hw\_max)}$$

The AArch64 assembly stub performs piecewise-linear interpolation between adjacent table entries in twelve instructions. Execution is branchless, exact, and requires zero external floating-point library calls.

---

## Installation & Staging

### 1. Vendor Files
Stage the following files into `/vendor`:

```text
/vendor/bin/hw/vendor.xia.display.adaptor-V6@1.0-service
/vendor/etc/init/init.xia.display.adaptor.rc
/vendor/etc/vintf/manifest/manifest_rianixia_displaypanelfeature_aidl.xml
```

File permissions:
- Binary: `0755` (`u:object_r:hal_allocator_default_exec:s0` or `u:object_r:mtk_hal_light_exec:s0`)
- Configs: `0644` (`u:object_r:vendor_configs_file:s0`)

Package archive:
[obright_v6@2.0.zip](file:///mnt/project/RomTools/ROM_WORKDIR/oos/ReconstructedHALSource/Oplus/OBrightNew/obright_v6@2.0.zip) contains these three files rooted at `vendor/`.

### 2. Context of the MediaTek Light HAL
Do not remove the stock MediaTek Light HAL (`android.hardware.lights-service.mediatek` or `lights-mtk-default`). 

The OPlus framework hardcodes `mForceSurfaceControl = true`, making standard Light HAL brightness calls unreachable. However, the Light HAL remains responsible for notification LEDs, battery charging indicators, and secondary illumination. This adaptor runs alongside the stock Light HAL without conflict.

### 3. Init Configuration & Capabilities
The init configuration (`init.xia.display.adaptor.rc`) defines:

```text
service xia.display.adaptor /vendor/bin/hw/vendor.xia.display.adaptor-V6@1.0-service
    class hal
    user system
    group system
    capabilities SYS_PTRACE DAC_OVERRIDE
    interface aidl vendor.oplus.hardware.displaypanelfeature.IDisplayPanelFeature/default
    restart_period 5000
```

`SYS_PTRACE` permits the adaptor to attach to `system_server` or `composer` to map memory and install branches. `DAC_OVERRIDE` ensures initial directory creation under `/data/addon/oplus_display`.

---

## Configuration & Diagnostics

All multi-brightness tracing properties, legacy OS 14 property tables, and manual curve mode properties have been removed. The adaptor auto-detects panel boundaries and scales the curve dynamically.

### Optional Properties

| Property | Type | Default | Description |
| :--- | :--- | :--- | :--- |
| `persist.sys.rianixia.display-debug` | Bool | `false` | Enables verbose diagnostic logging in logcat under tag `Xia-DisplayAdaptor`. |
| `persist.sys.rianixia.custom.devmax.brightness` | Int | `None` | Manually overrides detected kernel maximum hardware brightness. |
| `persist.sys.rianixia.custom.devmin.brightness` | Int | `None` | Manually overrides detected kernel minimum hardware brightness. |

### Diagnostic Verification
To verify the active hook and telemetry on a running device:

```bash
# Run one-shot check against running system_server
adb shell su -c '/vendor/bin/hw/vendor.xia.display.adaptor-V6@1.0-service --gui-hook'

# Inspect live telemetry in logcat
adb logcat -d -s Xia-DisplayAdaptor:V
```

Sample output:
```text
D Xia-DisplayAdaptor: [GuiHook] matched setDisplayBrightness: "_ZN7android21SurfaceComposerClient20setDisplayBrightness..." (score=105, abi=Struct { offset: 16 })
D Xia-DisplayAdaptor: [GuiHook] system_server already patched: support_calls=1 set_calls=3981 raw_float=0.4999 last_val=255 wresult=4
```

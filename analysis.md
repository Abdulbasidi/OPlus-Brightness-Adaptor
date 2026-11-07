# OOS brightness on MediaTek — stock vendor/odm vs. our port

Read-only static analysis. No file in any ROM tree was modified, nothing was flashed or tested on device.

Trees:

- **STOCK** = `/mnt/project/RomTools/ROM_WORKDIR/oos15_old/erofs` (Oplus stock OOS, MT6983, has `vendor/` + `odm/`)
- **OURS**  = `/mnt/project/RomTools/ROM_WORKDIR/oos/erofs` (ported OOS, MT6893, `vendor/` only; `vendor/odm/` is a stub symlink)
- Framework smali trees for OURS = `/mnt/project/RomTools/ROM_WORKDIR/oos/RE/framework/{services-classes2, oplus-services, oplus-framework, systemui-classes*}`
- Our adaptor source = `/mnt/project/RomTools/ROM_WORKDIR/oos/ReconstructedHALSource/Oplus/OBrightNew`

Tools: REA/Ghidra 12.1.2 pseudocode + xrefs, `llvm-objdump`/`readelf`/`nm`/`strings`, smali grep.

---

## 1. Executive summary

| Question | Stock OOS (MT6983) | Ours (MT6893 port) |
|---|---|---|
| Who writes the panel backlight? | **HWC** (`hwcomposer.mtk_common.so` → `LedDevice::setBrightness`) | **Nothing in HWC.** Two writers instead: the vendor light HAL and our Rust adaptor |
| Backlight node | `/sys/class/leds/lcd-backlight/brightness` | `/sys/class/leds/lcd-backlight/brightness` (same) |
| HWC brightness support | Yes — per-display `supported` flag from a successful `open()` of the node | **No** — `displaySetBrightness` returns `HWC2_ERROR_UNSUPPORTED` (8); `displayGetBrightnessSupport` writes `*outSupport=false` |
| Framework route | `SurfaceControl.setDisplayBrightness` → SF → HWC. `ILights` path is dead code | Identical (framework unchanged) — so brightness reaches a HWC that refuses it |
| Who fills the gap | nothing needed | `vendor.xia.display.adaptor-V6@1.0-service` — listens on `debug.tracing.screen_brightness` and writes sysfs itself |
| Oplus panel feature HAL | `odm/bin/hw/vendor.oplus.hardware.displaypanelfeature@1.0-service` (HIDL/hwbinder), writes `/sys/kernel/oplus_display/*` and ioctls `/dev/oplus_display` | replaced by our AIDL adaptor; **no `/sys/kernel/oplus_display/*` labels/types exist in our tree** |
| Brightness float domain | framework level `0..4095` (normal max 2048) | same framework, but our adaptor assumes 0..255 → hw_max |

Two structural facts drive everything else:

1. OOS forces `mForceSurfaceControl = true`, so the framework **never** uses `android.hardware.light.ILights` for panel brightness. Brightness only reaches the panel through SurfaceFlinger → HWC2 → `displaySetBrightness`.
2. Our HWC (`hwcomposer.mt6893.so`) implements that callback as a stub. Therefore the stock chain terminates in a dead end, and the adaptor exists to re-implement the missing tail of it.

---

## 2. Framework side (system + system_ext) — identical in both trees

### 2.1 Brightness domain is an Oplus "level", not 0..255 and not nits

`RE/framework/services-classes2/com/android/server/display/OplusSmartBrightnessController.smali` is in `oplus-services.jar`:

```
1298: const/high16 v0, 0x42cc0000   # 102.0f   -> mDefaultDisplayBrightness
1301: const/high16 v0, 0x45000000   # 2048.0f  -> mMaxDisplayBrightness
1324: const/4      v6, 0x0          # 0.0f     -> mMinDisplayBrightness
1329: const       v7, 0x457ff000    # 4095.0f  -> mTotalDisplayBrightness
```

`clampBrightness(FI)` (8733) constrains with `MathUtils.constrain(min,max)` on those bounds. AOSP's own clamp was removed:

`RE/framework/services-classes2/com/android/server/display/brightness/BrightnessUtils.smali`

```
16: .method public static clampAbsoluteBrightness(F)F
21:   return p0          # AOSP's 0..1 clamp removed
```

So the framework float brightness is a level in `0..4095`, normal ceiling 2048, default 102. `DisplayBrightnessController.smali:175-177` fills the AOSP "range" fields from the Oplus extension instead of 0..1.

### 2.2 The property our adaptor listens to

`RE/framework/services-classes2/com/android/server/display/DisplayPowerController.smali`

```
2058: .method private animateScreenBrightness(FFF)V     # p1 = target
2116: const-string v0, "debug.tracing.screen_brightness"
2120: invoke-static {p1}, Ljava/lang/String;->valueOf(F)Ljava/lang/String;
2127: invoke-static {v0, v1}, Landroid/os/SystemProperties;->set(Ljava/lang/String;Ljava/lang/String;)V
```

The property carries the **raw level float** (`String.valueOf(target)`) — no rescaling. It is written **once per animation request**, not per frame; per-frame values flow through `RampAnimator$DualRampAnimator` into `DisplayPowerState.SCREEN_BRIGHTNESS_FLOAT`.

### 2.3 `mForceSurfaceControl` is forced true → the `ILights` path is dead code

`RE/framework/services-classes2/com/android/server/display/LocalDisplayAdapter$LocalDisplayDevice.smali`

```
1239: .method private loadDisplayDeviceConfig()V
1282: const-string v3, "canSetBrightnessViaHwc"
1288: invoke-virtual {v1, v2}, ...BacklightAdapter;->setForceSurfaceControl(Z)V   # quirk value
1291: iget-object v1, ...->mBacklightAdapter
1293: const/4 v2, 0x1
1295: invoke-virtual {v1, v2}, ...BacklightAdapter;->setForceSurfaceControl(Z)V   # overwritten: TRUE
```

In `LocalDisplayAdapter$BacklightAdapter.smali` the `LogicalLight.setBrightness(F)` calls at lines **135, 222, 338** are guarded by `mForceSurfaceControl` and therefore unreachable. `mUseSurfaceControlBrightness` comes from `SurfaceControlProxy.getDisplayBrightnessSupport()` — a HWC capability query. Consequence: in OOS the vendor light HAL's `LightType::BACKLIGHT` entry is **never driven by the framework**.

### 2.4 Where level → panel value happens

Not a bit shift. It is a **table lookup** against the vendor XML:

- `OplusDisplayBrightnessConfig.smali:1280-1292` → `mMaxLevel = 0x7ff` (2047), `mCurveMapRange = 0xfff` (4095), `mMinLevel = 4` (overridden by the XML `min`/`max`/`range` attributes in `OplusDisplayBrightnessConfig$OplusDisplayBrightnessConfigParser.smali:1905-1922, 2025-2032`).
- `OplusDisplayBrightnessApolloModel.getBacklightFromBrightness(F)F` (2087) → `mPanelBacklights[(int)brightness] / mMaxPanelBrightness`.
- `mPanelBacklights` is filled from the `<brightness_table>` of `vendor/etc/display_brightness_config_Tianma4095.xml` (`<brightness_table max="8191" min="4">`, 10240 `<level>` rows, nits 1.97…1051.15).

`Settings.System.SCREEN_BRIGHTNESS` ("screen_brightness") is parsed as an **int** and converted `int-to-float` with **no /255** (`DisplayPowerController.smali:12887-12899`, `DisplayBrightnessController.smali:973-988`) — the setting name is reused for a different scale. No `BRIGHTNESS_OVERRIDE` / `screen_brightness_float` in the decompiled trees (they would live in `framework.jar`, not decompiled here — unknown).

### 2.5 Oplus panel-feature HAL usage

Interface: `vendor.oplus.hardware.displaypanelfeature.IDisplayPanelFeature/default` (AIDL), with a HIDL `V1_0` fallback — `RE/framework/services-classes2/com/android/server/display/util/AidlDisplayPanelFeature.smali:815-820`, dispatcher `OplusDisplayPanelFeatureHelper.smali`.

Feature IDs observed in `OplusDisplayPanelFeatureHelper` (lines 7-91) and in SystemUI: `0x1/0x2/0x3` power, `0x4` seed, `0x5` panel id, `0x6` FFL, `0x7` AOD, **`0x8` max brightness**, `0x9` panel info, `0xc` HBM, `0xd` dim alpha, `0xe` dim DC alpha, `0xf` audio ready, `0x11` DSC, `0x13` regulator, `0x15` panel reg, **`0x16` DIMLAY_HBM**, `0x17` DIMLAYER_BL_EN, `0x18` blank, `0x19` SPR, `0x1b` DOC, **`0x1c` FPPRESS**, **`0x1d` Oplus brightness**, `0x21` LCD max brightness (get), `0x44` PQ trigger, `0xd2` shutdown flag, **`0xd3` FP type** (bit `0x4` = HW dim layer), `0xd5` fake AOD, `0xf5` auto max brightness, `0xf7` normal max brightness, `0xc7` PWM turbo, `0xc9` PWM pulse.

`0xd3` is *not* in the helper's table; it is a local constant in `AidlDisplayPanelFeature.smali:883-905` (`isFODHwDimlayer()` = value `0xd3 & 0x4`) and in `HidlDisplayPanelFeature.smali:674`. SystemUI reads it too (`systemui-classes6/.../KeyguardFeatureOption$isLocalHBM$2.smali:74`, `...$isDisableAppDimLayer$2.smali:74`).

---

## 3. Stock vendor: how OOS actually sets brightness on MediaTek

### 3.1 HWC owns the write (`vendor/lib64/hw/hwcomposer.mtk_common.so`)

`LedDevice` methods (Ghidra addresses):

| Symbol | addr |
|---|---|
| `LedDevice::getInstance()` | 0x0f345c |
| `LedDevice::LedDevice()` | 0x0f34d4 |
| `LedDevice::initLedState(ulong, DisplayLedState*, char const*, char const*, char const*)` | 0x0f355c |
| `LedDevice::readIntFromPath(char const*, int*)` | 0x0f3940 |
| `LedDevice::getBrightnessSupport(ulong, bool*)` | 0x0f3d28 |
| `LedDevice::setBrightness(ulong, float)` | 0x0f3e34 |
| `LedDevice::dump(String8*)` | 0x0f4670 |

The only LED sysfs literals in the binary:

```
/sys/class/leds/lcd-backlight/brightness
/sys/class/leds/lcd-backlight/max_brightness
/sys/class/leds/lcd-backlight/min_brightness
```

`initLedState` reads min/max, then `__open_2(led_path, O_RDWR)`; **support is set only if that open succeeds** — i.e. HWC advertises brightness support iff the node exists.

`setBrightness` (0x0f3e34) semantics:

```c
if (!st->supported) return 8;                       // unsupported display
if (st->float_cur == f) return 0;                    // no change
if (f != -1.0 && f != 0.0) {
    if (f < 0.0 || f > 1.0) return 4;                // invalid
    value = st->min + (int)((float)(st->max - st->min) * f);   // linear [0,1] -> [min,max]
} else value = 0;
if (value != st->int_cur) {
    PanelChaplinWrapper::GetInstance();
    PanelChaplinWrapper::SetBrightness(value);       // notify colour pipeline FIRST
    snprintf(buf, 0x10, "%d", value);
    write(st->fd, buf, len);                         // then the sysfs node
}
```

HWC2 callbacks: `HWCMediator::displaySetBrightness` @ 0x080f50 → `HWCDisplay::setBrightness` @ 0x0e10f4 → `LedDevice::setBrightness`; `HWCMediator::displayGetBrightnessSupport` @ 0x07bbd4 → `HWCDisplay::getBrightnessSupport` @ 0x0e10c8 → `LedDevice::getBrightnessSupport` @ 0x0f3d28. No `HWC2_CAPABILITY_*` brightness bit was found — advertisement is via the callback.

`hwcomposer.mtk_fold.so` carries the same class in a newer shape: `initLedState(DisplayLedState*)` builds the paths at runtime from the fragments `/sys/class/leds` + `lcd-backlight` + `max_brightness`/`min_brightness` (no full literal), reads a connector id, and adds `HWCLayer::setBrightness(float)` @ 0x00e7510. Scaling and write semantics are identical.

### 3.2 PanelChaplin — colour/LUT, not the backlight node

- `vendor/lib64/libPanelChaplin_wrapper.so` — a dlopen shim: `openLib("libPanelChaplin_mtk.so")` → `dlsym("FETCH_IPanelChaplin")`; config dir `/odm/etc/PanelChaplin`.
- `PanelChaplinWrapper::SetBrightness(int)` (0x102cb8) forwards to the HIDL service: `IPanelChaplin::configureSet(0x7b /*123*/, {value})`, only when the main panel is powered on.
- `vendor/lib64/vendor.display.PanelChaplin@1.0.so` is the **HIDL** (hwbinder) proxy/stub for `vendor.display.PanelChaplin@1.0::IPanelChaplin`. (No AIDL.)
- `odm/lib64/libPanelChaplin_mtk.so` implements it. `PanelChaplinService::configureSet` case `0x16` (cmd 0x7b): stores the raw value, clamps to `0x1fff` (8191, 12-bit), computes `index = (8191 - bri) / 100` (0..81) and regenerates/selects the 3D LUT via the MediaTek PQ service (`PQFeatureID_C3D`, `PQFeatureID_GAMMA`).
- The **only** sysfs string in `libPanelChaplin_mtk.so` is `/sys/kernel/oplus_display/panel_serial_number`; it contains **no** `lcd-backlight` and no `ioctl` import. PanelChaplin does not write the backlight node — it only re-tunes the colour LUT for the new brightness.

So the stock backlight write is HWC-only; PanelChaplin rides along for colour.

### 3.3 The Oplus panel-feature HAL (stock `odm/`)

- Interface: HIDL `vendor.oplus.hardware.displaypanelfeature@1.0::IDisplayPanelFeature` (hwbinder), with an AIDL NDK copy at `system/system/lib64/vendor.oplus.hardware.displaypanelfeature-V1-ndk.so`. Methods: `getDisplayPanelFeatureValue`, `setDisplayPanelFeatureValue`, `getDisplayPanelInfo`.
- Service binary: `odm/bin/hw/vendor.oplus.hardware.displaypanelfeature@1.0-service`, opens `/dev/oplus_display`.
- Feature id = `param & 0xfff`, display id = `param >> 0xc`.
- **Brightness-related feature ids**: `0x8` (max brightness), `0x1d` (Oplus brightness), `0xc5` (DC real brightness), `0x21` (LCD max brightness, get-only). `0xd3` = `SetDisplayFpType`, `0xd5` = fake AOD — **not** brightness.
- Mechanism: `ioctl(fd, 0x40046f2a, &v)` for Oplus brightness, `ioctl(fd, 0x40046f0a, &v)` for max brightness on `/dev/oplus_display`, with sysfs fallbacks to `/sys/kernel/oplus_display/{oplus_brightness, max_brightness, hbm, dim_alpha, dim_dc_alpha, dimlayer_hbm, seed, ffl_set, notify_fppress, write_panel_reg}` (28 `oplus_display` node strings total in the binary).
- Stock init rc (`odm/etc/init/vendor.oplus.hardware.displaypanelfeature@1.0-service.rc`): `on early-boot` chowns `/dev/oplus_display` and 14 `/sys/kernel/oplus_display/*` nodes to system, then `service displaypanelfeature-1-0 … class hal animation / user system / group system`.

### 3.4 Stock light HAL is present but unused for panel brightness

`vendor/bin/hw/android.hardware.lights-service.mediatek` (55,168 B, AIDL `ILights/default`) *does* implement a panel-backlight entry: type id 0 → `/sys/class/leds/lcd-backlight/brightness`, type id 8 → `/sys/class/leds/lcd-backlight1/brightness` (dual-panel device). Value conversion: luma `(77R+150G+29B)>>8`, then rescaled by the panel's own range read from `max_brightness`/`min_brightness`: `(max-min)*(luma-1)/254 + min`. It also loads `/data/vendor/liblight/oplusLights.xml` (tinyxml2) for **LED colour** parameters, not brightness curves; that file ships in neither tree.

But nothing in the stock framework calls it for the panel: `LIGHT_TYPE_BACKLIGHT` does not appear in `framework.jar`/`libandroid_servers.so` (stock or ours), and the only binaries referencing `android.hardware.light.ILights` are the light service itself and `vendor/bin/factory`. The stock light HAL is effectively dead for screen brightness — a second, unused path to the same node.

### 3.5 Stock brightness XML

| File | Content |
|---|---|
| `vendor/etc/cust_silky_brightness.xml` (521,950 B) | `output_bits 12`, `backlight_level 10239`, `ASL_limit 8192`, 10240 `index_*` rows, backlight step 0..4095 |
| `system_ext/etc/display_brightness_config_default.xml` (502,439 B) | `brightness_table max="8191" min="222"`, 10240 `<level>` rows (level, backlight, nits, gamma), nits 0..800 |
| `system_ext/etc/display_brightness_config_common.xml` (75,027 B) | 48 lux tables + HBM tables, 1947 `<lux>` rows |

`cust_silky_brightness.xml` is consumed by the **MTK PQ HAL** (`vendor.mediatek.hardware.pq@2.15-impl.so`, `parseSilkyBrightnessTableFromXML`), not by the light HAL.

---

## 4. Our ported tree: how it differs

### 4.1 HWC has no brightness support at all

`vendor/lib64/hw/hwcomposer.mt6893.so` (1,205,032 B, sha256 `27ff1ff9…`):

```
00000000000745e8 T HWCMediator::displayGetBrightnessSupport(hwc2_device*, unsigned long, bool*)
0000000000078d68 T HWCMediator::displaySetBrightness(hwc2_device*, unsigned long, float)

78d68: mov w0, #0x8     # HWC2_ERROR_UNSUPPORTED
78d6c: ret
```

and `displayGetBrightnessSupport` ends with `7469c: strb wzr, [x20]` → `*outSupport = false`.

- **No `LedDevice` class** (`nm -C | grep -i led` → nothing; stock has `LedDevice::getBrightnessSupport` @ 0x1f3d28).
- **Zero `/sys/` strings** in the whole binary. Its complete device-path list: `/dev/dri/card0`, `/dev/graphics/fb`, `/devices/virtual/switch/{hdmi,res_hdmi,widevine}`, `/proc/`, `/proc/m4u`.
- Note: `vendor/build.prop` still says `ro.hardware.hwcomposer=mtk_common`, a filename absent from our tree; nothing in `vendor/etc/init/*.rc` references `hwcomposer.mt6893.so`. Whether it is loaded at runtime is **unknown** (statically it is unreferenced).

### 4.2 Two writers on the same node

`vendor/bin/hw/android.hardware.lights-service.mediatek` (ours, 29,032 B — a reduced build, no tinyxml2, no `lcd-backlight1`, no min/max read):

```
/sys/class/leds/lcd-backlight/brightness        <- LightType::BACKLIGHT (id 0)
/sys/class/leds/keyboard-backlight/brightness   <- id 1
/sys/class/leds/button-backlight/brightness     <- id 2
/sys/class/leds/jogball-backlight/brightness    <- ids 4,5
/sys/class/leds/{red,green,blue}/brightness     <- notification RGB
```

Value path: `rgbToBrightness = (77R+150G+29B)>>8`, then on the optimised branch `× 16.058823` (0..255 → 0..4095) before `write_int(...)`. Gated by `ro.vendor.transsion.backlight_hal.optimization` (exact polarity of the branch conditions **unknown**).

Our Rust adaptor `vendor/bin/hw/vendor.xia.display.adaptor-V6@1.0-service` (526,656 B) writes:

```
/sys/class/leds/lcd-backlight/brightness
/sys/class/leds/lcd-backlight/max_hw_brightness   (read; default 511 if missing)
/sys/class/leds/lcd-backlight/min_brightness      (read; default 1 if missing)
/data/addon/oplus_display/last_brightness
```

and listens on `debug.tracing.screen_brightness` (plus `debug.tracing.screen_state`, `sys.boot_completed`). It resolves `android.hardware.light.ILights/default` first and falls back to raw sysfs (`[DisplayAdaptor] Light HAL unavailable; using direct sysfs control`).

**Both write the same node with different scalings** (light HAL: 0..4095 fixed; adaptor: 0..hw_max, default 511). Which one wins at runtime is **unknown** — this needs device testing.

Also note the domain mismatch: the framework emits an Oplus **level** in `0..4095` (normal max 2048) into `debug.tracing.screen_brightness`, while the adaptor's own model (`src/scaling.rs`, `src/range.rs`) treats its input as 0..255-style and scales to hw_max. The adaptor therefore receives values up to 2048/4095 from the framework and must clamp.

### 4.3 PanelChaplin is orphaned in our tree

`vendor/lib64/libPanelChaplin_wrapper.so` and `libPanelChaplin_mtk.so` are present (md5-identical to stock's `odm/` copies) but:

- **no binary in our tree has `DT_NEEDED libPanelChaplin_wrapper.so` / `libPanelChaplin_mtk.so`**;
- no init rc or SELinux context references them;
- our HWC has **zero** PanelChaplin symbols (stock's HWC has 7 undefined refs).

So they are dead code here. Stock's `vendor/lib64/vendor.display.PanelChaplin@1.0.so` (the HIDL server side needed by the HWC) and the whole `odm/etc/PanelChaplin/` calibration directory (Tianma NT37705/NT4095 `.odf`, `MTK_Gamma_Table_12bit.txt`, PWM tables) are **absent** from our tree.

### 4.4 Our panel-feature service

`vendor/etc/init/init.xia.display.adaptor.rc`:

```
service xia.display.adaptor /vendor/bin/hw/vendor.xia.display.adaptor-V6@1.0-service
    class hal
    user system
    group system
    interface aidl vendor.oplus.hardware.displaypanelfeature.IDisplayPanelFeature/default
    restart_period 5000

on post-fs-data
    mkdir /data/addon 0777 system system
    mkdir /data/addon/oplus_display 0777 system system
    chown system system /data/addon/oplus_display
    chmod 0777 /data/addon/oplus_display
```

VINTF: `vendor/etc/vintf/manifest/manifest_rianixia_displaypanelfeature_aidl.xml` declares the AIDL HAL `vendor.oplus.hardware.displaypanelfeature / IDisplayPanelFeature/default`.

Versus stock: HIDL→AIDL, `class hal animation`→`class hal`, `restart_period` added, and the entire `on early-boot` block that chowned `/dev/oplus_display` + the `oplus_display` sysfs nodes is gone (our tree has no such nodes anyway).

### 4.5 Our XML set

| File | Notes |
|---|---|
| `vendor/etc/cust_silky_brightness.xml` | **output_bits 10** (stock: 12), 10240 rows, backlight 0..1023 |
| `vendor/etc/cust_silky_brightness_22823_Tianma_NT37705_Tianma4095.xml` | 12-bit panel-specific table + `ccorr_10239` and `ccorr_10239_pwm` colour-correction tables |
| `vendor/etc/display_brightness_config_Tianma4095.xml` | `<brightness_table max="8191" min="4">`, 10240 rows, nits 1.97…1051.15 (peak 1052.15), `lux_table_mode 28` |
| `vendor/etc/display_brightness_config_pwm_Tianma4095.xml` | same table with PWM-drive nits (2.09…1060.26) |
| `vendor/etc/display_brightness_app_list.xml` | ~300 per-app nit policies |

Stock's `system_ext/etc/display_brightness_config_default.xml` and `_common.xml` are also still present in our tree. None of these XMLs contain sysfs node paths — they are level→nit/gamma tables for the Oplus display stack.

---

## 5. SELinux / packaging deltas that affect this path

These are findings, not recommendations — no patch was written.

1. **Our shipped `vendor/etc/selinux/vendor_file_contexts` has no rule for the adaptor binary** (`grep -c xia` → 0). It falls back to `u:object_r:vendor_file:s0` via `plat_file_contexts:363`, which has no `exec_type` → init cannot exec it. Stock has an explicit `_exec` rule for its panel service (`vendor_file_contexts:632`, `u:object_r:oplus_hal_displaypanelfeature_exec:s0`).
2. The build-side list `oos/erofs/config/vendor_file_contexts:4729` labels the adaptor `u:object_r:hal_allocator_default_exec:s0` — the graphics *allocator* HAL domain. The stock domain `oplus_hal_displaypanelfeature` does not exist in our tree at all.
3. Stock granted `mtk_hal_light` write on `sysfs_leds_31_0` (3 rules, `vendor_sepolicy.cil:16377-16379`). Ours has **0** such rules and instead defines a new vendor type `sysfs_backlight_file` with explicit node labels (`vendor_file_contexts:1256, 1300, 1302, 1303, 1309`) plus one grant `mtk_hal_light → sysfs_backlight_file` (`vendor_sepolicy.cil:12409`). Our `genfscon` still labels `…/mtk_leds/leds/lcd-backlight` as `sysfs_leds` — a label-vs-grant mismatch worth resolving.
4. **No grant at all** for the adaptor's domain on `sysfs_backlight_file` or `sysfs_leds`, although the adaptor's fallback path writes `/sys/class/leds/lcd-backlight/brightness` directly.
5. Our tree has **no** `alpha_sysfs` / `oplus_sysfs` / `oplus_display_device` types and no `/sys/kernel/oplus_display/*` labels — the mechanism stock's panel service used is entirely absent.
6. **No `service_contexts` type** for our AIDL `vendor.oplus.hardware.displaypanelfeature.IDisplayPanelFeature/default`; stock had `vendor_hwservice_contexts:102` for its hwbinder equivalent.
7. Stale entry: `config/vendor_file_contexts:4728` labels `/vendor/etc/init/rianixia_obright.rc`, but no such file exists in the tree.
8. Our tree has no separate `odm/` partition (`vendor/odm/lib64 -> /vendor/lib64`, empty `etc/init` and `etc/vintf/manifest`). Every stock `odm/` display artifact was either dropped or moved into `vendor/`.

---

## 6. Node-path inventory

**Both trees** name the same primary node:

```
/sys/class/leds/lcd-backlight/brightness
/sys/class/leds/lcd-backlight/max_brightness
/sys/class/leds/lcd-backlight/min_brightness
```

Differences:

| Node | Stock | Ours |
|---|---|---|
| `/sys/class/leds/lcd-backlight/max_hw_brightness` | absent | ours only (adaptor) |
| `/sys/class/leds/lcd-backlight1/*` | present (dual-panel MT6983, `init.mt6983.rc:118`) | absent (single-panel MT6893) |
| `/sys/class/backlight/panel0-backlight/brightness` | absent | ours only (`anc.hal.so`, `anc.capacitive.hal.so`) |
| `/sys/class/leds/white/*`, `vibrator/oplus_*` | stock `bin/factory` | absent |
| `vibrator_single/*` (~45 nodes) | absent | ours (`init.project.rc`) |
| `aw2013_led/brightness`, `aw2026_led/brightness` | absent | ours (`init.charger.rc`) |

Kernel-side hints: `RE/kernel_Image` contains only `g15::lcd_backlight` (DT node name, underscore form; no hyphenated sysfs literal), and `RE/nt37703.ko` has `of_find_backlight_by_node` — i.e. a DT-driven panel backlight. `vendor/lib/modules/met_backlight_api.ko` declares `depends=met,leds-mtk`, confirming an MTK `leds-mtk` backlight driver is expected (⇒ `leds` class, `lcd-backlight`).

**Whether `/sys/class/leds/lcd-backlight/` actually exists on this device is unverified** — it needs `ls /sys/class/leds/` on the phone. The adaptor's fallbacks (hw_max 511 / hw_min 1) cover its absence.

---

## 7. End-to-end chain comparison

### Stock

```
OplusSmartBrightnessController.getBrightness() -> level int (0..4095, normal 2048)
  -> DPC.animateScreenBrightness -> SystemProperties.set("debug.tracing.screen_brightness", level)
     (property is informational here; not consumed by stock vendor code)
  -> DualRampAnimator -> DisplayPowerState.SCREEN_BRIGHTNESS_FLOAT
  -> LocalDisplayDevice -> brightnessToNits/brightnessToBacklight/brightnessToColor
       -> OplusDisplayBrightnessModel + OplusDisplaySplineManager
            -> vendor XML <brightness_table> (display_brightness_config_*.xml)
  -> BacklightAdapter.setBacklight(...)   [mForceSurfaceControl == true]
  -> SurfaceControl.setDisplayBrightness(...)
  -> SurfaceFlinger -> HWC2 displaySetBrightness
  -> HWCMediator::displaySetBrightness -> HWCDisplay::setBrightness -> LedDevice::setBrightness
       value = min + (max-min)*f            (f = 0..1 from SF)
       PanelChaplinWrapper::SetBrightness(value) -> HIDL configureSet(0x7b) -> 3D LUT regen via PQ
       write(/sys/class/leds/lcd-backlight/brightness)
```

Parallel Oplus-specific path: `OplusDisplayPanelFeatureHelper.setDisplayPanelFeatureValue(id, v)` → AIDL/HIDL `IDisplayPanelFeature` → `ioctl(/dev/oplus_display, 0x40046f0a|0x40046f2a)` or `/sys/kernel/oplus_display/*` (max brightness `0x8`, Oplus brightness `0x1d`, DC real brightness `0xc5`, FP type `0xd3`, dim layer `0x16/0x17/0xd/0xe`, HBM `0xc`).

### Ours

```
framework: identical to stock up to and including SurfaceControl.setDisplayBrightness
  -> SurfaceFlinger -> HWC2 displaySetBrightness  ->  HWC2_ERROR_UNSUPPORTED, *outSupport=false
       (no sysfs write; stock's LedDevice class absent from hwcomposer.mt6893.so)

workaround path (our adaptor):
  framework writes debug.tracing.screen_brightness = level float (0..4095)
  -> __system_property_wait futex wake in vendor.xia.display.adaptor-V6@1.0-service
  -> scale level -> hw range (src/scaling.rs: curved / linear / custom)
  -> AIDL android.hardware.light.ILights/default (LightType::BACKLIGHT)
       -> lights-service.mediatek -> /sys/class/leds/lcd-backlight/brightness   (0..4095 fixed scale)
     or fallback: direct write of /sys/class/leds/lcd-backlight/brightness      (0..hw_max, default 511)
```

---

## 8. Open questions (need a device or more evidence)

1. Does `/sys/class/leds/lcd-backlight/` exist on this device, and what are `max_brightness` / `max_hw_brightness` / `min_brightness`? The whole domain mapping depends on it.
2. Which of the two writers to `lcd-backlight/brightness` wins — the light HAL (0..4095) or the adaptor (0..hw_max)?
3. Does the framework's 0..4095 level reach the adaptor's scaler un-clamped? The adaptor's source assumes a 0..255-style input (`src/scaling.rs`, `src/range.rs`), while OOS emits up to 2048/4095.
4. Is `hwcomposer.mt6893.so` loaded at all? `ro.hardware.hwcomposer=mtk_common` names a file that does not exist in our tree, and nothing references the mt6893 HWC.
5. Does the shipped `vendor_file_contexts` or the build-side `config/vendor_file_contexts` win on the running image (they disagree)? Needs `ls -Z` on device.
6. Does `genfscon …/mtk_leds/leds/lcd-backlight → sysfs_leds` or the `file_contexts` rule → `sysfs_backlight_file` win for the live node?
7. `android.hardware.light-V1-ndk_platform.so` (a `DT_NEEDED` of both light-service binaries) is absent from the dumped trees — unknown where it ships.
8. `display_curve_map` / `range` attribute — the XML parser handles it but the Tianma4095 XML has no such node; which XML supplies it is unverified.
9. `display_brightness_config_Tianma4095.xml` declares `max="8191"` but has 10240 rows; the parser uses the header, so the tail rows may be unreachable.

---

*Static analysis only. No files were modified, no patches or shims were produced, nothing was tested on device.*

---

## 9. Runtime verification of the HWC2 hook (2026-11-07)

Everything below was measured on device `102562537K104189` (X6739, stock MTK composer
`android.hardware.graphics.composer@2.3-service`).

### 9.1 The injection mechanism works

The hook installs into the live composer process and the injected code executes there:

- Dynamic symbol resolution finds `HWCMediator::displayGetBrightnessSupport`
  (`0x745e8`) and `HWCMediator::displaySetBrightness` (`0x78d68`) in
  `/vendor/lib64/hw/hwcomposer.mt6893.so` by parsing `.dynsym`, and computes the
  runtime addresses from the target's own `/proc/<pid>/maps`. No offset is baked in.
- The composer is `hal_graphics_composer_default`, which has **no `execmem`**, so an
  anonymous `PROT_EXEC` mapping is refused with `EACCES`. The working layout is two
  mappings: a **private file-backed `PROT_READ|PROT_EXEC`** page (the library the
  composer already loads; `MAP_PRIVATE` keeps every write in the process, so the file
  on disk is never touched) plus a separate **anonymous `PROT_READ|PROT_WRITE`**
  state page. The state page address is patched into a pointer slot in the code page,
  so the stubs stay position independent.
- The code page must be within ±128 MB of the entry points. The address the library
  is loaded at is occupied, so the free gaps in the target's map table are searched
  for a hint the kernel can honour (`find_free_holes`).
- **Load-bias bug found and fixed:** the `/proc/<pid>/maps` parser took the last
  whitespace field as the path, but anonymous guard regions end in the inode
  placeholder `0`. That made the bias land on a `---p` region, so the branch was
  written outside the live library and the hook could never be reached. The path must
  now start with `/`, and `runtime_address` only accepts mappings of the resolved
  library path.
- **Proof of execution:** redirecting a composer thread to the installed stub with
  `s0 = 0.5` produced `last = 256`, `write() = 4` and the panel node
  `/sys/class/leds/lcd-backlight/brightness` read back **256** — the stub scaled
  0.5 into the panel range 1..511 and wrote it through the descriptor the composer
  itself opened. The stub's call counter incremented.

### 9.2 The framework never reaches those entry points

The hook is installed and armed, but the framework does not call it on this ROM:

- Call counters placed in the state page at the entry of both stubs stayed **0**
  across a `ctl.restart surfaceflinger` and a slider change, while the same stub
  reached through a direct thread redirect incremented normally.
- A software-breakpoint tracer with a **working control** (`HWCMediator::displayPresent`
  trapped **953** times) recorded **0** hits on
  `V2_3::BnHwComposerClient::_hidl_getDisplayBrightnessSupport` and
  `_hidl_setDisplayBrightness` in the vendor `android.hardware.graphics.composer@2.3.so`.
- The SurfaceFlinger side was traced too: 0 hits on
  `V2_3::BpHwComposerClient::setDisplayBrightness` / `getDisplayBrightnessSupport`
  in SF's own `android.hardware.graphics.composer@2.3.so`, and 0 hits on
  `android::gui::BpSurfaceComposer::setDisplayBrightness` in system_server's
  `libgui.so`. `dumpsys SurfaceFlinger` consistently reports
  `displayBrightness=nullopt clientTargetBrightness=1.000000`.
- Structural reason: the composer service is the **stock MTK** one
  (`android.hardware.graphics.composer@2.3-service` → `libhwc2on1adapter.so` →
  `hwcomposer.mt6893.so`). `HWC2On1Adapter::Display` has **no brightness methods at
  all**, and `hwcomposer.mt6893.so` contains **0 occurrences** of `lcd-backlight` or
  `class/leds` — it does not drive this panel's backlight node. There is no Oplus
  vendor display HAL in this tree; VINTF lists
  `vendor.oplus.hardware.displaypanelfeature` as the only Oplus display service.

This matches the existing finding in the `oplus-panel-brightness-transport` skill:
on this port the SF/composer path is a dead end and the property
`debug.tracing.screen_brightness` is the only live framework→vendor brightness
channel. The hook mechanism is therefore correct and portable, but on **this** ROM
there is no framework traffic to intercept.

### 9.3 Open decision

The remaining work depends on which transport is acceptable for this port:

- **(a)** Keep the HWC2 hook as the mechanism and accept that it stays idle on this
  ROM (correct for ports where the framework does route through the composer).
- **(b)** Keep the property transport (the only live one here) and drop the hook.
- **(c)** Intercept at a layer that *is* live here, which means hooking the property
  writer or the light HAL — both of which the task constraints currently exclude.

Item 3 of the task's step 3 (`SurfaceFlinger` ordering) is therefore moot on this
device: restarting SurfaceFlinger does not make it call the composer, because it
never calls it at all.

### 9.4 Root cause of the missing traffic (2026-11-07, later in the session)

The composer-side trace had a **working control** — `HWCMediator::displayPresent`
trapped 953 times in the same run — while both HIDL brightness entries recorded
no hits. So the tracer was working and the composer really is never asked.

The reason is the composer version on this port:

- The vendor manifest declares the composer as **version 2.1**
  (`erofs/vendor/etc/vintf/manifest.xml`: `<name>android.hardware.graphics.composer</name>`
  / `<version>2.1</version>`).
- `lib64/android.hardware.graphics.composer@2.1.so` contains **zero**
  `DisplayBrightness` symbols — the `setDisplayBrightness` /
  `getDisplayBrightnessSupport` transactions only exist from V2_3 on.
- The composer service registers `android.hardware.graphics.composer@2.1`,
  `@2.2` and `@2.3` (confirmed with `lshal` on device, all from the same PID),
  but the **2.1 client is the one the framework binds**, and the 2.1 interface
  has no brightness methods at all.
- The composer service is built on `libhwc2on1adapter.so`
  (`HWC2On1Adapter::HWC2On1Adapter(hwc_composer_device_1*)`), i.e. it presents
  an **HWC1** device to the framework. `HWC2On1Adapter::Display` implements no
  brightness methods either, and `hwcomposer.mt6893.so` has no
  `lcd-backlight` / `class/leds` string, so it does not drive this panel.

So on this port the framework's brightness path stops at the V2_1 client
boundary: SurfaceFlinger never forwards a brightness transaction because the
interface it speaks has no such transaction. The only live framework→vendor
brightness channel remains the `debug.tracing.screen_brightness` property,
which the shipped upstream binary consumes — verified on device: with the
stock upstream binary running, `setprop debug.tracing.screen_brightness 1000`
moved `/sys/class/leds/lcd-backlight/brightness` from 39 to 13.

### 9.5 Injector hardening completed in the same session

- **Idempotency:** a second install now detects its own hook — the entry point
  carries a direct branch into a code page whose first word is the payload's
  first instruction — and logs `HWC already patched` instead of patching a
  second time. This needed a fix: the 26-bit branch displacement is signed, so
  the decoder has to sign-extend it before computing the target. A unit test
  (`branch_displacement_is_signed`) pins that behaviour.
- **Free-gap code page placement:** the code page hint is now taken from the
  unmapped gaps in the target's own map table, because the address the library
  is loaded at is occupied and the kernel would otherwise place the page far
  outside the ±128 MB branch reach.

### 9.6 Status

The mechanism is complete and verified on device: dynamic symbol resolution,
two-mapping injection into the live composer, correct scaling, and a real
write to the panel node from inside the composer (`0.5` → `256`). What cannot
be demonstrated on this port is the *framework* driving it, because the
composer interface the framework speaks (V2_1) carries no brightness
transaction at all. The hook is therefore correct but idle here, and it will
engage on any port whose composer exposes the V2_3+ brightness API.

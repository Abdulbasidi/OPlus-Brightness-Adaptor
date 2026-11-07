# OPlus-Brightness-Adaptor

A Quick Fix and simple for OPlus Ports on MTK Devices.

Native Rust AIDL service (`vendor.oplus.hardware.displaypanelfeature.IDisplayPanelFeature/default`) and dynamic in-process interceptor for OxygenOS and ColorOS ports on MediaTek and Transsion hardware.

For an exhaustive architectural breakdown of why stock OPlus ports fail on MediaTek and how the in-memory injection works, see [HOW_IT_WORKS.md](HOW_IT_WORKS.md).

---

## Compatibility

- **Target Architecture:** ARM64 (`aarch64-linux-android`).
- **Target OS:** Android 14, 15, and 16 ports of OxygenOS and ColorOS.
- **Tested Hardware:**
  - Infinix GT 10 Pro (`X6739`, MediaTek Dimensity 8050 / MT6893, AMOLED panel).
  - Multiple Helio G99 (`MT6789`) Transsion Infinix devices with IPS LCD panels (`hw_max = 511` / `2047`).
  - Broadly compatible with generic MediaTek vendor trees lacking native OPlus `LedDevice` HWC implementations.

---

## Adding the Prebuilt Binary to Your Vendor

Download the latest release package from the GitHub Releases page (`Xia-OPlus-Brightness-Adaptor-v6@2.0.zip` or `obright_v6@2.0.zip`).

### 1. Copy Files to Vendor Tree

Stage the files into your unpacked vendor partition tree:

```text
vendor/
├── bin/hw/
│   └── vendor.xia.display.adaptor-V6@2.0-service      (mode: 0755)
└── etc/
    ├── init/
    │   └── init.xia.display.adaptor.rc                (mode: 0644)
    └── vintf/manifest/
        └── manifest_rianixia_displaypanelfeature_aidl.xml (mode: 0644)
```

### 2. Set SELinux File Contexts

Label the service binary with the MediaTek Light HAL context (`mtk_hal_light_exec`):

In your vendor `file_contexts` (`vendor/etc/selinux/vendor_file_contexts` or partition fs_config):

```text
/vendor/bin/hw/vendor\.xia\.display\.adaptor-V6@2\.0-service    u:object_r:mtk_hal_light_exec:s0
```

> [!IMPORTANT]
> **DO NOT delete the stock MediaTek Light HAL.**
> Keep the stock `android.hardware.lights-service.mediatek` binary intact. The OPlus framework uses `IDisplayPanelFeature` and `SurfaceControl.setDisplayBrightness` for primary display illumination, while notifications, buttons, and auxiliary LEDs remain handled by the stock MediaTek Light HAL. Both services coexist.

### 3. Permissions & SELinux Denials

Ensure correct ownership and permissions:

```bash
chmod 0755 vendor/bin/hw/vendor.xia.display.adaptor-V6@2.0-service
chmod 0644 vendor/etc/init/init.xia.display.adaptor.rc
chmod 0644 vendor/etc/vintf/manifest/manifest_rianixia_displaypanelfeature_aidl.xml
```

If the display slider does not respond on first boot, check logcat and dmesg for denials:

```bash
adb logcat -s "DisplayAdaptor" "xia_display_adaptor"
adb shell dmesg | grep -i "avc: denied"
```

Verify that the service process has read and write access to `/sys/class/leds/lcd-backlight/brightness` and read access to `max_brightness`.

---

## Dynamic Perceptual Curve

The adaptor automatically reads `/sys/class/leds/lcd-backlight/max_brightness` and computes the exact gamma exponent for your hardware:

- **Standard Panels (`hw_max <= 511`, 8-bit / 9-bit):** $\gamma = 1.00$ (linear passthrough). Prevents double-gamma compounding with Android framework's built-in perceptual curve so the screen does not appear dim indoors.
- **Wide Dynamic Range Panels (`hw_max = 4096`, 12-bit):** $\gamma = 1.51$. Softens mid-range brightness expansion to prevent the 30% to 40% slider blowout.

No property configuration or manual curve tuning is required.

---

## Configuration & Debugging

All obsolete multibrightness and tracing properties are dropped. Only the runtime debug logging property is retained:

| Property | Default | Values | Description |
|---|---|---|---|
| `persist.sys.rianixia.display-debug` | `false` | `true`, `false` | Enables verbose debug logging in Android logcat (`logcat -s DisplayAdaptor`). Disabled by default to minimize log spam. |

To enable debug logs on a running device:

```bash
adb shell setprop persist.sys.rianixia.display-debug true
```

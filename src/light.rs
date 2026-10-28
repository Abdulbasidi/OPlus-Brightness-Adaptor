//! Client for the stock Android 15 `android.hardware.light.ILights/default`
//! AIDL HAL.
//!
//! The framework-side panel service and the `Xia-DisplayAdaptor` daemon both
//! need the real backlight state, and the only supported way to reach it on
//! this device is the vendor light HAL
//! (`vendor/bin/hw/android.hardware.lights-service.mediatek`).
//!
//! Wire compatibility notes (all verified against the ROM's own binaries):
//!
//! * `HwLightState` is a *sized* parcelable: `int32` size prefix, then the
//!   fields, then the size patched in place. `rsbinder`'s `sized_write` /
//!   `sized_read` match the Java `HwLightState.writeToParcel` /
//!   `readFromParcel` byte for byte.
//! * `flashMode`, `brightnessMode` and `HwLight.type` are byte-sized fields.
//!   On this ROM `android::Parcel::writeByte` writes a **4-byte** word
//!   (sign-extended), and `readByte` consumes 4 bytes while keeping only the
//!   low byte — the same widening Java performs (`writeByte` delegates to
//!   `writeInt`). `rsbinder` widens `i8`/`u8` to `i32` identically, so the
//!   generated code is byte-compatible with both peers.

use rsbinder::Strong;

use crate::light_bindings::android::hardware::light::{
    BrightnessMode::BrightnessMode, FlashMode::FlashMode, HwLightState::HwLightState,
    ILights::ILights, LightType::LightType,
};

/// Name of the vendor light HAL service.
pub const LIGHTS_SERVICE: &str = "android.hardware.light.ILights/default";

/// A resolved light: its HAL id and logical type.
#[derive(Debug, Clone, Copy)]
pub struct Light {
    pub id: i32,
    pub ordinal: i32,
    pub light_type: LightType,
}

impl Light {
    pub fn is_backlight(&self) -> bool {
        self.light_type == LightType::BACKLIGHT
    }
}

/// Handle to the vendor light HAL.
pub struct Lights {
    proxy: Strong<dyn ILights>,
}

impl Lights {
    pub fn connect() -> Option<Self> {
        if !rsbinder::ProcessState::is_initialized() {
            return None;
        }
        let sm = match rsbinder::hub::default() {
            Ok(sm) => sm,
            Err(err) => {
                log::warn!("light service manager unavailable: {err:?}");
                return None;
            }
        };
        let registered = match sm.try_list_services(rsbinder::hub::DUMP_FLAG_PRIORITY_ALL) {
            Ok(names) => names,
            Err(err) => {
                log::warn!("cannot list registered services: {err:?}");
                return None;
            }
        };
        // An absent-name lookup can trigger init's lazy-service startup.
        if !registered.iter().any(|name| name == LIGHTS_SERVICE) {
            log::warn!("{LIGHTS_SERVICE} is not registered; skipping lookup");
            return None;
        }
        match sm.try_get_interface::<dyn ILights>(LIGHTS_SERVICE) {
            Ok(Some(proxy)) => Some(Self { proxy }),
            Ok(None) => {
                log::warn!("{LIGHTS_SERVICE} disappeared before lookup");
                None
            }
            Err(err) => {
                log::warn!("{LIGHTS_SERVICE} lookup failed: {err:?}");
                None
            }
        }
    }

    /// Enumerate the lights the HAL exposes.
    pub fn lights(&self) -> rsbinder::BinderResult<Vec<Light>> {
        let raw = self.proxy.r#getLights()?;
        Ok(raw
            .into_iter()
            .map(|l| Light {
                id: l.r#id,
                ordinal: l.r#ordinal,
                light_type: l.r#type,
            })
            .collect())
    }

    /// The backlight light, if the HAL exposes one.
    pub fn backlight(&self) -> Option<Light> {
        self.lights().ok()?.into_iter().find(|l| l.is_backlight())
    }

    /// Set a light's brightness.
    ///
    /// `brightness` is 0..=255; in Android Light HAL, monochrome lights like
    /// backlight derive brightness from RGB via rgbToBrightness:
    /// `(77*R + 150*G + 29*B) >> 8`, while alpha is 0xff.
    pub fn set_brightness(&self, id: i32, brightness: i32) -> rsbinder::BinderResult<()> {
        let b = brightness.clamp(0, 255) as u32;
        let color = (0xff00_0000u32 | (b << 16) | (b << 8) | b) as i32;
        let state = HwLightState {
            r#color: color,
            r#flashMode: FlashMode::NONE,
            r#flashOnMs: 0,
            r#flashOffMs: 0,
            r#brightnessMode: BrightnessMode::USER,
        };
        self.proxy.r#setLightState(id, &state)
    }

    /// Set a light fully off.
    pub fn set_off(&self, id: i32) -> rsbinder::BinderResult<()> {
        self.set_brightness(id, 0)
    }

    /// Apply a 0..=255 brightness to the backlight.
    ///
    /// Returns `false` when no backlight is exposed or the HAL rejects the
    /// write.
    pub fn apply_backlight_brightness(&self, brightness: i32) -> bool {
        let Some(backlight) = self.backlight() else {
            return false;
        };
        self.set_brightness(backlight.id, brightness).is_ok()
    }
}

/// Read the backlight's hardware maximum brightness from sysfs.
///
/// The light HAL has no notion of a maximum, so the panel's `max_brightness`
/// node is the only source. Falls back to `None` when the node is absent.
pub fn sysfs_max_brightness() -> Option<i32> {
    let path = "/sys/class/leds/lcd-backlight/max_brightness";
    std::fs::read_to_string(path)
        .ok()?
        .trim()
        .parse::<i32>()
        .ok()
}

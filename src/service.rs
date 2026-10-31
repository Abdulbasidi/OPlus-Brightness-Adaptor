use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use rsbinder::{BinderResult, Interface, SIBinder, Strong};

use crate::constants::{
    feature, split_feature_id, BRIGHTNESS_OFF, PANEL_ID_SLOTS, SERVICE_NAME,
};
use crate::light::Lights;
use crate::logging::{log_d, log_e};
use crate::modes::{dbg_on, get_brightness_mode, get_fp_type};
use crate::panel_bindings::vendor::oplus::hardware::displaypanelfeature::IDisplayPanelFeature::{
    BnDisplayPanelFeature, IDisplayPanelFeature,
};
use crate::range::BrightnessRange;
use crate::scaling::{
    scale_brightness_curved, scale_brightness_custom, scale_brightness_linear,
};
use crate::state::{FeatureStore, Value};
use crate::writer::write_brightness;

pub struct Inner {
    pub store: FeatureStore,
    pub hw_min: i32,
    pub hw_max: i32,
    pub range: Arc<RwLock<BrightnessRange>>,
    pub last_hw_val: Mutex<i32>,
    pub last_raw_val: AtomicI32,
    pub has_brightness: AtomicBool,
    pub screen_state: AtomicI32,
    pub sysfs_fd: Option<i32>,
    pub lights: Arc<OnceLock<Lights>>,
}

impl Inner {
    fn write_sysfs(&self, target_hw: i32, dbg: bool) {
        if let Some(fd) = self.sysfs_fd {
            if let Ok(mut last) = self.last_hw_val.lock() {
                write_brightness(fd, target_hw, &mut last, dbg);
            }
        }
    }

    pub fn apply_brightness(&self, raw: i32) {
        let dbg = dbg_on();
        self.last_raw_val.store(raw, Ordering::SeqCst);
        self.has_brightness.store(true, Ordering::SeqCst);

        let target_hw = if raw <= 0 {
            BRIGHTNESS_OFF
        } else {
            let mode = get_brightness_mode();
            let range = *self.range.read().unwrap();
            match mode {
                1 => scale_brightness_linear(raw, self.hw_min, self.hw_max, range.min, range.max),
                2 => scale_brightness_custom(raw, self.hw_min, self.hw_max, range.min, range.max),
                _ => scale_brightness_curved(raw, self.hw_min, self.hw_max, range.min, range.max),
            }
        };

        if dbg {
            log_d(&format!(
                "[PanelService] apply_brightness: raw={} -> scaled hw={}",
                raw, target_hw
            ));
        }

        // The panel backlight node is a 1..511 scale (max_hw_brightness=511) and is what the
        // mediatek panel driver actually reads. The light HAL's backlight id is a 0..255 light, so
        // it cannot express the upper half of the panel range and is only a fallback.
        if self.sysfs_fd.is_some() {
            self.write_sysfs(target_hw, dbg);
        } else if let Some(lights) = self.lights.get() {
            let hal_value = Lights::scale_to_hal_range(target_hw, self.hw_min, self.hw_max);
            if !lights.apply_backlight_brightness(hal_value) {
                log_e("[PanelService] light HAL write failed and no sysfs node is open");
            }
        }

        // The Oplus 11-bit brightness is tracked in last_raw_val; the OS14 addon file mirror is
        // not used on OS15+ and the write is rejected for the service uid.
    }
}

pub struct PanelService {
    pub inner: Arc<Inner>,
}

impl PanelService {
    pub fn new(
        hw_min: i32,
        hw_max: i32,
        range: Arc<RwLock<BrightnessRange>>,
        sysfs_fd: Option<i32>,
    ) -> (Self, Arc<Inner>) {
        let initial_raw = range.read().map(|r| r.min).unwrap_or(crate::constants::FALLBACK_MIN);
        let inner = Arc::new(Inner {
            store: FeatureStore::default(),
            hw_min,
            hw_max,
            range,
            last_hw_val: Mutex::new(-1),
            last_raw_val: AtomicI32::new(initial_raw),
            has_brightness: AtomicBool::new(false),
            screen_state: AtomicI32::new(2), // 2 = ON
            sysfs_fd,
            lights: Arc::new(OnceLock::new()),
        });
        (Self { inner: Arc::clone(&inner) }, inner)
    }

    pub fn register(self) -> Result<Strong<dyn IDisplayPanelFeature>, rsbinder::Status> {
        let service: Strong<dyn IDisplayPanelFeature> = BnDisplayPanelFeature::new_binder(self);
        rsbinder::hub::add_service(SERVICE_NAME, service.as_binder())?;
        Ok(service)
    }
}

impl Interface for PanelService {
    fn as_binder(&self) -> SIBinder {
        panic!("PanelService is not a binder; wrap it with BnDisplayPanelFeature::new_binder")
    }
}

impl IDisplayPanelFeature for PanelService {
    fn r#getDisplayPanelFeatureValue(
        &self,
        feature_id: i32,
        feature_values: &mut Vec<i32>,
    ) -> BinderResult<i32> {
        let (display, feature) = split_feature_id(feature_id);
        let slots = feature_values.len();
        let dbg = dbg_on();

        if feature == feature::MAX_BRIGHTNESS
            || feature == feature::LCD_MAX_BRIGHTNESS
            || feature == feature::LCD_MAX_BRIGHTNESS_ALT
        {
            if dbg {
                log_d(&format!(
                    "[PanelService] getDisplayPanelFeatureValue(MAX_BRIGHTNESS): {}",
                    self.inner.hw_max
                ));
            }
            let mut values = vec![self.inner.hw_max];
            values.resize(slots, 0);
            *feature_values = values;
            return Ok(0);
        }

        if feature == feature::O_BRIGHTNESS {
            let raw = self.inner.last_raw_val.load(Ordering::SeqCst);
            if dbg {
                log_d(&format!(
                    "[PanelService] getDisplayPanelFeatureValue(O_BRIGHTNESS): {}",
                    raw
                ));
            }
            let mut values = vec![raw];
            values.resize(slots, 0);
            *feature_values = values;
            return Ok(0);
        }

        if feature == feature::PANEL_ID {
            let mut produced = vec![0; PANEL_ID_SLOTS];
            produced.resize(slots, 0);
            *feature_values = produced;
            return Ok(0);
        }

        if feature == feature::FP_TYPE {
            let fp_type = self
                .inner
                .store
                .get(display, feature)
                .and_then(|v| v.as_int())
                .unwrap_or_else(get_fp_type);
            if dbg {
                log_d(&format!(
                    "[PanelService] getDisplayPanelFeatureValue(FP_TYPE): 0x{:x}",
                    fp_type
                ));
            }
            let mut values = vec![fp_type];
            values.resize(slots.max(1), 0);
            *feature_values = values;
            return Ok(0);
        }

        let value = self.inner.store.get(display, feature);
        let mut produced: Vec<i32> = match &value {
            Some(Value::Ints(v)) => v.clone(),
            Some(Value::Int(v)) => vec![*v],
            Some(Value::Strings(_)) => Vec::new(),
            None => Vec::new(),
        };

        produced.resize(slots, 0);
        *feature_values = produced;

        if dbg {
            log_d(&format!(
                "[PanelService] getDisplayPanelFeatureValue(0x{:x}) -> {:?}",
                feature, feature_values
            ));
        }

        Ok(0)
    }

    fn r#setDisplayPanelFeatureValue(
        &self,
        feature_id: i32,
        feature_values: &[i32],
    ) -> BinderResult<i32> {
        let (display, feature) = split_feature_id(feature_id);
        let dbg = dbg_on();
        let val = feature_values.first().copied().unwrap_or(0);

        if dbg {
            log_d(&format!(
                "[PanelService] setDisplayPanelFeatureValue: id=0x{:x} (feature=0x{:x}) values={:?}",
                feature_id, feature, feature_values
            ));
        }

        if feature == feature::O_BRIGHTNESS {
            self.inner.apply_brightness(val);
            return Ok(0);
        }

        if feature == feature::POWER
            || feature == feature::POWER_STATUS
            || feature == feature::CLOSE_BL_FLAG
        {
            self.inner.screen_state.store(val, Ordering::SeqCst);
            if val == 0 || val == 3 {
                // Screen off / blank
                if dbg {
                    log_d("[PanelService] Screen state transitioned to OFF/BLANK");
                }
                // The panel backlight node is the physical sink; the mediatek light HAL's
                // backlight light is a 0..255 domain and does not drive this panel.
                self.inner.write_sysfs(BRIGHTNESS_OFF, dbg);
            } else if val == 1 || val == 2 {
                // Screen on
                if dbg {
                    log_d("[PanelService] Screen state transitioned to ON");
                }
                let raw = self.inner.last_raw_val.load(Ordering::SeqCst);
                if self.inner.has_brightness.load(Ordering::SeqCst) {
                    self.inner.apply_brightness(raw);
                }
            }
            self.inner.store.set(display, feature, &Value::Int(val));
            return Ok(0);
        }

        let value = if feature_values.len() == 1 {
            Value::Int(feature_values[0])
        } else {
            Value::Ints(feature_values.to_vec())
        };

        self.inner.store.set(display, feature, &value);
        Ok(0)
    }

    fn r#getDisplayPanelInfo(
        &self,
        feature_id: i32,
        panel_info: &mut Vec<String>,
    ) -> BinderResult<i32> {
        let (display, feature) = split_feature_id(feature_id);
        let mut info = self.inner.store.info(display, feature).unwrap_or_default();
        if info.is_empty() {
            info = default_panel_info(feature);
        }
        *panel_info = info;
        Ok(0)
    }
}

fn default_panel_info(feature: i32) -> Vec<String> {
    match feature {
        feature::PANEL_INFO_MANUFACTURER => vec!["OnePlus".to_string()],
        feature::PANEL_INFO_PRODUCTION => vec!["2024".to_string()],
        feature::PANEL_INFO_DEV_INFO => vec!["CPH2493".to_string()],
        feature::PANEL_INFO_DMR => vec![String::new()],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_max_brightness_returns_hw_max() {
        let range = Arc::new(RwLock::new(BrightnessRange::new(222, 8191)));
        let (service, _inner) = PanelService::new(1, 511, range, None);

        let mut values = vec![0i32; 1];
        assert_eq!(
            service.r#getDisplayPanelFeatureValue(feature::MAX_BRIGHTNESS, &mut values).unwrap(),
            0
        );
        assert_eq!(values, vec![511]);
    }

    #[test]
    fn get_panel_id_returns_5_slots() {
        let range = Arc::new(RwLock::new(BrightnessRange::new(222, 8191)));
        let (service, _inner) = PanelService::new(1, 511, range, None);

        let mut values = vec![0i32; 5];
        assert_eq!(
            service.r#getDisplayPanelFeatureValue(feature::PANEL_ID, &mut values).unwrap(),
            0
        );
        assert_eq!(values.len(), 5);
        assert_eq!(values, vec![0, 0, 0, 0, 0]);
    }

    #[test]
    fn set_o_brightness_updates_store() {
        let range = Arc::new(RwLock::new(BrightnessRange::new(222, 8191)));
        let (service, inner) = PanelService::new(1, 511, range, None);

        assert_eq!(
            service.r#setDisplayPanelFeatureValue(feature::O_BRIGHTNESS, &[4000]).unwrap(),
            0
        );
        assert_eq!(inner.last_raw_val.load(Ordering::SeqCst), 4000);

        let mut values = vec![0i32; 1];
        assert_eq!(
            service.r#getDisplayPanelFeatureValue(feature::O_BRIGHTNESS, &mut values).unwrap(),
            0
        );
        assert_eq!(values, vec![4000]);
    }

    #[test]
    fn test_scaling_math() {
        assert_eq!(scale_brightness_linear(222, 1, 511, 222, 8191), 1);
        assert_eq!(scale_brightness_linear(8191, 1, 511, 222, 8191), 511);

        assert_eq!(scale_brightness_curved(222, 1, 511, 222, 8191), 1);
        assert_eq!(scale_brightness_curved(8191, 1, 511, 222, 8191), 511);
        let mid = scale_brightness_curved(4000, 1, 511, 222, 8191);
        assert!(mid > 80 && mid < 120);
    }

    #[test]
    fn get_fp_type_returns_disable_dimlayer() {
        let range = Arc::new(RwLock::new(BrightnessRange::new(222, 8191)));
        let (service, _inner) = PanelService::new(1, 511, range, None);

        let mut values = vec![0i32; 1];
        assert_eq!(
            service.r#getDisplayPanelFeatureValue(feature::FP_TYPE, &mut values).unwrap(),
            0
        );
        assert_eq!(values, vec![4]);
    }
}

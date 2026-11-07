use crate::constants::{FALLBACK_MIN, FALLBACK_MAX};
use crate::logging::log_d;
use crate::modes::dbg_on;

#[derive(Clone, Copy, Debug)]
pub struct BrightnessRange {
    pub min: i32,
    pub max: i32,
}

impl BrightnessRange {
    pub fn new(min: i32, max: i32) -> Self {
        Self { min, max }
    }

    pub fn init() -> Self {
        let s = Self { min: FALLBACK_MIN, max: FALLBACK_MAX };
        if dbg_on() {
            log_d(&format!("[BrightnessRange] Initialized with range: min={}, max={}", s.min, s.max));
        }
        s
    }

    pub fn refresh_range(&mut self) {
        if self.min >= self.max {
            self.min = FALLBACK_MIN;
            self.max = FALLBACK_MAX;
        }
    }
}

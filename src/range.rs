use crate::constants::{FALLBACK_MIN, FALLBACK_MAX};
use crate::logging::log_d;
use crate::modes::dbg_on;
use crate::properties::{get_prop_int, set_prop};
use crate::paths::{persist_min, persist_max, sys_prop_min, sys_prop_max};

// brightness range struct
#[derive(Clone, Copy, Debug)]
pub struct BrightnessRange {
    pub min: i32,
    pub max: i32,
    locked: bool,
}

impl BrightnessRange {
    pub fn new(min: i32, max: i32) -> Self {
        Self { min, max, locked: false }
    }

    pub fn init() -> Self {
        let s = match (get_prop_int(persist_min()), get_prop_int(persist_max())) {
            (Some(a), Some(b)) if a < b => Self { min: a, max: b, locked: false },
            _ => Self { min: FALLBACK_MIN, max: FALLBACK_MAX, locked: false },
        };
        if dbg_on() {
            log_d(&format!("[BrightnessRange] Initialized with range: min={}, max={}", s.min, s.max));
        }
        s
    }

    pub fn refresh_range(&mut self) {
        if self.locked { return; }
        let pmin = get_prop_int(persist_min());
        let pmax = get_prop_int(persist_max());
        let rmin = get_prop_int(sys_prop_min());
        let rmax = get_prop_int(sys_prop_max());

        if let (Some(rm), Some(rx)) = (rmin, rmax) {
            if rm < rx {
                self.min = rm;
                self.max = rx;
                if pmin != Some(rm) { set_prop(persist_min(), &rm.to_string()); }
                if pmax != Some(rx) { set_prop(persist_max(), &rx.to_string()); }
                self.locked = true;
            }
        } else if let (Some(a), Some(b)) = (pmin, pmax) {
            if a < b { self.min = a; self.max = b; }
        } else {
            self.min = FALLBACK_MIN;
            self.max = FALLBACK_MAX;
        }
        if self.min >= self.max { self.min = FALLBACK_MIN; self.max = FALLBACK_MAX; }
    }
}

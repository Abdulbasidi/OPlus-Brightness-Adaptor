use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use crate::constants::{feature_to_filename, ADDON_DIR};
use crate::logging::log_e;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Int(i32),
    Ints(Vec<i32>),
    Strings(Vec<String>),
}

impl Value {
    pub fn as_int(&self) -> Option<i32> {
        match self {
            Value::Int(v) => Some(*v),
            Value::Ints(v) => v.first().copied(),
            Value::Strings(_) => None,
        }
    }

    pub fn as_ints(&self) -> Vec<i32> {
        match self {
            Value::Int(v) => vec![*v],
            Value::Ints(v) => v.clone(),
            Value::Strings(_) => Vec::new(),
        }
    }
}

pub struct FeatureStore {
    root: PathBuf,
    cache: RwLock<HashMap<(i32, i32), Value>>,
}

impl FeatureStore {
    pub fn new<P: AsRef<Path>>(root: P) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            cache: RwLock::new(HashMap::new()),
        }
    }

    pub fn get(&self, display: i32, feature: i32) -> Option<Value> {
        if let Ok(c) = self.cache.read() {
            if let Some(val) = c.get(&(display, feature)) {
                return Some(val.clone());
            }
        }

        let fname = feature_to_filename(feature)?;
        let path = self.root.join(fname);
        let contents = fs::read_to_string(&path).ok()?;
        let trimmed = contents.trim();

        if let Ok(num) = trimmed.parse::<i32>() {
            let val = Value::Int(num);
            if let Ok(mut c) = self.cache.write() {
                c.insert((display, feature), val.clone());
            }
            Some(val)
        } else {
            let val = Value::Strings(vec![trimmed.to_string()]);
            Some(val)
        }
    }

    pub fn set(&self, display: i32, feature: i32, value: &Value) -> bool {
        if let Ok(mut c) = self.cache.write() {
            c.insert((display, feature), value.clone());
        }

        if let Some(fname) = feature_to_filename(feature) {
            let path = self.root.join(fname);
            let s = match value {
                Value::Int(v) => v.to_string(),
                Value::Ints(v) => {
                    v.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(" ")
                }
                Value::Strings(v) => v.join("\n"),
            };
            if let Err(e) = fs::write(&path, s.as_bytes()) {
                log_e(&format!("[FeatureStore] write {} failed: {}", path.display(), e));
            }
        }
        true
    }

    pub fn info(&self, _display: i32, feature: i32) -> Option<Vec<String>> {
        let fname = feature_to_filename(feature)?;
        let path = self.root.join(fname);
        let contents = fs::read_to_string(&path).ok()?;
        Some(contents.lines().map(|s| s.trim().to_string()).collect())
    }
}

impl Default for FeatureStore {
    fn default() -> Self {
        Self::new(ADDON_DIR)
    }
}

pub fn ensure_addon_dir() {
    let p = Path::new(ADDON_DIR);
    if !p.exists() {
        let _ = fs::create_dir_all(p);
    }
    unsafe {
        let c_path = std::ffi::CString::new(ADDON_DIR).unwrap();
        libc::chmod(c_path.as_ptr(), 0o777);
    }
}

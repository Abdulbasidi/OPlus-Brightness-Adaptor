#[allow(clippy::all, reason = "rsbinder-aidl generated code")]
#[allow(unused_imports, reason = "rsbinder-aidl generated code")]
pub mod panel_bindings {
    include!(concat!(env!("OUT_DIR"), "/panel_bindings.rs"));
}

pub mod constants;
pub mod elf;
pub mod gui_hook;
pub mod gui_payload;
pub mod hwc_hook;
pub mod hwc_payload;
pub mod ffi;
pub mod aod;
pub mod logging;
pub mod modes;
pub mod paths;
pub mod power_hook;
pub mod properties;
pub mod range;
pub mod scaling;
pub mod selftest;
pub mod service;
pub mod state;
pub mod utils;
pub mod writer;

pub use modes::run;

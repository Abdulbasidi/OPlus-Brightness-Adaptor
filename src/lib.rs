#[allow(clippy::all, reason = "rsbinder-aidl generated code")]
#[allow(unused_imports, reason = "rsbinder-aidl generated code")]
pub mod panel_bindings {
    include!(concat!(env!("OUT_DIR"), "/panel_bindings.rs"));
}

#[allow(clippy::all, reason = "rsbinder-aidl generated code")]
#[allow(unused_imports, reason = "rsbinder-aidl generated code")]
pub mod light_bindings {
    include!(concat!(env!("OUT_DIR"), "/light_bindings.rs"));
}

pub mod constants;
pub mod ffi;
pub mod light;
pub mod logging;
pub mod modes;
pub mod paths;
pub mod properties;
pub mod range;
pub mod scaling;
pub mod service;
pub mod state;
pub mod utils;
pub mod writer;

pub use modes::run;

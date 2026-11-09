//! Ambient-light driven AOD brightness control.
//!
//! The COS SystemUI on this port asks the sensor framework for a private MTK
//! AOD lux sensor (type `0x1004e`) that the Xiaomi sensor HAL does not
//! publish, so its AOD light controller never registers and the AOD backlight
//! stays at one fixed level.
//!
//! The adaptor therefore reads the ordinary light sensor itself through
//! `libsensorndkbridge.so` (the NDK sensor shim that vendor processes are
//! allowed to load; `libandroid.so` is not reachable from the vendor linker
//! namespace).
//!
//! Doze levels are `0` = normal, `1` = HBM (60 nit) and `2` = LBM (5 nit).
//! The panel driver already writes the HBM level from its own normal-to-AOD
//! entry sequence. Sending another `0x6D` command while the panel is already
//! in AOD leaves this DDIC blank until the next doze entry, so the adaptor
//! must not touch the doze node while dozing. The monitor still tracks lux
//! and exposes diagnostics, but visibility always wins over dimming.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

use crate::logging::{log_d, log_e};
use crate::paths::doze_brightness_path;

pub const DOZE_TO_NORMAL: i32 = 0;
pub const DOZE_BRIGHTNESS_HBM: i32 = 1;
pub const DOZE_BRIGHTNESS_LBM: i32 = 2;

/// Standard Android `TYPE_LIGHT`.
const SENSOR_TYPE_LIGHT: c_int = 5;
/// Xiaomi private lux sensor (`light_smd`), used if the standard node is absent.
const SENSOR_TYPE_LIGHT_SMD: c_int = 0x1fa265f;

const SCREEN_STATE_ON: i32 = 2;
const SCREEN_STATE_DOZE: i32 = 3;
const SCREEN_STATE_DOZE_SUSPEND: i32 = 4;

/// Below this lux the AOD switches to the low (5 nit) level.
const DARK_LUX: f32 = 10.0;
/// Above this lux the AOD switches to the high (60 nit) level.
const LIGHT_LUX: f32 = 40.0;

const POLL_TIMEOUT_MS: c_int = 200;
const SENSOR_RATE_US: i32 = 500_000;

/// Diagnostic command channel, polled only while the debug property is set.
const CMD_PATH: &str = "/data/local/tmp/xia_aod_cmd";
const SENSOR_TEST_LOG: &str = "/data/local/tmp/xia_sensor_test.log";
/// Raw DSI command channel (mi_display sysfs, writable by the system uid).
const MIPI_RW_PATH: &str =
    "/sys/devices/virtual/mi_display/disp_feature/disp-DSI-0/mipi_rw";

/// Last doze-brightness value this process wrote (0/1/2), -1 when unknown.
static LAST_MODE: AtomicI32 = AtomicI32::new(-1);

// ---------------------------------------------------------------------------
// libsensorndkbridge FFI
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
struct ASensorEvent {
    version: i32,
    sensor: i32,
    type_: i32,
    reserved0: i32,
    timestamp: i64,
    data: [f32; 16],
    flags: u32,
    reserved1: [i32; 3],
}

impl ASensorEvent {
    fn zeroed() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

type ALooperCallback = Option<unsafe extern "C" fn(c_int, c_int, *mut c_void) -> c_int>;

#[derive(Clone, Copy)]
struct Api {
    get_instance: unsafe extern "C" fn() -> *mut c_void,
    get_default_sensor: unsafe extern "C" fn(*mut c_void, c_int) -> *const c_void,
    create_event_queue:
        unsafe extern "C" fn(*mut c_void, *mut c_void, c_int, ALooperCallback, *mut c_void) -> *mut c_void,
    destroy_event_queue: unsafe extern "C" fn(*mut c_void),
    enable_sensor: unsafe extern "C" fn(*mut c_void, *const c_void) -> c_int,
    disable_sensor: unsafe extern "C" fn(*mut c_void, *const c_void) -> c_int,
    set_event_rate: unsafe extern "C" fn(*mut c_void, *const c_void, i32) -> c_int,
    get_events: unsafe extern "C" fn(*mut c_void, *mut ASensorEvent, usize) -> isize,
    looper_prepare: unsafe extern "C" fn(c_int) -> *mut c_void,
    looper_poll_once: unsafe extern "C" fn(c_int, *mut c_int, *mut c_int, *mut *mut c_void) -> c_int,
    sensor_get_name: unsafe extern "C" fn(*const c_void) -> *const c_char,
    sensor_get_type: unsafe extern "C" fn(*const c_void) -> c_int,
    sensor_get_handle: unsafe extern "C" fn(*const c_void) -> c_int,
}

fn dlerror_string() -> String {
    unsafe {
        let p = libc::dlerror();
        if p.is_null() {
            "unknown dlopen error".to_string()
        } else {
            CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }
}

unsafe fn sym<T: Copy>(handle: *mut c_void, name: &str) -> Result<T, String> {
    let c_name = CString::new(name).map_err(|e| e.to_string())?;
    let p = libc::dlsym(handle, c_name.as_ptr());
    if p.is_null() {
        return Err(format!("missing symbol {}", name));
    }
    Ok(std::mem::transmute_copy(&p))
}

impl Api {
    fn load() -> Result<Self, String> {
        // The vendor linker namespace cannot open libandroid.so, but it can
        // open libsensorndkbridge.so (the NDK API shim used by vendor HALs).
        // Keep libandroid as a fallback for the diagnostic /data execution.
        let mut handle = std::ptr::null_mut();
        let mut errors = Vec::new();
        for name in ["libsensorndkbridge.so", "libandroid.so"] {
            let c_name = CString::new(name).unwrap();
            let h = unsafe { libc::dlopen(c_name.as_ptr(), libc::RTLD_NOW) };
            if !h.is_null() {
                handle = h;
                break;
            }
            errors.push(format!("{}: {}", name, dlerror_string()));
        }
        if handle.is_null() {
            return Err(errors.join("; "));
        }

        unsafe {
            Ok(Self {
                get_instance: sym(handle, "ASensorManager_getInstance")?,
                get_default_sensor: sym(handle, "ASensorManager_getDefaultSensor")?,
                create_event_queue: sym(handle, "ASensorManager_createEventQueue")?,
                destroy_event_queue: sym(handle, "ASensorManager_destroyEventQueue")?,
                enable_sensor: sym(handle, "ASensorEventQueue_enableSensor")?,
                disable_sensor: sym(handle, "ASensorEventQueue_disableSensor")?,
                set_event_rate: sym(handle, "ASensorEventQueue_setEventRate")?,
                get_events: sym(handle, "ASensorEventQueue_getEvents")?,
                looper_prepare: sym(handle, "ALooper_prepare")?,
                looper_poll_once: sym(handle, "ALooper_pollOnce")?,
                sensor_get_name: sym(handle, "ASensor_getName")?,
                sensor_get_type: sym(handle, "ASensor_getType")?,
                sensor_get_handle: sym(handle, "ASensor_getHandle")?,
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Sensor client
// ---------------------------------------------------------------------------

struct SensorClient {
    api: Api,
    _manager: *mut c_void,
    queue: *mut c_void,
    sensor: *const c_void,
    _looper: *mut c_void,
    name: String,
    sensor_type: c_int,
    handle: c_int,
}

impl SensorClient {
    fn open(api: Api, forced_type: Option<c_int>) -> Result<Self, String> {
        let manager = unsafe { (api.get_instance)() };
        if manager.is_null() {
            return Err("ASensorManager_getInstance returned null".to_string());
        }

        let candidates: Vec<c_int> = match forced_type {
            Some(t) => vec![t],
            None => vec![SENSOR_TYPE_LIGHT, SENSOR_TYPE_LIGHT_SMD],
        };

        let mut errors = Vec::new();
        for sensor_type in candidates {
            let sensor = unsafe { (api.get_default_sensor)(manager, sensor_type) };
            if sensor.is_null() {
                errors.push(format!("sensor type {} is not present", sensor_type));
                continue;
            }

            let name = unsafe {
                let p = (api.sensor_get_name)(sensor);
                if p.is_null() {
                    "?".to_string()
                } else {
                    CStr::from_ptr(p).to_string_lossy().into_owned()
                }
            };
            let actual_type = unsafe { (api.sensor_get_type)(sensor) };
            let handle = unsafe { (api.sensor_get_handle)(sensor) };

            let looper = unsafe { (api.looper_prepare)(0) };
            if looper.is_null() {
                errors.push("ALooper_prepare failed".to_string());
                continue;
            }

            let queue =
                unsafe { (api.create_event_queue)(manager, looper, 1, None, std::ptr::null_mut()) };
            if queue.is_null() {
                errors.push("ASensorManager_createEventQueue failed".to_string());
                continue;
            }

            let rc = unsafe { (api.enable_sensor)(queue, sensor) };
            if rc != 0 {
                unsafe { (api.destroy_event_queue)(queue) };
                errors.push(format!("enableSensor({}) returned {}", name, rc));
                continue;
            }
            unsafe { (api.set_event_rate)(queue, sensor, SENSOR_RATE_US) };

            return Ok(Self {
                api,
                _manager: manager,
                queue,
                sensor,
                _looper: looper,
                name,
                sensor_type: actual_type,
                handle,
            });
        }

        Err(errors.join("; "))
    }

    fn poll(&self, events: &mut [ASensorEvent]) -> usize {
        unsafe {
            (self.api.looper_poll_once)(
                POLL_TIMEOUT_MS,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            let n = (self.api.get_events)(self.queue, events.as_mut_ptr(), events.len());
            if n <= 0 {
                0
            } else {
                n as usize
            }
        }
    }

    fn is_lux_event(&self, event: &ASensorEvent) -> bool {
        event.type_ == SENSOR_TYPE_LIGHT
            || event.type_ == SENSOR_TYPE_LIGHT_SMD
            || event.sensor == self.handle
    }

    fn disable(&self) {
        unsafe {
            (self.api.disable_sensor)(self.queue, self.sensor);
        }
    }
}

// ---------------------------------------------------------------------------
// doze_brightness control
// ---------------------------------------------------------------------------

fn read_current_mode() -> i32 {
    std::fs::read_to_string(doze_brightness_path())
        .ok()
        .and_then(|s| s.trim().parse::<i32>().ok())
        .unwrap_or(-1)
}

fn write_mode(mode: i32, reason: &str, force: bool) {
    if !force && LAST_MODE.load(Ordering::SeqCst) == mode {
        return;
    }

    match std::fs::write(doze_brightness_path(), format!("{}\n", mode)) {
        Ok(_) => {
            LAST_MODE.store(mode, Ordering::SeqCst);
            log_d(&format!(
                "[Aod] doze_brightness={} ({}) reason={}",
                mode,
                match mode {
                    DOZE_BRIGHTNESS_HBM => "HBM/60nit",
                    DOZE_BRIGHTNESS_LBM => "LBM/5nit",
                    DOZE_TO_NORMAL => "normal",
                    _ => "unknown",
                },
                reason
            ));
        }
        Err(e) => {
            log_e(&format!(
                "[Aod] cannot write {}={} ({}): {}",
                doze_brightness_path(),
                mode,
                reason,
                e
            ));
        }
    }
}

pub fn apply_doze_mode(mode: i32, reason: &str) {
    write_mode(mode, reason, false);
}

pub fn in_doze() -> bool {
    matches!(
        crate::properties::get_prop_int("debug.tracing.screen_state"),
        Some(SCREEN_STATE_DOZE) | Some(SCREEN_STATE_DOZE_SUSPEND)
    )
}

/// SystemUI's `OplusOSAodManager.setAODLightMode(I)` path (panel feature 7).
///
/// The panel driver has already selected the bright AOD level when the panel
/// entered doze. A second level change is not forwarded because this DDIC
/// blanks itself when `0x6D` is rewritten during AOD.
pub fn on_aod_light_mode(mode: i32) {
    // Do not forward this to the panel: a mid-AOD 0x6D write blanks the
    // DDIC, and the entry sequence already selected HBM.
    log_d(&format!(
        "[Aod] feature 7 light mode {} ignored (AOD stays at HBM)",
        mode
    ));
}

fn desired_for_lux(lux: f32) -> i32 {
    if lux <= DARK_LUX {
        DOZE_BRIGHTNESS_LBM
    } else if lux >= LIGHT_LUX {
        DOZE_BRIGHTNESS_HBM
    } else {
        // Hysteresis: keep the level chosen for the previous environment.
        let current = LAST_MODE.load(Ordering::SeqCst);
        if current == DOZE_BRIGHTNESS_HBM || current == DOZE_BRIGHTNESS_LBM {
            current
        } else {
            DOZE_BRIGHTNESS_LBM
        }
    }
}

// ---------------------------------------------------------------------------
// Background monitor
// ---------------------------------------------------------------------------

pub fn spawn() {
    let _ = std::thread::Builder::new()
        .name("aod-light".to_string())
        .spawn(aod_thread);
}

fn aod_thread() {
    let api = match Api::load() {
        Ok(api) => api,
        Err(e) => {
            log_e(&format!("[Aod] sensor backend unavailable: {}", e));
            return;
        }
    };

    log_d(&format!(
        "[Aod] ambient light monitor started (doze node: {})",
        doze_brightness_path()
    ));

    let initial = read_current_mode();
    if initial >= 0 {
        LAST_MODE.store(initial, Ordering::SeqCst);
    }

    let mut client: Option<SensorClient> = None;
    let mut next_open = Instant::now();
    let mut last_state = i32::MIN;
    let mut was_dozing = false;
    let mut last_lux: Option<f32> = None;
    let mut manual_lux: Option<f32> = None;
    let mut last_lux_log = Instant::now() - Duration::from_secs(5);
    let mut last_cmd = std::fs::read_to_string(CMD_PATH)
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    loop {
        // (Re)open the sensor if it is not available yet.
        if client.is_none() && Instant::now() >= next_open {
            match SensorClient::open(api, None) {
                Ok(c) => {
                    log_d(&format!(
                        "[Aod] sensor registered: name={} type={} handle=0x{:08x}",
                        c.name, c.sensor_type, c.handle
                    ));
                    client = Some(c);
                }
                Err(e) => {
                    log_e(&format!("[Aod] cannot register light sensor: {}", e));
                    next_open = Instant::now() + Duration::from_secs(30);
                }
            }
        }

        let mut events = [ASensorEvent::zeroed(); 8];
        let count = match client.as_ref() {
            Some(c) => c.poll(&mut events),
            None => {
                std::thread::sleep(Duration::from_millis(POLL_TIMEOUT_MS as u64));
                0
            }
        };

        for event in &events[..count] {
            let is_lux = client
                .as_ref()
                .map(|c| c.is_lux_event(event))
                .unwrap_or(false);
            if !is_lux {
                continue;
            }
            let lux = event.data[0];
            if lux.is_finite() && lux >= 0.0 {
                last_lux = Some(lux);
                if last_lux_log.elapsed() >= Duration::from_secs(2) {
                    log_d(&format!(
                        "[Aod] lux={:.1} (sensor type {}, handle 0x{:08x})",
                        lux, event.type_, event.sensor
                    ));
                    last_lux_log = Instant::now();
                }
            }
        }

        let state = crate::properties::get_prop_int("debug.tracing.screen_state")
            .unwrap_or(SCREEN_STATE_ON);

        // Diagnostic file: "lux <value>", "auto", "mode <0|1|2>", "status".
        if crate::modes::dbg_on() {
            if let Ok(content) = std::fs::read_to_string(CMD_PATH) {
                let cmd = content.trim().to_string();
                if cmd != last_cmd && !cmd.is_empty() {
                    last_cmd = cmd.clone();
                    let mut parts = cmd.split_whitespace();
                    match parts.next() {
                        Some("lux") => {
                            if let Some(v) = parts.next().and_then(|s| s.parse::<f32>().ok()) {
                                manual_lux = Some(v);
                                log_d(&format!("[Aod] manual lux set to {:.1}", v));
                            }
                        }
                        Some("auto") => {
                            manual_lux = None;
                            log_d("[Aod] manual lux cleared");
                        }
                        Some("mode") => {
                            if let Some(v) = parts.next().and_then(|s| s.parse::<i32>().ok()) {
                                write_mode(v, "command", true);
                            }
                        }
                        Some("mipi") => {
                            let payload = cmd[4..].trim();
                            match std::fs::write(MIPI_RW_PATH, format!("{}\n", payload)) {
                                Ok(_) => log_d(&format!("[Aod] mipi_rw <= {}", payload)),
                                Err(e) => {
                                    log_e(&format!("[Aod] mipi_rw write failed: {}", e))
                                }
                            }
                        }
                        Some("normal") | Some("off") => {
                            write_mode(DOZE_TO_NORMAL, "command", true)
                        }
                        Some("status") => {
                            log_d(&format!(
                                "[Aod] status: screen={} lux={:?} manual={:?} last_mode={} sensor={}",
                                state,
                                last_lux,
                                manual_lux,
                                LAST_MODE.load(Ordering::SeqCst),
                                client
                                    .as_ref()
                                    .map(|c| c.name.as_str())
                                    .unwrap_or("none")
                            ));
                        }
                        other => log_d(&format!("[Aod] unknown command: {:?}", other)),
                    }
                }
            }
        }

        if state != last_state {
            log_d(&format!(
                "[Aod] screen state {} -> {} (lux {:?})",
                last_state, state, last_lux
            ));
            last_state = state;
            // Intentionally no doze-node writes on screen state changes: the
            // kernel's AOD entry sequence selects HBM by itself.
        }

        let dozing = state == SCREEN_STATE_DOZE || state == SCREEN_STATE_DOZE_SUSPEND;
        let effective_lux = manual_lux.or(last_lux);

        if dozing {
            if let Some(lux) = effective_lux {
                let desired = desired_for_lux(lux);
                if !was_dozing {
                    log_d(&format!(
                        "[Aod] doze entry at lux {:.1}: panel keeps its HBM level (desired {})",
                        lux, desired
                    ));
                } else if desired != LAST_MODE.load(Ordering::SeqCst) {
                    log_d(&format!(
                        "[Aod] lux {:.1} wants level {}; not written (mid-AOD 0x6D blanks the panel)",
                        lux, desired
                    ));
                }
            }
        }
        was_dozing = dozing;
    }
}

// ---------------------------------------------------------------------------
// Diagnostic entry point
// ---------------------------------------------------------------------------

/// `--sensor-test [seconds]`: registers the light sensor and logs raw events.
///
/// Used from an adb shell to verify that a native process can reach the sensor
/// service through libsensorndkbridge before the module is rebuilt.
pub fn sensor_test(seconds: u64) {
    let mut out = String::new();
    let api = match Api::load() {
        Ok(api) => {
            out.push_str("sensor backend loaded\n");
            api
        }
        Err(e) => {
            let msg = format!("sensor backend unavailable: {}\n", e);
            log_e(&format!("[AodTest] {}", msg.trim()));
            append_test_log(&msg);
            return;
        }
    };

    for sensor_type in [SENSOR_TYPE_LIGHT, SENSOR_TYPE_LIGHT_SMD] {
        let manager = unsafe { (api.get_instance)() };
        if manager.is_null() {
            out.push_str("getInstance returned null\n");
            break;
        }
        let sensor = unsafe { (api.get_default_sensor)(manager, sensor_type) };
        if sensor.is_null() {
            out.push_str(&format!("type {}: absent\n", sensor_type));
            continue;
        }
        let name = unsafe {
            let p = (api.sensor_get_name)(sensor);
            if p.is_null() {
                "?".to_string()
            } else {
                CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        };
        let actual_type = unsafe { (api.sensor_get_type)(sensor) };
        let handle = unsafe { (api.sensor_get_handle)(sensor) };
        out.push_str(&format!(
            "type {}: name={} actual_type={} handle=0x{:08x}\n",
            sensor_type, name, actual_type, handle
        ));
    }

    let client = match SensorClient::open(api, None) {
        Ok(client) => client,
        Err(e) => {
            let msg = format!("register failed: {}\n", e);
            out.push_str(&msg);
            log_e(&format!("[AodTest] {}", msg.trim()));
            append_test_log(&out);
            return;
        }
    };

    out.push_str(&format!(
        "registered name={} type={} handle=0x{:08x}\n",
        client.name, client.sensor_type, client.handle
    ));
    log_d(&format!("[AodTest] {}", out.replace('\n', " | ")));

    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut events = [ASensorEvent::zeroed(); 8];
    let mut count = 0usize;
    while Instant::now() < deadline {
        let n = client.poll(&mut events);
        for event in &events[..n] {
            let lux = event.data[0];
            out.push_str(&format!(
                "event {}: type={} handle=0x{:08x} value0={:.2}\n",
                count, event.type_, event.sensor, lux
            ));
            count += 1;
            if count <= 20 {
                log_d(&format!(
                    "[AodTest] event {}: type={} handle=0x{:08x} value0={:.2}",
                    count, event.type_, event.sensor, lux
                ));
            }
        }
    }

    client.disable();
    out.push_str(&format!("done, {} events\n", count));
    append_test_log(&out);
    log_d(&format!("[AodTest] done, {} events", count));
}

fn append_test_log(text: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(SENSOR_TEST_LOG)
    {
        let _ = f.write_all(text.as_bytes());
    }
}

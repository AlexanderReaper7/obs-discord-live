//! OBS module: an output that encodes the program view for Discord Go Live and
//! hands the frames to a client over a Unix socket (`odl_proto`).
//!
//! The output has no button in OBS. It starts when a client sends `Start` and
//! stops when the client sends `Stop` or disconnects.

mod encoder;
mod output;
mod server;

mod obs {
    #![allow(non_camel_case_types, non_snake_case, non_upper_case_globals, dead_code, unsafe_op_in_unsafe_fn)]
    include!(concat!(env!("OUT_DIR"), "/obs.rs"));
}

use std::ffi::{CString, c_char, c_void};
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

static MODULE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static SHARED: OnceLock<Arc<server::Shared>> = OnceLock::new();
static SERVER: Mutex<Option<server::Server>> = Mutex::new(None);

fn shared() -> &'static Arc<server::Shared> {
    SHARED.get_or_init(Arc::default)
}

fn log(level: u32, msg: &str) {
    let msg = CString::new(format!("[obs-discord-live] {msg}")).unwrap_or_default();
    unsafe { obs::blog(level as i32, c"%s".as_ptr(), msg.as_ptr()) };
}

fn info(msg: &str) {
    log(obs::LOG_INFO, msg);
}

fn warn(msg: &str) {
    log(obs::LOG_WARNING, msg);
}

#[unsafe(no_mangle)]
pub extern "C" fn obs_module_set_pointer(module: *mut c_void) {
    MODULE.store(module, Ordering::SeqCst);
}

#[unsafe(no_mangle)]
pub extern "C" fn obs_current_module() -> *mut c_void {
    MODULE.load(Ordering::SeqCst)
}

#[unsafe(no_mangle)]
pub extern "C" fn obs_module_ver() -> u32 {
    (obs::LIBOBS_API_MAJOR_VER << 24) | (obs::LIBOBS_API_MINOR_VER << 16) | obs::LIBOBS_API_PATCH_VER
}

#[unsafe(no_mangle)]
pub extern "C" fn obs_module_name() -> *const c_char {
    c"obs-discord-live".as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn obs_module_description() -> *const c_char {
    c"Encodes the program view for Discord Go Live, with keyframes and bitrate on the receiver's request".as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn obs_module_load() -> bool {
    output::register();
    true
}

#[unsafe(no_mangle)]
pub extern "C" fn obs_module_post_load() {
    output::create_instance();
    match server::spawn(shared().clone()) {
        Ok(s) => *SERVER.lock().unwrap() = Some(s),
        Err(e) => warn(&format!("socket: {e}")),
    }
    unsafe { obs::obs_frontend_add_event_callback(Some(on_event), std::ptr::null_mut()) };
}

unsafe extern "C" fn on_event(event: obs::obs_frontend_event, _: *mut c_void) {
    if event == obs::obs_frontend_event_OBS_FRONTEND_EVENT_EXIT {
        shutdown();
    }
}

/// Stops the server's threads and the output before OBS unloads this library,
/// which it does with dlclose.
fn shutdown() {
    if let Some(mut s) = SERVER.lock().unwrap().take() {
        s.shutdown(shared());
    }
    output::release_instance();
}

#[unsafe(no_mangle)]
pub extern "C" fn obs_module_unload() {
    shutdown();
}

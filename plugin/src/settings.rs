//! The plugin's settings, edited in OBS's own properties window from Tools →
//! Discord Go Live.
//!
//! OBS has no settings page for an output, but the frontend opens the
//! properties window of any source. The settings therefore live in a private
//! source of a type hidden from the Add Source menu; OBS calls `update` when
//! the user presses OK. They are saved to `settings.json` in the module's
//! config directory (`plugin_config/obs-discord-live`), the same for every
//! profile and scene collection.

use std::ffi::{CStr, CString, c_char, c_void};
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Mutex, MutexGuard};

use odl_proto::Addr;

use crate::encoder::{MAX_KBPS, MIN_KBPS};
use crate::obs;

const ID: &CStr = c"discord_live_settings";
const NAME: &CStr = c"Discord Go Live";
const FILE: &CStr = c"settings.json";

pub const X264_PRESETS: [&str; 6] = ["ultrafast", "superfast", "veryfast", "faster", "fast", "medium"];
pub const NVENC_PRESETS: [&str; 7] = ["p1", "p2", "p3", "p4", "p5", "p6", "p7"];
pub const PROFILES: [&str; 3] = ["baseline", "main", "high"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// `libx264` or `h264_nvenc`.
    pub h264_encoder: String,
    pub h264_profile: String,
    pub x264_preset: String,
    pub nvenc_preset: String,
    pub keyframe_secs: u32,
    /// The highest bitrate the encoder takes from the receiver's estimate.
    pub max_kbps: u32,
    /// Empty means `$OBS_DISCORD_LIVE_ADDR`, else the built-in default.
    pub address: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            h264_encoder: "libx264".into(),
            // WebRTC offers H.264 as 42e01f, constrained baseline level 3.1.
            h264_profile: "baseline".into(),
            x264_preset: "veryfast".into(),
            nvenc_preset: "p4".into(),
            keyframe_secs: 10,
            max_kbps: MAX_KBPS,
            address: String::new(),
        }
    }
}

impl Config {
    pub fn addr(&self) -> Addr {
        let a = self.address.trim();
        if a.is_empty() { odl_proto::addr() } else { Addr::parse(a) }
    }
}

static CONFIG: Mutex<Option<Config>> = Mutex::new(None);
static SOURCE: AtomicPtr<obs::obs_source_t> = AtomicPtr::new(ptr::null_mut());

/// The current settings; the defaults before the source exists.
pub fn get() -> Config {
    lock().clone().unwrap_or_default()
}

/// The bitrate cap alone, read on every encoded frame.
pub fn max_kbps() -> u32 {
    lock().as_ref().map_or(MAX_KBPS, |c| c.max_kbps)
}

fn lock() -> MutexGuard<'static, Option<Config>> {
    CONFIG.lock().unwrap_or_else(|e| e.into_inner())
}

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

unsafe fn get_string(data: *mut obs::obs_data_t, key: &CStr) -> String {
    let p = unsafe { obs::obs_data_get_string(data, key.as_ptr()) };
    if p.is_null() { String::new() } else { unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned() }
}

/// A list value the dialog offered, or the default when the file holds something else.
fn one_of(v: String, allowed: &[&str], default: String) -> String {
    if allowed.contains(&v.as_str()) { v } else { default }
}

unsafe fn read(data: *mut obs::obs_data_t) -> Config {
    let d = Config::default();
    unsafe {
        Config {
            h264_encoder: one_of(get_string(data, c"h264_encoder"), &["libx264", "h264_nvenc"], d.h264_encoder),
            h264_profile: one_of(get_string(data, c"h264_profile"), &PROFILES, d.h264_profile),
            x264_preset: one_of(get_string(data, c"x264_preset"), &X264_PRESETS, d.x264_preset),
            nvenc_preset: one_of(get_string(data, c"nvenc_preset"), &NVENC_PRESETS, d.nvenc_preset),
            keyframe_secs: obs::obs_data_get_int(data, c"keyframe_secs".as_ptr()).clamp(1, 60) as u32,
            max_kbps: obs::obs_data_get_int(data, c"max_kbps".as_ptr()).clamp(MIN_KBPS as i64, MAX_KBPS as i64)
                as u32,
            address: get_string(data, c"address").trim().to_string(),
        }
    }
}

fn config_path() -> Option<CString> {
    let p = unsafe { obs::obs_module_get_config_path(crate::obs_current_module() as *mut _, FILE.as_ptr()) };
    if p.is_null() {
        return None;
    }
    let path = unsafe { CStr::from_ptr(p) }.to_owned();
    unsafe { obs::bfree(p as *mut c_void) };
    Some(path)
}

unsafe extern "C" fn get_name(_: *mut c_void) -> *const c_char {
    NAME.as_ptr()
}

unsafe extern "C" fn create(settings: *mut obs::obs_data_t, _: *mut obs::obs_source_t) -> *mut c_void {
    *lock() = Some(unsafe { read(settings) });
    // OBS treats a null return as a failed create.
    ptr::NonNull::<u8>::dangling().as_ptr() as *mut c_void
}

unsafe extern "C" fn destroy(_: *mut c_void) {}

unsafe extern "C" fn get_defaults(data: *mut obs::obs_data_t) {
    let d = Config::default();
    unsafe {
        obs::obs_data_set_default_string(data, c"h264_encoder".as_ptr(), cstr(&d.h264_encoder).as_ptr());
        obs::obs_data_set_default_string(data, c"h264_profile".as_ptr(), cstr(&d.h264_profile).as_ptr());
        obs::obs_data_set_default_string(data, c"x264_preset".as_ptr(), cstr(&d.x264_preset).as_ptr());
        obs::obs_data_set_default_string(data, c"nvenc_preset".as_ptr(), cstr(&d.nvenc_preset).as_ptr());
        obs::obs_data_set_default_int(data, c"keyframe_secs".as_ptr(), d.keyframe_secs as i64);
        obs::obs_data_set_default_int(data, c"max_kbps".as_ptr(), d.max_kbps as i64);
        obs::obs_data_set_default_string(data, c"address".as_ptr(), c"".as_ptr());
    }
}

unsafe fn add_list(
    props: *mut obs::obs_properties_t,
    key: &CStr,
    label: &CStr,
    items: &[(&str, &str)],
) -> *mut obs::obs_property_t {
    unsafe {
        let p = obs::obs_properties_add_list(
            props,
            key.as_ptr(),
            label.as_ptr(),
            obs::obs_combo_type_OBS_COMBO_TYPE_LIST,
            obs::obs_combo_format_OBS_COMBO_FORMAT_STRING,
        );
        for (value, text) in items {
            obs::obs_property_list_add_string(p, cstr(text).as_ptr(), cstr(value).as_ptr());
        }
        p
    }
}

unsafe fn add_info(props: *mut obs::obs_properties_t, key: &CStr, text: &str, kind: obs::obs_text_info_type) {
    unsafe {
        let p = obs::obs_properties_add_text(props, key.as_ptr(), cstr(text).as_ptr(), obs::obs_text_type_OBS_TEXT_INFO);
        obs::obs_property_text_set_info_type(p, kind);
    }
}

unsafe extern "C" fn get_properties(_: *mut c_void) -> *mut obs::obs_properties_t {
    unsafe {
        let props = obs::obs_properties_create();
        // Nothing applies until OK, so a half-typed address never rebinds the socket.
        obs::obs_properties_set_flags(props, obs::OBS_PROPERTIES_DEFER_UPDATE);

        add_info(props, c"status", &crate::status(), obs::obs_text_info_type_OBS_TEXT_INFO_NORMAL);

        let p = add_list(
            props,
            c"h264_encoder",
            c"H.264 encoder",
            &[("libx264", "x264 (CPU)"), ("h264_nvenc", "NVENC (NVIDIA GPU)")],
        );
        obs::obs_property_set_long_description(
            p,
            c"Discord negotiates H.264, so this is the encoder a Go Live uses. AV1, VP8 and VP9 use SVT-AV1 and libvpx."
                .as_ptr(),
        );
        add_info(
            props,
            c"nvenc_warning",
            "NVENC hung a passed-through RTX 3080 in a VM guest on 2026-10-24 and ended the running game. \
             Use it on a host GPU only.",
            obs::obs_text_info_type_OBS_TEXT_INFO_WARNING,
        );
        add_list(
            props,
            c"h264_profile",
            c"H.264 profile",
            &[
                ("baseline", "Constrained baseline (what WebRTC offers)"),
                ("main", "Main"),
                ("high", "High (about 10 % less bitrate, if every viewer decodes it)"),
            ],
        );
        let x264: Vec<(&str, &str)> = X264_PRESETS.iter().map(|p| (*p, *p)).collect();
        add_list(props, c"x264_preset", c"x264 preset", &x264);
        let nvenc: Vec<(&str, &str)> = NVENC_PRESETS.iter().map(|p| (*p, *p)).collect();
        add_list(props, c"nvenc_preset", c"NVENC preset", &nvenc);
        let p = obs::obs_properties_add_int(props, c"keyframe_secs".as_ptr(), c"Keyframe interval".as_ptr(), 1, 60, 1);
        obs::obs_property_int_set_suffix(p, c" s".as_ptr());
        obs::obs_property_set_long_description(
            p,
            c"The longest time between keyframes. A viewer's request for one is answered on the next frame regardless."
                .as_ptr(),
        );
        let p = obs::obs_properties_add_int(
            props,
            c"max_kbps".as_ptr(),
            c"Maximum bitrate".as_ptr(),
            MIN_KBPS as i32,
            MAX_KBPS as i32,
            100,
        );
        obs::obs_property_int_set_suffix(p, c" kbps".as_ptr());
        obs::obs_property_set_long_description(
            p,
            c"Discord sets the bitrate from its bandwidth estimate; the encoder never goes above this.".as_ptr(),
        );
        let p = obs::obs_properties_add_text(
            props,
            c"address".as_ptr(),
            c"Listen address".as_ptr(),
            obs::obs_text_type_OBS_TEXT_DEFAULT,
        );
        obs::obs_property_set_long_description(
            p,
            c"HOST:PORT for TCP, anything else is a Unix socket path. Empty: $OBS_DISCORD_LIVE_ADDR, else a socket \
              in $XDG_RUNTIME_DIR on Linux and 0.0.0.0:4457 on Windows."
                .as_ptr(),
        );
        add_info(
            props,
            c"apply_note",
            "Encoder changes apply at the next Go Live; the maximum bitrate applies at once. \
             A new listen address ends a running Go Live.",
            obs::obs_text_info_type_OBS_TEXT_INFO_NORMAL,
        );
        props
    }
}

unsafe extern "C" fn update(_: *mut c_void, data: *mut obs::obs_data_t) {
    let new = unsafe { read(data) };
    let old = lock().replace(new.clone());
    if let Some(path) = config_path() {
        let ok = unsafe { obs::obs_data_save_json_safe(data, path.as_ptr(), c"tmp".as_ptr(), c"bak".as_ptr()) };
        if !ok {
            crate::warn(&format!("could not save {}", path.to_string_lossy()));
        }
    }
    crate::info(&format!("settings: {new:?}"));
    if old.is_some_and(|o| o.addr() != new.addr()) {
        crate::restart_server();
    }
}

pub fn register() {
    let info = obs::obs_source_info {
        id: ID.as_ptr(),
        type_: obs::obs_source_type_OBS_SOURCE_TYPE_INPUT,
        // No audio or video, and hidden from Add Source.
        output_flags: obs::OBS_SOURCE_CAP_DISABLED,
        get_name: Some(get_name),
        create: Some(create),
        destroy: Some(destroy),
        get_defaults: Some(get_defaults),
        get_properties: Some(get_properties),
        update: Some(update),
        ..Default::default()
    };
    unsafe { obs::obs_register_source_s(&info, size_of::<obs::obs_source_info>()) };
}

/// Loads the saved settings into a private source and adds the Tools menu item.
pub fn load() {
    let saved = match config_path() {
        Some(path) => unsafe { obs::obs_data_create_from_json_file_safe(path.as_ptr(), c"bak".as_ptr()) },
        None => ptr::null_mut(),
    };
    let src = unsafe { obs::obs_source_create_private(ID.as_ptr(), NAME.as_ptr(), saved) };
    if !saved.is_null() {
        unsafe { obs::obs_data_release(saved) };
    }
    if src.is_null() {
        crate::warn("could not create the settings source; using the defaults");
        return;
    }
    SOURCE.store(src, Ordering::SeqCst);
    if let Some(path) = config_path() {
        // The config directory does not exist until something is saved in it.
        if let Some(dir) = std::path::Path::new(path.to_str().unwrap_or_default()).parent() {
            let _ = std::fs::create_dir_all(dir);
        }
    }
    unsafe { obs::obs_frontend_add_tools_menu_item(NAME.as_ptr(), Some(open), ptr::null_mut()) };
}

unsafe extern "C" fn open(_: *mut c_void) {
    let src = SOURCE.load(Ordering::SeqCst);
    if !src.is_null() {
        unsafe { obs::obs_frontend_open_source_properties(src) };
    }
}

pub fn release() {
    let src = SOURCE.swap(ptr::null_mut(), Ordering::SeqCst);
    if !src.is_null() {
        unsafe { obs::obs_source_release(src) };
    }
}

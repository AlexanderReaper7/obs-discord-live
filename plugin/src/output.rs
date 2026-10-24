//! The OBS output: raw I420 frames from the program view, in the canvas's output size and rate.

use std::ffi::{CStr, CString, c_char, c_void};
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use ffmpeg_next as ff;
use odl_proto::ServerMsg;

use crate::encoder::{Encoder, Settings};
use crate::obs;
use crate::server::Shared;

pub const ID: &CStr = c"discord_live_output";

/// The one instance the plugin creates; null before post-load and after exit.
pub static OUTPUT: AtomicPtr<obs::obs_output_t> = AtomicPtr::new(ptr::null_mut());

pub enum Action {
    Start,
    Stop,
}

/// Runs `action` on OBS's UI thread, where the frontend starts and stops its own outputs.
pub fn queue(action: Action) {
    unsafe extern "C" fn run(param: *mut c_void) {
        let action = unsafe { Box::from_raw(param as *mut Action) };
        let out = OUTPUT.load(Ordering::SeqCst);
        if out.is_null() {
            return;
        }
        unsafe {
            let active = obs::obs_output_active(out);
            match *action {
                Action::Start => {
                    if active {
                        obs::obs_output_stop(out);
                    }
                    if !obs::obs_output_start(out) {
                        let err = obs::obs_output_get_last_error(out);
                        let msg = if err.is_null() {
                            "the output did not start; see the OBS log".to_string()
                        } else {
                            CStr::from_ptr(err).to_string_lossy().into_owned()
                        };
                        crate::warn(&format!("start: {msg}"));
                        crate::shared().send(&ServerMsg::Error { message: msg });
                    }
                }
                Action::Stop => {
                    if active {
                        obs::obs_output_stop(out);
                    }
                }
            }
        }
    }
    let param = Box::into_raw(Box::new(action)) as *mut c_void;
    unsafe { obs::obs_queue_task(obs::obs_task_type_OBS_TASK_UI, Some(run), param, false) };
}

struct Run {
    tx: SyncSender<ff::frame::Video>,
    thread: JoinHandle<()>,
    width: u32,
    height: u32,
    fps_num: u32,
    fps_den: u32,
    first_ts: Option<u64>,
    dropped: u64,
}

struct Output {
    raw: *mut obs::obs_output_t,
    run: Mutex<Option<Run>>,
}

unsafe extern "C" fn get_name(_: *mut c_void) -> *const c_char {
    c"Discord Go Live".as_ptr()
}

unsafe extern "C" fn create(_: *mut obs::obs_data_t, raw: *mut obs::obs_output_t) -> *mut c_void {
    Box::into_raw(Box::new(Output { raw, run: Mutex::new(None) })) as *mut c_void
}

unsafe extern "C" fn destroy(data: *mut c_void) {
    let out = unsafe { Box::from_raw(data as *mut Output) };
    stop_run(&out);
}

fn fail(out: &Output, msg: &str) -> bool {
    crate::warn(msg);
    if let Ok(c) = CString::new(msg) {
        unsafe { obs::obs_output_set_last_error(out.raw, c.as_ptr()) };
    }
    false
}

unsafe extern "C" fn start(data: *mut c_void) -> bool {
    let out = unsafe { &*(data as *const Output) };
    let shared = crate::shared();
    let Some(req) = *shared.request.lock().unwrap() else {
        return fail(out, "no client has asked for a stream");
    };
    let mut ovi = obs::obs_video_info::default();
    if !unsafe { obs::obs_get_video_info(&mut ovi) } {
        return fail(out, "OBS has no video");
    }
    let settings = Settings {
        codec: req.codec,
        width: ovi.output_width,
        height: ovi.output_height,
        fps_num: ovi.fps_num,
        fps_den: ovi.fps_den,
        bitrate_kbps: shared
            .bitrate_kbps
            .load(Ordering::SeqCst)
            .clamp(crate::encoder::MIN_KBPS, crate::encoder::MAX_KBPS),
    };
    let conversion = obs::video_scale_info {
        format: obs::video_format_VIDEO_FORMAT_I420,
        width: settings.width,
        height: settings.height,
        range: obs::video_range_type_VIDEO_RANGE_PARTIAL,
        colorspace: obs::video_colorspace_VIDEO_CS_709,
    };
    unsafe { obs::obs_output_set_video_conversion(out.raw, &conversion) };
    if !unsafe { obs::obs_output_can_begin_data_capture(out.raw, 0) } {
        return fail(out, "OBS cannot begin data capture");
    }
    let encoder = match Encoder::open(settings) {
        Ok(e) => e,
        Err(e) => {
            shared.send(&ServerMsg::Error { message: e.clone() });
            return fail(out, &e);
        }
    };
    crate::info(&format!(
        "encoding {} with {} at {}x{} {}/{} fps, {} kbps",
        settings.codec.name(),
        encoder.name,
        settings.width,
        settings.height,
        settings.fps_num,
        settings.fps_den,
        settings.bitrate_kbps
    ));
    shared.send(&ServerMsg::Config {
        codec: settings.codec.name().into(),
        encoder: encoder.name.into(),
        width: settings.width,
        height: settings.height,
        fps_num: settings.fps_num,
        fps_den: settings.fps_den,
    });
    // Two frames of slack; past that the encoder is behind and frames are dropped.
    let (tx, rx) = sync_channel(2);
    let shared = shared.clone();
    let thread = match std::thread::Builder::new()
        .name("odl-encode".into())
        .spawn(move || encode_loop(encoder, rx, shared))
    {
        Ok(t) => t,
        Err(e) => return fail(out, &format!("encoder thread: {e}")),
    };
    *out.run.lock().unwrap() = Some(Run {
        tx,
        thread,
        width: settings.width,
        height: settings.height,
        fps_num: settings.fps_num,
        fps_den: settings.fps_den,
        first_ts: None,
        dropped: 0,
    });
    if !unsafe { obs::obs_output_begin_data_capture(out.raw, 0) } {
        stop_run(out);
        return fail(out, "OBS did not begin data capture");
    }
    true
}

fn encode_loop(mut encoder: Encoder, rx: Receiver<ff::frame::Video>, shared: Arc<Shared>) {
    for mut frame in rx {
        encoder.follow_bitrate(shared.bitrate_kbps.load(Ordering::SeqCst));
        if shared.keyframe.swap(false, Ordering::SeqCst) {
            encoder.request_keyframe();
        }
        let result = encoder.encode(&mut frame, |pts_us, keyframe, data| {
            shared.send(&ServerMsg::Frame { pts_us, keyframe, data: data.to_vec() });
        });
        if let Err(e) = result {
            crate::warn(&e);
            shared.send(&ServerMsg::Error { message: e });
            queue(Action::Stop);
            break;
        }
    }
}

fn stop_run(out: &Output) {
    let run = out.run.lock().unwrap().take();
    if let Some(run) = run {
        drop(run.tx);
        let _ = run.thread.join();
        crate::info(&format!("stopped; {} frames dropped before the encoder", run.dropped));
    }
}

unsafe extern "C" fn stop(data: *mut c_void, _ts: u64) {
    let out = unsafe { &*(data as *const Output) };
    unsafe { obs::obs_output_end_data_capture(out.raw) };
    stop_run(out);
}

unsafe extern "C" fn raw_video(data: *mut c_void, frame: *mut obs::video_data) {
    let out = unsafe { &*(data as *const Output) };
    let frame = unsafe { &*frame };
    let mut guard = out.run.lock().unwrap();
    let Some(run) = guard.as_mut() else { return };

    let first = *run.first_ts.get_or_insert(frame.timestamp);
    let elapsed = frame.timestamp.saturating_sub(first) as u128;
    // Frame index from OBS's clock, so a dropped frame leaves a gap in pts.
    let den = run.fps_den as u128 * 1_000_000_000;
    let index = (elapsed * run.fps_num as u128 + den / 2) / den;

    let mut v = ff::frame::Video::new(ff::format::Pixel::YUV420P, run.width, run.height);
    for plane in 0..3 {
        let (w, h) = if plane == 0 {
            (run.width as usize, run.height as usize)
        } else {
            (run.width as usize / 2, run.height as usize / 2)
        };
        let src = frame.data[plane];
        let src_stride = frame.linesize[plane] as usize;
        let dst_stride = v.stride(plane);
        let dst = v.data_mut(plane);
        for y in 0..h {
            let row = unsafe { std::slice::from_raw_parts(src.add(y * src_stride), w) };
            dst[y * dst_stride..y * dst_stride + w].copy_from_slice(row);
        }
    }
    v.set_pts(Some(index as i64));
    match run.tx.try_send(v) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => run.dropped += 1,
        Err(TrySendError::Disconnected(_)) => {}
    }
}

pub fn register() {
    let info = obs::obs_output_info {
        id: ID.as_ptr(),
        flags: obs::OBS_OUTPUT_VIDEO,
        get_name: Some(get_name),
        create: Some(create),
        destroy: Some(destroy),
        start: Some(start),
        stop: Some(stop),
        raw_video: Some(raw_video),
        ..Default::default()
    };
    unsafe { obs::obs_register_output_s(&info, size_of::<obs::obs_output_info>()) };
}

pub fn create_instance() {
    let out = unsafe {
        obs::obs_output_create(ID.as_ptr(), c"Discord Go Live".as_ptr(), ptr::null_mut(), ptr::null_mut())
    };
    if out.is_null() {
        crate::warn("obs_output_create returned null");
    }
    OUTPUT.store(out, Ordering::SeqCst);
}

/// Stops and releases the instance. Runs on the UI thread at frontend exit.
pub fn release_instance() {
    let out = OUTPUT.swap(ptr::null_mut(), Ordering::SeqCst);
    if !out.is_null() {
        unsafe {
            if obs::obs_output_active(out) {
                obs::obs_output_stop(out);
            }
            obs::obs_output_release(out);
        }
    }
}

//! The plugin's own video encoder, through libavcodec.
//!
//! libobs has no call that asks one of its encoders for a keyframe, so the
//! output takes raw frames and encodes them here, where a PLI from the receiver
//! can force an IDR on the next frame.

use std::time::{Duration, Instant};

use ffmpeg_next as ff;

use crate::settings::Config;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    H264,
    Av1,
    Vp8,
    Vp9,
}

impl Codec {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_uppercase().as_str() {
            "H264" => Some(Codec::H264),
            "AV1" => Some(Codec::Av1),
            "VP8" => Some(Codec::Vp8),
            "VP9" => Some(Codec::Vp9),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Codec::H264 => "H264",
            Codec::Av1 => "AV1",
            Codec::Vp8 => "VP8",
            Codec::Vp9 => "VP9",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub codec: Codec,
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub bitrate_kbps: u32,
}

pub const MIN_KBPS: u32 = 100;
pub const MAX_KBPS: u32 = 50_000;

/// Encoders not in [`LIVE_RATE`] take a new bitrate only by reopening, which costs
/// a keyframe, so they follow the receiver's estimate at most this often and
/// only when it moved by more than [`REOPEN_STEP`].
const REOPEN_INTERVAL: Duration = Duration::from_secs(2);
const REOPEN_STEP: f64 = 0.15;

pub struct Encoder {
    enc: ff::encoder::Video,
    pub settings: Settings,
    pub name: &'static str,
    last_reopen: Instant,
    force_key: bool,
}

/// Encoders that take a new bitrate on the next frame, without a reopen.
const LIVE_RATE: [&str; 2] = ["libx264", "h264_nvenc"];

/// The encoder's bitrate: the receiver's estimate within the settings' maximum.
pub fn clamp_kbps(kbps: u32) -> u32 {
    kbps.clamp(MIN_KBPS, crate::settings::max_kbps())
}

/// The H.264 encoder from the settings. NVENC is never the default: in the
/// adversary guest on 2026-10-24 each NVENC probe (OBS's obs-nvenc-test.exe, at
/// OBS start and once by hand) hung the passed-through 3080 (nvlddmkm event
/// 153), and the second hang killed the running game with DXGI_ERROR_DEVICE_HUNG.
fn h264_order(c: &Config) -> Vec<&'static str> {
    match c.h264_encoder.as_str() {
        "h264_nvenc" => vec!["h264_nvenc"],
        _ => vec!["libx264"],
    }
}

/// The private options for each libavcodec encoder. Every set is low-delay: no
/// B-frames, no lookahead, a keyframe only on request or at the settings' interval.
fn options(name: &str, c: &Config) -> Vec<(&'static str, String)> {
    match name {
        "h264_nvenc" => vec![
            ("preset", c.nvenc_preset.clone()),
            ("tune", "ull".into()),
            ("profile", c.h264_profile.clone()),
            ("rc", "cbr".into()),
            ("zerolatency", "1".into()),
            ("delay", "0".into()),
            ("rc-lookahead", "0".into()),
            ("forced-idr", "1".into()),
            ("no-scenecut", "1".into()),
        ],
        "libx264" => vec![
            ("preset", c.x264_preset.clone()),
            ("tune", "zerolatency".into()),
            ("profile", c.h264_profile.clone()),
            ("forced-idr", "1".into()),
            // A scene-cut IDR is a bitrate spike nobody asked for; the
            // receiver asks with a PLI when it needs one.
            ("x264-params", "scenecut=0".into()),
        ],
        "libsvtav1" => vec![
            ("preset", "10".into()),
            // Without enable-force-key-frames, SVT-AV1 ignores a frame's
            // key picture type unless every frame is one (libsvtav1.c).
            ("svtav1-params", "rtc=1:pred-struct=1:enable-force-key-frames=1".into()),
        ],
        "libvpx" => vec![
            ("deadline", "realtime".into()),
            ("cpu-used", "8".into()),
            ("lag-in-frames", "0".into()),
            // A keyframe may take nine frames' worth of bits, as in libwebrtc.
            ("max-intra-rate", "900".into()),
        ],
        "libvpx-vp9" => vec![
            ("deadline", "realtime".into()),
            ("cpu-used", "8".into()),
            ("lag-in-frames", "0".into()),
            ("max-intra-rate", "900".into()),
            ("row-mt", "1".into()),
            // Cyclic refresh, libwebrtc's VP9 setting for real-time.
            ("aq-mode", "3".into()),
        ],
        _ => vec![],
    }
}

fn candidates(codec: Codec, c: &Config) -> Vec<&'static str> {
    match codec {
        Codec::H264 => h264_order(c),
        Codec::Av1 => vec!["libsvtav1"],
        Codec::Vp8 => vec!["libvpx"],
        Codec::Vp9 => vec!["libvpx-vp9"],
    }
}

/// Opens the first of the codec's encoders that opens.
fn open(s: &Settings) -> Result<(ff::encoder::Video, &'static str), String> {
    let c = crate::settings::get();
    let mut errors = Vec::new();
    for name in candidates(s.codec, &c) {
        match open_one(name, s, &c) {
            Ok(enc) => return Ok((enc, name)),
            Err(e) => errors.push(e),
        }
    }
    Err(errors.join("; "))
}

fn open_one(name: &'static str, s: &Settings, c: &Config) -> Result<ff::encoder::Video, String> {
    let options = options(name, c);
    let codec =
        ff::encoder::find_by_name(name).ok_or_else(|| format!("libavcodec has no {name}"))?;
    let mut ctx = ff::codec::context::Context::new_with_codec(codec)
        .encoder()
        .video()
        .map_err(|e| format!("{name}: {e}"))?;
    ctx.set_width(s.width);
    ctx.set_height(s.height);
    ctx.set_format(ff::format::Pixel::YUV420P);
    ctx.set_time_base((s.fps_den as i32, s.fps_num as i32));
    ctx.set_frame_rate(Some((s.fps_num as i32, s.fps_den as i32)));
    ctx.set_gop(c.keyframe_secs * s.fps_num / s.fps_den.max(1));
    ctx.set_max_b_frames(0);
    // OBS converts to I420 with these, set in output.rs.
    ctx.set_colorspace(ff::color::Space::BT709);
    ctx.set_color_range(ff::color::Range::MPEG);
    ctx.set_color_primaries(ff::color::Primaries::BT709);
    ctx.set_color_transfer_characteristic(ff::color::TransferCharacteristic::BT709);
    set_rate(&mut ctx, s.bitrate_kbps);

    let mut dict = ff::Dictionary::new();
    for (k, v) in &options {
        dict.set(k, v);
    }
    ctx.open_with(dict).map_err(|e| format!("{name}: {e}"))
}

/// Constant bitrate with a half-second buffer, so a frame never overshoots what
/// the receiver's bandwidth estimate allows for long.
fn set_rate(ctx: &mut ff::codec::Context, kbps: u32) {
    let bps = kbps as i64 * 1000;
    unsafe {
        let p = ctx.as_mut_ptr();
        (*p).bit_rate = bps;
        (*p).rc_max_rate = bps;
        // libvpx is CBR only when min, max and target are equal (libvpxenc.c).
        (*p).rc_min_rate = bps;
        (*p).rc_buffer_size = (bps / 2) as i32;
    }
}

impl Encoder {
    pub fn open(settings: Settings) -> Result<Self, String> {
        let (enc, name) = open(&settings)?;
        Ok(Encoder { enc, settings, name, last_reopen: Instant::now(), force_key: false })
    }

    pub fn request_keyframe(&mut self) {
        self.force_key = true;
    }

    /// Follows `kbps` if the encoder can; libx264 and NVENC reconfigure on the next frame.
    pub fn follow_bitrate(&mut self, kbps: u32) {
        let kbps = clamp_kbps(kbps);
        let old = self.settings.bitrate_kbps;
        if kbps == old {
            return;
        }
        if LIVE_RATE.contains(&self.name) {
            set_rate(&mut self.enc, kbps);
            self.settings.bitrate_kbps = kbps;
            return;
        }
        let step = (kbps as f64 - old as f64).abs() / old as f64;
        if step < REOPEN_STEP || self.last_reopen.elapsed() < REOPEN_INTERVAL {
            return;
        }
        let mut next = self.settings;
        next.bitrate_kbps = kbps;
        self.last_reopen = Instant::now();
        match open(&next) {
            Ok((enc, _)) => {
                self.enc = enc;
                self.settings = next;
                self.force_key = true;
                crate::info(&format!("{} reopened at {kbps} kbps", self.name));
            }
            Err(e) => crate::warn(&format!("reopen at {kbps} kbps: {e}")),
        }
    }

    /// Encodes one frame and hands each finished packet to `out` as
    /// (pts in microseconds, keyframe, bytes).
    pub fn encode(
        &mut self,
        frame: &mut ff::frame::Video,
        mut out: impl FnMut(u64, bool, &[u8]),
    ) -> Result<(), String> {
        let kind = if std::mem::take(&mut self.force_key) {
            ff::picture::Type::I
        } else {
            ff::picture::Type::None
        };
        frame.set_kind(kind);
        self.enc.send_frame(frame).map_err(|e| format!("{}: send_frame: {e}", self.name))?;
        let mut pkt = ff::Packet::empty();
        while self.enc.receive_packet(&mut pkt).is_ok() {
            let Some(data) = pkt.data() else { continue };
            let pts = pkt.pts().unwrap_or(0).max(0) as u64;
            let us = pts * 1_000_000 * self.settings.fps_den as u64 / self.settings.fps_num as u64;
            out(us, pkt.is_key(), data);
        }
        Ok(())
    }
}

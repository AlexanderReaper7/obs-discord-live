//! The plugin's own video encoder, through libavcodec.
//!
//! libobs has no call that asks one of its encoders for a keyframe, so the
//! output takes raw frames and encodes them here, where a PLI from the receiver
//! can force an IDR on the next frame.

use std::time::{Duration, Instant};

use ffmpeg_next as ff;

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

/// Encoders other than libx264 take a new bitrate only by reopening, which costs
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

/// The libavcodec encoder and private options for a codec. Every set is
/// low-delay: no B-frames, no lookahead, a keyframe only on request or every
/// ten seconds.
fn choice(codec: Codec) -> (&'static str, Vec<(&'static str, String)>) {
    match codec {
        Codec::H264 => (
            "libx264",
            vec![
                ("preset", "veryfast".into()),
                ("tune", "zerolatency".into()),
                // WebRTC offers H.264 as 42e01f, constrained baseline level 3.1.
                // ODL_X264_PROFILE=high trades that for about 10 % less bitrate.
                ("profile", std::env::var("ODL_X264_PROFILE").unwrap_or("baseline".into())),
                ("forced-idr", "1".into()),
                // A scene-cut IDR is a bitrate spike nobody asked for; the
                // receiver asks with a PLI when it needs one.
                ("x264-params", "scenecut=0".into()),
            ],
        ),
        Codec::Av1 => (
            "libsvtav1",
            vec![
                ("preset", "10".into()),
                // Without enable-force-key-frames, SVT-AV1 ignores a frame's
                // key picture type unless every frame is one (libsvtav1.c).
                ("svtav1-params", "rtc=1:pred-struct=1:enable-force-key-frames=1".into()),
            ],
        ),
        Codec::Vp8 => (
            "libvpx",
            vec![
                ("deadline", "realtime".into()),
                ("cpu-used", "8".into()),
                ("lag-in-frames", "0".into()),
                // A keyframe may take nine frames' worth of bits, as in libwebrtc.
                ("max-intra-rate", "900".into()),
            ],
        ),
        Codec::Vp9 => (
            "libvpx-vp9",
            vec![
                ("deadline", "realtime".into()),
                ("cpu-used", "8".into()),
                ("lag-in-frames", "0".into()),
                // A keyframe may take nine frames' worth of bits, as in libwebrtc.
                ("max-intra-rate", "900".into()),
                ("row-mt", "1".into()),
                // Cyclic refresh, libwebrtc's VP9 setting for real-time.
                ("aq-mode", "3".into()),
            ],
        ),
    }
}

fn open(s: &Settings) -> Result<(ff::encoder::Video, &'static str), String> {
    let (name, options) = choice(s.codec);
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
    ctx.set_gop(10 * s.fps_num / s.fps_den.max(1));
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
    let enc = ctx.open_with(dict).map_err(|e| format!("{name}: {e}"))?;
    Ok((enc, name))
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

    /// Follows `kbps` if the encoder can; libx264 reconfigures on the next frame.
    pub fn follow_bitrate(&mut self, kbps: u32) {
        let kbps = kbps.clamp(MIN_KBPS, MAX_KBPS);
        let old = self.settings.bitrate_kbps;
        if kbps == old {
            return;
        }
        if self.name == "libx264" {
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

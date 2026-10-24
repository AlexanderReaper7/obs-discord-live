//! Test client for the plugin: starts a stream, saves it, and exercises the feedback path.
//!
//! odl-client CODEC KBPS SECONDS OUT [--keyframe-at FRAME]... [--bitrate-at FRAME:KBPS]...
//!
//! H264 is saved as an Annex B elementary stream, the others as IVF. Prints every
//! keyframe and one line per second of video with the measured bitrate.
//!
//! Connects to `$OBS_DISCORD_LIVE_ADDR`, else the plugin's default socket.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use odl_proto::{ClientMsg, ServerMsg, Stream};

struct Args {
    codec: String,
    kbps: u32,
    seconds: u64,
    out: String,
    keyframe_at: Vec<u64>,
    bitrate_at: Vec<(u64, u32)>,
}

fn parse() -> Result<Args, String> {
    let usage = "usage: odl-client CODEC KBPS SECONDS OUT [--keyframe-at FRAME]... [--bitrate-at FRAME:KBPS]...";
    let mut it = std::env::args().skip(1);
    let mut next = |what: &str| it.next().ok_or(format!("missing {what}\n{usage}"));
    let codec = next("CODEC")?;
    let kbps = next("KBPS")?.parse().map_err(|_| "KBPS is not a number")?;
    let seconds = next("SECONDS")?.parse().map_err(|_| "SECONDS is not a number")?;
    let out = next("OUT")?;
    let mut a = Args { codec, kbps, seconds, out, keyframe_at: vec![], bitrate_at: vec![] };
    let rest: Vec<String> = std::env::args().skip(5).collect();
    let mut i = 0;
    while i < rest.len() {
        let v = rest.get(i + 1).ok_or(format!("{} needs a value", rest[i]))?;
        match rest[i].as_str() {
            "--keyframe-at" => a.keyframe_at.push(v.parse().map_err(|_| "bad FRAME")?),
            "--bitrate-at" => {
                let (f, k) = v.split_once(':').ok_or("--bitrate-at takes FRAME:KBPS")?;
                a.bitrate_at.push((
                    f.parse().map_err(|_| "bad FRAME")?,
                    k.parse().map_err(|_| "bad KBPS")?,
                ));
            }
            other => return Err(format!("unknown option {other}\n{usage}")),
        }
        i += 2;
    }
    Ok(a)
}

fn ivf_header(fourcc: &[u8; 4], w: u32, h: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity(32);
    v.extend_from_slice(b"DKIF");
    v.extend_from_slice(&0u16.to_le_bytes());
    v.extend_from_slice(&32u16.to_le_bytes());
    v.extend_from_slice(fourcc);
    v.extend_from_slice(&(w as u16).to_le_bytes());
    v.extend_from_slice(&(h as u16).to_le_bytes());
    // Time base 1/1000000, matching pts_us.
    v.extend_from_slice(&1_000_000u32.to_le_bytes());
    v.extend_from_slice(&1u32.to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes());
    v
}

fn run(a: Args) -> Result<(), String> {
    let addr = odl_proto::addr();
    let mut sock = Stream::connect(&addr).map_err(|e| format!("{addr}: {e}"))?;
    let mut writer = sock.try_clone().map_err(|e| e.to_string())?;
    let send = |w: &mut Stream, m: ClientMsg| {
        println!("-> {m:?}");
        odl_proto::write(w, &m.encode()).map_err(|e| e.to_string())
    };
    send(&mut writer, ClientMsg::Start { codec: a.codec.clone(), bitrate_kbps: a.kbps })?;
    sock.set_read_timeout(Some(Duration::from_secs(10))).map_err(|e| e.to_string())?;

    let mut out = BufWriter::new(File::create(&a.out).map_err(|e| format!("{}: {e}", a.out))?);
    let mut ivf = false;
    let mut fps = 30.0;
    let mut frames: u64 = 0;
    let mut window_bytes = 0usize;
    let mut window_start = 0u64;
    let start = Instant::now();
    loop {
        let msg = match odl_proto::read_server(&mut sock) {
            Ok(Some(m)) => m,
            Ok(None) => return Err("the plugin closed the connection".into()),
            Err(e) => return Err(format!("read: {e}")),
        };
        match msg {
            ServerMsg::Config { codec, encoder, width, height, fps_num, fps_den } => {
                println!("<- Config {codec} by {encoder}, {width}x{height} at {fps_num}/{fps_den}");
                fps = fps_num as f64 / fps_den as f64;
                let fourcc = match codec.as_str() {
                    "AV1" => Some(b"AV01"),
                    "VP8" => Some(b"VP80"),
                    "VP9" => Some(b"VP90"),
                    _ => None,
                };
                if let Some(f) = fourcc {
                    out.write_all(&ivf_header(f, width, height)).map_err(|e| e.to_string())?;
                    ivf = true;
                }
            }
            ServerMsg::Error { message } => return Err(format!("plugin: {message}")),
            ServerMsg::Frame { pts_us, keyframe, data } => {
                if ivf {
                    out.write_all(&(data.len() as u32).to_le_bytes()).map_err(|e| e.to_string())?;
                    out.write_all(&pts_us.to_le_bytes()).map_err(|e| e.to_string())?;
                }
                out.write_all(&data).map_err(|e| e.to_string())?;
                if keyframe {
                    println!("   frame {frames}: keyframe, {} bytes, pts {pts_us} us", data.len());
                }
                window_bytes += data.len();
                frames += 1;
                if frames - window_start >= fps.round() as u64 {
                    let secs = (frames - window_start) as f64 / fps;
                    println!(
                        "   frames {window_start}..{frames}: {:.0} kbps",
                        window_bytes as f64 * 8.0 / secs / 1000.0
                    );
                    window_start = frames;
                    window_bytes = 0;
                }
                if a.keyframe_at.contains(&frames) {
                    send(&mut writer, ClientMsg::KeyframeRequest)?;
                }
                for &(_, kbps) in a.bitrate_at.iter().filter(|(f, _)| *f == frames) {
                    send(&mut writer, ClientMsg::SetBitrate { kbps })?;
                }
                if frames as f64 >= a.seconds as f64 * fps {
                    break;
                }
            }
        }
    }
    send(&mut writer, ClientMsg::Stop)?;
    out.flush().map_err(|e| e.to_string())?;
    println!("{frames} frames in {:.1} s wall clock", start.elapsed().as_secs_f64());
    Ok(())
}

fn main() -> ExitCode {
    match parse().and_then(run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("odl-client: {e}");
            ExitCode::FAILURE
        }
    }
}

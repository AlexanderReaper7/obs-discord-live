# obs-discord-live

Streams OBS's program view into a Discord Go Live without Discord encoding it a second time. Two parts:

- `plugin/` is an OBS output in Rust. It takes raw I420 frames at the canvas's output size and rate, encodes them through OBS's own libavcodec, and serves them on `$XDG_RUNTIME_DIR/obs-discord-live.sock`. The client can ask for a keyframe (a PLI from a viewer) and set the bitrate (the sender's bandwidth estimate). The output has no button; it runs while a client holds a session.
- The Vesktop patch (not written yet) replaces the Go Live sender's encoded frames with the plugin's through an encoded transform and forwards PLIs and `targetBitrate` back.

`proto/` is the wire format, documented in `proto/src/lib.rs`. `client/` is a test client that stands in for Vesktop.

## Why the plugin encodes

libobs 32.2 has no call that asks an encoder for a keyframe, and its WHIP output ignores PLIs. A viewer who joins late or loses a packet would wait for the next periodic keyframe. Owning the encoder makes a keyframe request take effect on the next frame.

## Encoders

Picked by the codec the client names, which is the one Discord negotiated:

| Codec | Encoder | Bitrate change |
| --- | --- | --- |
| H264 | libx264, veryfast, zerolatency, constrained baseline (`42e01f`), no scene cuts | live, next frame |
| AV1 | libsvtav1 preset 10, `rtc=1`, low-delay | reopen plus keyframe, at most every 2 s and only for a change over 15 % |
| VP8 | libvpx realtime, CBR | reopen, same limits |
| VP9 | libvpx-vp9 realtime, CBR, cyclic refresh | reopen, same limits |

`ODL_X264_PROFILE=high` in OBS's environment trades the baseline profile for about 10 % less bitrate, if the receivers accept it.

## Build and install for development

```sh
nix develop -c cargo build --release
cp target/release/libobs_discord_live.so ~/.config/obs-studio/plugins/obs-discord-live/bin/64bit/obs-discord-live.so
```

OBS loads it at start. `.cargo/config.toml` builds for `znver5`, the value nixcfg's `refresh-cpu` wrote for strix. LTO is off for build time; `lto = "fat"` in `Cargo.toml` turns it on.

## Test

With OBS running:

```sh
target/release/odl-client H264 2500 10 test-out/h.264 --keyframe-at 90 --bitrate-at 150:800
```

## Verification

2026-10-24, OBS 32.2.2, Adversary profile at 1280x720 30 fps, the Looking Glass source live. `odl-client` for 8 s per codec at 2500 kbps, a keyframe request after frames 90 and 180, 800 kbps from frame 120. ffmpeg decoded all four files without an error.

| Codec | Keyframes at | kbps before the drop | kbps after |
| --- | --- | --- | --- |
| H264 | 0, 90, 180 | 2212 to 2944 | 653 to 903 |
| AV1 | 0, 90, 120, 180 | 2237 to 3186 | 763 to 928 |
| VP8 | 0, 90, 120, 180 | 2398 to 3125 | 1618 to 2196 |
| VP9 | 0, 90, 120, 180 | 2285 to 2931 | 788 to 1486 |

VP8 does not get down to 800 kbps on this content; the cause is not measured yet. Zero frames dropped between OBS and the encoder in every run. Quitting OBS removes the socket and OBS reports 0 memory leaks.

# obs-discord-live

Streams OBS's program view into a Discord Go Live without Discord encoding it a second time. Two parts:

- `plugin/` is an OBS output in Rust. It takes raw I420 frames at the canvas's output size and rate, encodes them through OBS's own libavcodec, and serves them on `$XDG_RUNTIME_DIR/obs-discord-live.sock`. The client can ask for a keyframe (a PLI from a viewer) and set the bitrate (the sender's bandwidth estimate). The output has no button; it runs while a client holds a session.
- The Vesktop patch (not written yet) puts the plugin's frames into the Go Live sender inside Discord's encryption worker, and forwards PLIs and `targetBitrate` back. Design below.

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

## What Discord's Go Live sender is

Measured 2026-10-24 in Vesktop 1.6.7 (Electron 43, Chrome 150) with `phase0/probe.js` over CDP, during a Go Live with one viewer:

- Discord negotiates H.264 `42e01f` (constrained baseline 3.1, packetization mode 1), RTX, `nack`, `nack pli`, `ccm fir`, transport-cc and REMB. Chromium encodes it with OpenH264, one layer (`L1T1`).
- Both connections have `encodedInsertableStreams: true`, and every sender carries an `RTCRtpScriptTransform` into a worker named `encryption-worker`: Discord's end-to-end encryption. A sender takes one transform, so the patch cannot add its own.
- In that worker, `RTCEncodedVideoFrame.data` has a setter. `RTCRtpScriptTransformer` has `sendKeyFrameRequest` but no `generateKeyFrame`.
- Outbound stats carry `targetBitrate` (3.5 Mbps with a viewer), `pliCount`, `firCount`, `nackCount` and `keyFramesEncoded`.
- With no viewer, Discord's transform passes no frames on: Chromium counts frames encoded and sent while the connection moves 0 RTP packets.
- Chromium delivered 25 fps from a `getDisplayMedia` that asked for 1920x1080 at 30.
- `MediaStreamTrackGenerator` exists in the page.

## Design of the Vesktop side

Decided 2026-10-24 from the measurements above.

- **The swap happens in Discord's encryption worker, before encryption.** A prelude wraps the worker's `onrtctransform` so that, for the video encrypt operation, each frame's `data` becomes the plugin's access unit before Discord's own code sees it. Discord's encryption, packetization and congestion control stay as they are.
- **Chromium's frame clock follows the plugin's.** H.264 P-frames reference every frame before them, so the swap must be one plugin frame per Chromium frame, and Chromium captures at its own rate (25 fps measured). The Go Live track is therefore a `MediaStreamTrackGenerator` that emits one black 1280x720 `VideoFrame` per plugin frame. OpenH264 encodes black almost for free and never needs to drop a frame for bitrate.
- **A lost frame costs a keyframe.** If Chromium drops a generated frame anyway, the plugin's frame for it never goes out and the next ones reference it; the worker then asks the plugin for a keyframe.
- **Feedback.** A rising `pliCount` or `firCount`, or OpenH264 producing a keyframe nobody asked it for, becomes `KeyframeRequest`. `targetBitrate` becomes `SetBitrate`, polled every second.
- **The plugin socket stays in Vesktop's main process**, which can open Unix sockets; frames reach the worker over IPC and a `MessagePort`.
- **WebRTC uses only the default-route interface.** strix gathers 34 local candidates (Docker bridges, `svmbr0`, Tailscale, Mullvad), all leaving through one public IP on different ports. One Go Live attempt nominated the Docker bridge `172.23.0.1` and never finished DTLS: 22 packets out, 0 bytes back. `--force-webrtc-ip-handling-policy` had no effect in Electron 43, so the patch calls `webContents.setWebRTCIPHandlingPolicy("default_public_interface_only")`.

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

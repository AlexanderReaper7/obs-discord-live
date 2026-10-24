# obs-discord-live

Streams OBS's program view into a Discord Go Live without Discord encoding it a second time. Two parts:

- `plugin/` is an OBS output in Rust. It takes raw I420 frames at the canvas's output size and rate, encodes them through OBS's own libavcodec, and serves them on `$XDG_RUNTIME_DIR/obs-discord-live.sock`. The client can ask for a keyframe (a PLI from a viewer) and set the bitrate (the sender's bandwidth estimate). The output has no button; it runs while a client holds a session.
- `vesktop/obs-discord-live.patch`, against Vesktop 1.6.7, adds OBS as a Go Live source. It puts the plugin's frames into the Go Live sender ahead of Discord's encryption, and forwards keyframe requests and `targetBitrate` back. Design below.

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

Decided 2026-10-24 from the measurements above and `phase0/loopback.js`.

- **OBS is a source in the picker.** When the plugin's socket exists, Vesktop's picker shows an OBS card, and on Wayland a Portal card for the usual screen capture. Picking OBS hands Electron's display media handler Vesktop's own frame, so no portal dialog appears, and the page swaps that track for a `MediaStreamTrackGenerator`.
- **The swap happens in the page, before Discord's encryption.** Every sender gets an `RTCRtpScriptTransform` into Discord's `encryption-worker`, and a sender takes one transform. For a video sender while an OBS session runs, the patch calls `createEncodedStreams()` instead, pipes the frames through its swap, and posts the streams to the same worker as `{type: 1, operation: 0, readable, writable}`, the message the worker already handles for a transform. Discord's encryption, packetization and congestion control are untouched.
- **One black frame per plugin frame.** The generator gets a black I420 frame at the plugin's size for each access unit, so OpenH264 encodes almost nothing and the sender emits one encoded frame per plugin frame.
- **Frames pair in order, not by timestamp.** In the loopback test an encoded frame's RTP timestamp followed the wall clock of the `write()`, not `VideoFrame.timestamp`, with residuals up to 43 ms, and Chromium dropped 2 of 88 frames. The swap is a queue: each encoded frame takes the oldest access unit; a queued plugin keyframe discards everything older; an encoded frame with no access unit waiting is dropped. A queue more than 2 frames behind asks the plugin for a keyframe.
- **Feedback.** A keyframe from OpenH264 after the first (it makes one on a PLI) and a rising `pliCount` or `firCount` become `KeyframeRequest`. `targetBitrate` from the sender's stats becomes `SetBitrate` when it moves more than 5 %, polled every second.
- **The encoding's frame rate cap is raised to 60.** At Discord's 30, Chromium dropped about one generated frame a minute that arrived early; at 60 it dropped none in a minute. Each drop costs a keyframe.
- **The local preview decodes the plugin's frames.** Discord's preview plays the sender's track, which is black. A `VideoDecoder` feeds a second generator track, and a `<video>` handed the Go Live track gets that one; the `srcObject` getter still returns what Discord set.
- **The plugin socket stays in Vesktop's main process**, which can open Unix sockets; access units reach the page over IPC.
- **WebRTC uses only the default-route interface.** strix gathers 34 local candidates (Docker bridges, `svmbr0`, Tailscale, Mullvad), all leaving through one public IP on different ports. One Go Live attempt nominated the Docker bridge `172.23.0.1` and never finished DTLS: 22 packets out, 0 bytes back. `--force-webrtc-ip-handling-policy` had no effect in Electron 43, so the patch calls `webContents.setWebRTCIPHandlingPolicy("default_public_interface_only")` on the main window. That applies to voice calls too.

## Install with nix

The flake has `packages.x86_64-linux.default`, the plugin, and `packages.x86_64-linux.vesktop`, nixpkgs's Vesktop with the patch. For a NixOS config, `lib.plugin { pkgs; cpuArch = "znver5"; }` builds the plugin for a CPU, and `lib.vesktop { pkgs; }` the patched Vesktop:

```nix
programs.obs-studio.plugins = [ (inputs.obs-discord-live.lib.plugin { inherit pkgs; cpuArch = config.mine.cpu.arch; }) ];
environment.systemPackages = [ (inputs.obs-discord-live.lib.vesktop { inherit pkgs; }) ];
```

The patch is the one commit on top of Vesktop 1.6.7's source; it changes no dependency, so nixpkgs's pnpm hash still holds.

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

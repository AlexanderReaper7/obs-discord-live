# obs-discord-live

Streams OBS's program view into a Discord Go Live without Discord encoding it a second time. Two parts:

- `plugin/` is an OBS output in Rust, for Linux and Windows. It takes raw I420 frames at the canvas's output size and rate, encodes them through OBS's own libavcodec, and serves them on `$XDG_RUNTIME_DIR/obs-discord-live.sock` on Linux and TCP port 4457 on Windows. The listen address in its settings, or `OBS_DISCORD_LIVE_ADDR`, overrides either: `HOST:PORT` is TCP, anything else a socket path. The client can ask for a keyframe (a PLI from a viewer) and set the bitrate (the sender's bandwidth estimate). The output has no button; it runs while a client holds a session. Its settings are under Tools → Discord Go Live (below).
- `vesktop/obs-discord-live.patch`, against Vesktop 1.6.7, adds OBS as a Go Live source. It puts the plugin's frames into the Go Live sender ahead of Discord's encryption, and forwards keyframe requests and `targetBitrate` back. Design below.

`proto/` is the wire format, documented in `proto/src/lib.rs`. `client/` is a test client that stands in for Vesktop.

## Why the plugin encodes

libobs 32.2 has no call that asks an encoder for a keyframe, and its WHIP output ignores PLIs. A viewer who joins late or loses a packet would wait for the next periodic keyframe. Owning the encoder makes a keyframe request take effect on the next frame.

## Where OBS runs

Decided 2026-10-24: OBS runs in the `adversary` guest (stealth-vm), and Vesktop on the host connects to the plugin over `svmbr0`, `192.168.1.62:4457`. A Windows firewall rule lets only the host, `192.168.1.1`, in.

The first version ran OBS on the host with Looking Glass as the source. That path cost picture quality in two places:

- Looking Glass B7's D12 backend sends an HDR desktop as 10-bit UNORM of linear light (`HDR16to10`), scaled so SDR white is 1.0. Linear light in 10 bits has too few steps in the dark, which showed as grain, and everything above SDR white clips.
- The fix there is 16-bit float frames. A 3840x2160 RGBA16F frame is 66 MB, over the 62 MiB frame limit of the 128 MiB kvmfr device; 256 MiB needs a VM restart.

In the guest, OBS captures the display itself as 16-bit float and tonemaps HDR to SDR in its own shaders, at the full 3840x2160, then scales to the 1280x720 output with the area filter. Looking Glass stays the user's view of the guest and is no longer in the stream.

## HDR to SDR

Decided 2026-10-25: the guest's display capture carries OBS's HDR Tone Mapping filter, transform "SDR MaxRGB", input maximum 1000 nits, output maximum 150 nits. OBS's own tonemap, Reinhard with the profile's SDR white level, put Windows' SDR white (desktop and HUD white, about 100 nits) at 143 of 255 with the white level at 300, the value measured in the 3840x2160 capture. A lower white level brightens it but flattens highlights, since Reinhard compresses everything above it.

MaxRGB SDR is BT.2390's EETF: linear below a knee, then a roll-off that ends at the output maximum, which becomes 255. `phase0/tonemap.py` ports both tonemaps from OBS 33's shaders; it predicted 143 for Reinhard at 300 and 206 for this filter, and the screenshots measured 143 and 206. With this filter 1000 nits still lands below clipping. An output maximum of 120 gives 214 for SDR white and squeezes everything over 300 nits into 248 to 255.

| Gray level, nits | 5 | 20 | 50 | 100 | 200 | 500 | 1000 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Reinhard, white level 300 | 46 | 80 | 113 | 143 | 174 | 210 | 229 |
| MaxRGB SDR, 1000 to 150 | 62 | 110 | 161 | 206 | 236 | 253 | 255 |

## Encoders

Picked by the codec the client names, which is the one Discord negotiated. Discord negotiates H264, so in practice it is the H264 row:

| Codec | Encoder | Bitrate change |
| --- | --- | --- |
| H264 | libx264, veryfast, zerolatency, constrained baseline (`42e01f`), no scene cuts; or NVENC | live, next frame |
| AV1 | libsvtav1 preset 10, `rtc=1`, low-delay | reopen plus keyframe, at most every 2 s and only for a change over 15 % |
| VP8 | libvpx realtime, CBR | reopen, same limits |
| VP9 | libvpx-vp9 realtime, CBR, cyclic refresh | reopen, same limits |

## Settings

Tools → Discord Go Live in OBS opens OBS's own properties window for the plugin:

| Setting | Default | Applies |
| --- | --- | --- |
| H.264 encoder: x264 or NVENC | x264 | next Go Live |
| H.264 profile: constrained baseline, main, high | constrained baseline, what WebRTC offers | next Go Live |
| x264 preset, ultrafast to medium | veryfast | next Go Live |
| NVENC preset, p1 to p7 | p4 | next Go Live |
| Keyframe interval, 1 to 60 s | 10 s | next Go Live |
| Maximum bitrate, 100 to 50000 kbps | 50000 | at once |
| Listen address | empty: `OBS_DISCORD_LIVE_ADDR`, else the default above | at once; ends a running Go Live |

The window also shows where the plugin listens, whether a client is connected, and what it is encoding. The settings are saved to `plugin_config/obs-discord-live/settings.json` in OBS's config directory, the same for every profile.

Decided 2026-10-25: OBS has no settings page for an output, and a dock needs Qt in C++. The frontend API does open the properties window of any source, so the plugin registers a source type hidden from Add Source, keeps one private instance of it as the settings, and adds a Tools menu item that opens its window. The window applies nothing until OK. The settings replace the `ODL_X264_PROFILE` and `ODL_H264_ENCODER` variables of 2026-10-24. The low-delay options (zerolatency, no B-frames, no lookahead, no scene cuts) are not settings: the frame pairing in Vesktop depends on one encoded frame per input frame.

NVENC is not the default: in the guest on 2026-10-24, each NVENC probe hung the passed-through RTX 3080 (`nvlddmkm` event 153), and the second hang ended the running game with `DXGI_ERROR_DEVICE_HUNG`. The window warns about it. OBS's own NVENC test program is moved out of the guest's install for the same reason.

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
- **The encoding's frame rate cap is raised to 60.** At Discord's 30, Chromium dropped about one generated frame a minute that arrived early; at 60 it dropped none in a minute. Each drop costs a keyframe. Discord sets the cap again during a Go Live, so the patch checks it every second and raises it when it is lower. In the guest Go Live of 2026-10-25 it read 30 again some time after the start; set back to 60 by hand, it held for over 2 minutes, so the event that resets it is not known.
- **The local preview decodes the plugin's frames.** Discord's preview plays the sender's track, which is black. A `VideoDecoder` feeds a second generator track, and a `<video>` handed the Go Live track gets that one; the `srcObject` getter still returns what Discord set. Discord pauses its own preview of the user's stream while the window does not have focus: its React prop `paused` read `true` on that element with the window unfocused. A preview that stands still then is Discord's choice, and viewers still get frames.
- **The plugin socket stays in Vesktop's main process**, which can open Unix and TCP sockets; access units reach the page over IPC. With `OBS_DISCORD_LIVE_ADDR=HOST:PORT` it connects over TCP. The picker shows the OBS card when a connect to that address succeeds within 500 ms. The plugin takes the newest connection as its client, so that probe ends a running OBS session; opening the picker during one means a new Go Live anyway.
- **WebRTC uses only the default-route interface.** strix gathers 34 local candidates (Docker bridges, `svmbr0`, Tailscale, Mullvad), all leaving through one public IP on different ports. One Go Live attempt nominated the Docker bridge `172.23.0.1` and never finished DTLS: 22 packets out, 0 bytes back. `--force-webrtc-ip-handling-policy` had no effect in Electron 43, so the patch calls `webContents.setWebRTCIPHandlingPolicy("default_public_interface_only")` on the main window. That applies to voice calls too.

## Install with nix

The flake has `packages.x86_64-linux.default`, the plugin, `packages.x86_64-linux.vesktop`, nixpkgs's Vesktop with the patch, and `packages.x86_64-linux.windows`, the plugin as a DLL for OBS 33.0.0 on Windows x64. For a NixOS config, `lib.plugin { pkgs; cpuArch = "znver5"; }` builds the plugin for a CPU, and `lib.vesktop { pkgs; obsAddress = "192.168.1.62:4457"; }` the patched Vesktop, with `obsAddress` as its default `OBS_DISCORD_LIVE_ADDR` (omit it for OBS on the same machine):

```nix
programs.obs-studio.plugins = [ (inputs.obs-discord-live.lib.plugin { inherit pkgs; cpuArch = config.mine.cpu.arch; }) ];
environment.systemPackages = [ (inputs.obs-discord-live.lib.vesktop { inherit pkgs; obsAddress = "192.168.1.62:4457"; }) ];
```

The patch is the one commit on top of Vesktop 1.6.7's source; it changes no dependency, so nixpkgs's pnpm hash still holds.

## Install in a Windows guest

`nix build .#windows` cross-builds with mingw. It links `obs.dll` and `obs-frontend-api.dll` from the OBS 33.0.0 release and FFmpeg from the obs-deps build that release pins, so the DLL needs nothing OBS does not ship. A different OBS version needs those three inputs changed in `flake.nix`.

Copy `obs-plugins/obs-discord-live/bin/64bit/obs-discord-live.dll` from the result to `C:\ProgramData\obs-studio\plugins\obs-discord-live\bin\64bit\` in the guest, allow TCP 4457 from the host only, and start Vesktop with `OBS_DISCORD_LIVE_ADDR=<guest>:4457`.

The guest's OBS setup: profile at base 3840x2160, output 1280x720, 30 fps, area scaling, NV12 709 partial range; one scene with a display capture of the monitor (DXGI, no cursor) and the filter from "HDR to SDR" above. OBS starts at logon from a scheduled task. `obs-nvenc-test.exe` is moved out of `bin\64bit`, so OBS skips NVENC without probing the GPU (see Encoders); the `obs-nvenc` module itself stays, because a missing core module stops OBS 33 from loading every core module after it in its list.

## Build and install for development

```sh
nix develop -c cargo build --release
cp target/release/libobs_discord_live.so ~/.config/obs-studio/plugins/obs-discord-live/bin/64bit/obs-discord-live.so
```

OBS loads it at start. With the nix plugin also installed, OBS finds two modules of the same name, so remove this copy after testing. `.cargo/config.toml` builds for `znver5`, the value nixcfg's `refresh-cpu` wrote for strix. LTO is off for build time; `lto = "fat"` in `Cargo.toml` turns it on.

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

VP8 does not get down to 800 kbps on this content because its coarsest quantizer is not coarse enough at 1280x720. Measured with `phase0/vp8-rate.sh` 2026-10-25 with ffmpeg 9.0.1 and libvpx 1.17.0 on the 240 frames of the VP9 run decoded, with the plugin's libvpx options in CBR: targets of 800 and 1500 kbps both gave 1796 to 3198 kbps a second, the same stream, so libvpx was already at `rc_max_quantizer` 63. At 2500 it gave 2149 to 3201, at 4000 it gave 3647 to 4371. The same 800 kbps target at 640x360 gave 745 to 1126. Other `cpu-used` values, `screen-content-mode`, `noise-sensitivity` and `static-thresh` changed nothing below 1796; `drop-threshold 30` dropped frames and still sent 2710 to 3101 kbps per 30 frames. libwebrtc gets under the target by lowering the resolution. The plugin encodes at OBS's size, so a VP8 Go Live needs a lower output resolution in OBS for bandwidth estimates below about 2.5 Mbps. Zero frames dropped between OBS and the encoder in every run. Quitting OBS removes the socket and OBS reports 0 memory leaks.

2026-10-25, OBS 33.0.0 in the adversary guest, the znver5 DLL from `nix build .#windows`, the guest profile above, the Windows desktop on screen (Warframe not running). `odl-client H264 2500 8` over an SSH tunnel to the guest's port 4457, a keyframe request after frame 90, 800 kbps from frame 120: libx264 constrained baseline 1280x720 yuv420p BT.709 limited range, keyframes at 0 and 90, 614 kbps after the drop, and ffmpeg decoded it without an error. OBS reported 240 output frames, 0 skipped, 0.31 ms average render time.

2026-10-25, the settings window in OBS 32.2.2 on strix. A hand-written `settings.json` (profile high, keyframes every 2 s, 1500 kbps cap) gave a High profile stream with keyframes at frames 0, 60 and 120. A 300 kbps cap gave 207 kbps where the uncapped stream ran at 880, and a listen address of `127.0.0.1:14999` moved the socket to TCP. Tools → Discord Go Live opened the window with those values and the status lines; Defaults and OK wrote `{}` to the file, moved the socket back to `$XDG_RUNTIME_DIR` without an OBS restart, and the next stream was constrained baseline.

2026-10-25, a Go Live from OBS 33.0.0 in the adversary guest, Vesktop on strix connected to `192.168.1.62:4457`, started by the user with the OBS card. The guest log at 00:23:02 shows the client, H264 at 1000 kbps, then libx264 1280x720 30 fps. Over CDP the video sender was connected at 29.8 fps, 1280x720, encoder OpenH264, its track a `MediaStreamTrackGenerator`, and sent about 3.6 Mbps against a `targetBitrate` of 3500000. Black frames from OpenH264 come to a few kbps, so those bytes are the plugin's frames. The decoded preview showed the guest's OBS window once playing. The maxFramerate check in `pollStats` was added after this run and has not run in a live Go Live.

# TODO

- Stream from OBS in the adversary guest end to end: replace the guest's DLL with the libx264-default build, select the PG32UCWM in the display capture, start Vesktop with `OBS_DISCORD_LIVE_ADDR=192.168.1.62:4457`, and Go Live.
- Pick the guest profile's SDR white level from a measurement: OBS maps a pixel at that level to 191 of 255 (Reinhard), so measure HUD white in the 720p output at a few levels.
- Find out why obs-websocket on the guest, port 4455, timed out from the host while the plugin's port 4457 listened.
- Verify the decoded local preview and the 60 fps cap in a live Go Live (built 2026-10-24, not yet run).
- VP8 overshoots its target: 1618 to 2196 kbps at 800 (README, Verification). Cause not measured.
- Find out why NVENC hangs the guest's RTX 3080, before `h264_nvenc` is offered again. Not by probing it while a game runs.

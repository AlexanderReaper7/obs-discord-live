# TODO

- Check the HDR Tone Mapping filter (MaxRGB SDR, 1000 to 150 nits) on Warframe in HDR: HUD white, highlights, and OBS's render time and libx264's CPU use while the game runs. Measured so far only on the desktop.
- After the next Vesktop restart, check in a Go Live that the sender's `maxFramerate` stays at 60 (the patch raises it every second since 2026-10-25), and look for "maxFramerate was lowered" in the console to learn when Discord resets it.
- VP8 overshoots its target: 1618 to 2196 kbps at 800 (README, Verification). Cause not measured.
- Find out why NVENC hangs the guest's RTX 3080, before `h264_nvenc` is offered again. Not by probing it while a game runs.

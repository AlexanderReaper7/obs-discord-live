# TODO

- Go Live from OBS in the adversary guest: restart Vesktop, which nixcfg 5c90d13 points at `192.168.1.62:4457`, and Go Live. The host reaches that address since 2026-10-25.
- Check the HDR Tone Mapping filter (MaxRGB SDR, 1000 to 150 nits) on Warframe in HDR: HUD white, highlights, and OBS's render time and libx264's CPU use while the game runs. Measured so far only on the desktop.
- Verify the decoded local preview and the 60 fps cap in a live Go Live (built 2026-10-24, not yet run).
- VP8 overshoots its target: 1618 to 2196 kbps at 800 (README, Verification). Cause not measured.
- Find out why NVENC hangs the guest's RTX 3080, before `h264_nvenc` is offered again. Not by probing it while a game runs.

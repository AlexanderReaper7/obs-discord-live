# TODO

- Find out why NVENC hangs the guest's RTX 3080 while a game runs, before `h264_nvenc` is offered again. With no game running the probe passed on 2026-10-25. Finding the cause needs a probe with a game running, which the user has ruled out for now.
- Find which part of guest OBS makes Warframe load 2 to 3 times slower (README, Verification, 2026-10-25): the 4K HDR display capture, the HDR to SDR filter, or libx264. Needs loads in one Warframe session with each part turned off in turn.

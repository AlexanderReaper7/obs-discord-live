# TODO

- Find out why NVENC hangs the guest's RTX 3080 while a game runs, before `h264_nvenc` is offered again. With no game running the probe passed on 2026-10-25. Finding the cause needs a probe with a game running, which the user has ruled out for now.
- Confirm that an active Go Live makes Warframe load 2 to 3 times slower (README, Verification, 2026-10-25): time a load during a Go Live in the same session as one without. OBS capturing without a Go Live does not slow loads. If confirmed, the suspect is the output's copy of every frame from GPU to CPU for libx264.

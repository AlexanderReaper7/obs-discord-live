# TODO

- After the next Vesktop restart, check in a Go Live that the sender's `maxFramerate` stays at 60 (the patch raises it every second since 2026-10-25), and look for "maxFramerate was lowered" in the console to learn when Discord resets it.
- Find out why NVENC hangs the guest's RTX 3080, before `h264_nvenc` is offered again. Not by probing it while a game runs.

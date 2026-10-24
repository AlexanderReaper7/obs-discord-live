# TODO

- VP8 overshoots its target: 1618 to 2196 kbps at 800 (README, Verification). Cause not measured.
- Offer `h264_vaapi` on the iGPU as an H.264 encoder, if its latency and keyframe-on-request behaviour hold up.
- Verify the decoded local preview and the 60 fps cap in a live Go Live (built 2026-10-24, not yet run).

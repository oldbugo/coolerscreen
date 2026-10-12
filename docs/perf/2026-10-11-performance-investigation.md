# CoolerScreen 1.2.0 performance investigation (2026-10-11)

PC: Ryzen 7 9800X3D (8C/16T), RTX 5070 Ti, 32 GB. Bundled ffmpeg 6.1.1 essentials (gyan.dev): has cuda/nvdec/cuvid,
d3d11va, scale_cuda (lanczos), overlay_cuda, h264_nvenc, mjpeg encoder with slice threads. No newer build needed.
Caveat: 25-45 % background CPU during the runs (qemu VM, Wallpaper Engine, Firefox, another session's `find`), so absolute
times are inflated a bit; comparisons were run back to back.

Clips (copies in clips/): pb = pink-blue .ht (HEVC 1920x1080 25 fps 24.8 s), coast (H.264 2400x1080 25 fps 8.3 s),
rem = Wallpaper Engine gallery clip (H.264 2416x1086 18 fps 20 s), sakura4k60 = WE workshop 2970331102 (H.264 3840x2160 30 fps ~87 s),
zzz = Zenless Zone Zero WE clip (2416x1086 20 s).

## Scripts (scripts/)
- mon.js: counts/time every ffmpeg spawn, samples CPU (os.cpus) and GPU/NVDEC/NVENC (nvidia-smi -lms 250) per stage
- baseline.js: 1.2.0 functions exactly as the app calls them on opening a new clip and saving (out/baseline-*.log/json)
- exp_decode.js: loop-frame extraction CPU vs NVDEC vs D3D11VA, one-pass dual resolution (out/exp_decode.log)
- exp_estimate.js: raw-intermediate trial encodes, q sweep, 2-point model, 4-way parallel search, full-length exactness
- exp_loopcards.js: loop card previews and leader seeks, sequential vs parallel vs low-res vs NVENC
- after.js / open_seq.js / open_after.js: prototype of the proposed pipeline (proto/loopsearch2.js = loopsearch with one
  decode pass giving 64x36 + 192x108 and no leader seeks)
- click5.js: real 1.2.0 server (scratch library, fake device) replaying app.js requests while clicking 5 new clips 1.5 s apart

## Baseline 1.2.0 (ms)
| stage | pb | coast | rem | 4K 90 s |
|---|---|---|---|---|
| assets (thumb, frame, proxy) | 3232 (HEVC proxy) | 435 | 1608 | 522 |
| estimate #1 (initial 10 s selection) | 6588 | 3809 | 6071 | 5280 |
| loop: bytes/s sample | 443 | 435 | 680 | 528 |
| loop: findLoops | 3868 | 4607 | 4980 | 14361 |
|   - read frames (64x36 decode) | 888 | 402 | 1093 | 5119 |
|   - JS compare | 169 | 20 | 117 | 910 |
|   - leader re-check (16 ffmpeg seeks) | 3113 | 4490 | 4077 | 8632 |
| loop: 5 card previews (sequential, full 2400x1080 graph) | 3616 | 3203 | 3434 | 6610 |
| estimate #2 (picked loop) | 6837 | 7043 | 6347 | 4784 |
| save version (convert) | 7449 | 12183 | 8215 | 10453 |
Estimate = 7-9 ffmpeg runs, each re-seeking/decoding/scaling the same 2 s; convert repeats the whole search, makes a
full-res preview from the source again, encodes, then re-encodes at q-1 ("fill the space").

Click 5 new clips 1.5 s apart (click5-gap1500.log): last clip's meter after 51 s, loop cards after 60 s, picked-loop
meter after 65 s; 9 requests in flight at peak; 214 ffmpeg processes. Nothing is cancelled.

## Findings
- mjpeg size does not depend on -threads (bit-identical); swscale `threads=0` is bit-exact and cuts filter time 1.2-1.8x.
- Raw intermediate (decode+filter once, then encode many q) is bit-identical to the direct path ONLY if the raw input is
  tagged `-color_range pc`; without it the range is expanded twice and files are 11 % bigger.
- One mjpeg trial from raw: 130-180 ms (vs 500-1000 ms per 1.2.0 sample encode). Full-length encode from raw: 180-550 ms.
- Sample (2 s) extrapolation error vs real full-length size: <=1 % on all clips, so the 0.97 margin + fill re-encodes waste time.
- Exact full-length search from raw picked the same q as 1.2.0's final convert on all clips (21/22/11/12), same bytes.
- 2-point model (q=4,16 in log-log) hit the exact q in 3/4, off by 1 in 1/4; one parallel round q-1..q+1 makes it exact.
- NVDEC: 4K H.264 90 s loop frames 7.6 s CPU -> 3.85 s NVDEC+scale_cuda (and CPU freed); 1080p HEVC 1.05 -> 0.52 s;
  2.4K H.264 no gain (0.41 vs 0.47 s). D3D11VA with download was slower than CPU. GPU scale of a selection to 2400x1080:
  rem 1364 -> 825 ms, SSIM 0.995 vs CPU lanczos (not bit-exact).
- Proxies with NVDEC+NVENC: HEVC 25 s 2.6 -> 1.3 s; 4K 90 s 11.1 -> 4.0 s.
- Card previews: parallel + direct 320x144: pb 6.0 -> 1.4 s, rem 3.6 -> 1.2 s, 4K 5.1 -> 3.3-4.3 s (4K seek preroll bound).
- Leader re-check: 16 seeks 3.1-9.8 s -> 0 with 192x108 frames from the same decode pass (one-pass dual output costs ~0).
  Ranking changed slightly on 2 clips (pre-scale to 384x216 before area 64x36): validate before adopting.
- JS loop compare 20-910 ms (Rust SIMD estimate: <30 ms); not the bottleneck.

## Proposed pipeline, measured (prototype, ffmpeg sidecar, ordered: loops -> meter -> cards)
| action | 1.2.0 | proposed |
|---|---|---|
| open new clip, all ready (coast / rem / pb / 4K 90 s / zzz) | 19.0 / 21.5 / 21.3 / 31.6 / ~20 s | 3.7 / 4.3 / 9.2 / 9.5 / 2.4 s |
| ... meter shown | 3.8-6.6 s (initial) then +5-7 s | 2.6 / 2.6 / 6.3 / 6.2 / 1.8 s |
| click 5 clips quickly, last clip ready | 65 s | = one open (2.4-9.5 s) with cancellation |
| meter after a tuning change | 4.1-7.8 s | 1.9-3.0 s exact (first model value ~1-1.5 s) |
| save as version (meter already computed) | 7.4-12.2 s | 0.4-1.3 s |
| save as version (cold) | 7.4-12.2 s | 1.5-2.7 s |
| HEVC / 4K proxy | 2.6 / 11.1 s | 1.3 / 4.0 s |

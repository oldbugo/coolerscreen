# A4: why storing Embedded takes so long, and what was done

Written by stream A, 2026-10-19. User report: storing Embedded "takes a long time every time".

## Where the time goes (measured, not guessed)

From the user's `device.log` (2026-10-19, 4.7.1, the 10.6 MB Evangelion loop), start of the job to live resuming, about 220 s:

| step | time |
|---|---|
| live steps aside, open the screen, info read | about 1 s |
| size command `0x13` answered `{"key":19,"hex":"00"}` | 1.4 s |
| **streaming 10,610,433 bytes** (the 0x13 upload) | **209.9 s (50.5 KB/s)** |
| wait, restart (`0x01`), reopen, final info check, live resumes | about 8 s |

So **96 % of a store is the screen writing its flash.** 50 KB/s is the same rate the vendor tool and the 2.0 app got (phase 0:
51.7 KB/s; "the screen's flash is the limit"); the USB side is not the bottleneck and bigger or fewer writes do not help (the chunk size
is 40 KB, the screen takes bytes only as fast as it can write them). The closing `{"error":"No reply from screen"}` is normal (the
screen does not answer after a long upload; the code already treats it as a warning, and then restarts the screen).

Before the upload (`cargo test --release --lib time_the_steps_before_the_upload -- --ignored --nocapture`, default speed-1 effect of
180 frames, 10.6 MB budget): recording 0.5 to 1.3 s, conversion with the quality search 1.9 to 5.2 s. Plain clips skip the recording.
**Nothing to cut there** (at most 7 s of 220 s). The conversion step already reuses everything it can: a version with the same inputs
(settings, overlay, effect key, notch/fit key) is found by `create_version` and reused, `run_apply` reuses an existing base when
radius, fit, overlay and effect key match, effect recordings are cached by key, and the media engine keeps its raw-frame cache.

Upload time is simply file size: 5 MB takes 100 s, 10.6 MB takes 210 s. "Best that fits" always fills the flash (an aurora loop is
9.7 MB at quality 96, a retro loop 10.5 MB at quality 48). A recommendation for the PM, **not built** (it changes what the user gets):
a setting such as "Embedded size: best quality / balanced (about 6 MB) / quick to store (about 3 MB)" that caps the budget given to the
quality search; the loop would look a little softer and store in 1 to 2 minutes. The cap would be passed as `budget_bytes`; nothing
else changes, and the default would stay "best quality" (3.0 behaviour).

## Built: do not send what the screen already holds

`send_version` now first checks whether the clip about to be stored is exactly what the screen holds, and if so finishes at once with
"Already on the screen" (the Store as Embedded window says "Already on the screen. Nothing was sent."; job result `alreadyOnScreen`;
`device.log` says `send skipped`). All of these must hold, otherwise it sends as before:

1. the loop slot record is complete (not flagged `incomplete`) and has the same size;
2. same content: the slot now records a SHA-1 of the stored file (`hash`, new field in `slots.standby`, kept by the settings
   sanitiser). An older record without a hash counts only when it names this very version;
3. no "screen changed outside the app" note is open (C2);
4. a baseline exists (the free space after our last store, C2's `screenBaseline`), and a **fresh read-only info check of the screen**
   (`DeviceService::read_info`, a single `0x06`, never a restart) reports the same free space within C2's tolerance (64 blocks).
   If the read fails: send;
5. no boot splash needs writing in the same job.

When it skips, the slot is rewritten to name this version (identical bytes can come from another version of the same clip), the theme
base and the live layer follow as after a real store (`after_base_sent`), and the baseline is refreshed from the check just made.
Tests (`an_identical_clip_is_not_sent_again_but_anything_doubtful_is`): first store sends and records the hash; the same clip again
finishes at once; a different free space sends; a changed baseline sends then skips again; an open outside-change note, another size,
other content, an incomplete record and a missing baseline each say "send". Also, the older tests that re-store identical fake clips
exposed one bug that is fixed (the slot must follow the version on a skip).

Limits: it recognises the same bytes only. Re-converting the same inputs usually gives the same version (reused, so the same file),
but a new encode of identical settings may differ in bytes (different ffmpeg threads) and is then sent. Nothing is cached about the
screen between sessions beyond the baseline, which is the point of the safety check.

## Investigated: can live keep showing during the 3.5 minute send?

**No safe way found; not built, no experiment proposed that risks the screen.** What the code and the logs say:

- One program can hold the screen (the WinUSB interface claim; `holder.rs`, "a connect in progress counts as holding it"). A second
  handle, even in the same process, cannot be opened. So "live in a second connection" is out.
- Both an upload and a live stream are raw byte streams on the same bulk endpoint, interpreted by the screen's state: after `0x13 <size>`
  the next `size` bytes are the file (no per-chunk framing, no acknowledgements; one answer at the end that never arrives); in live
  mode (`0x11`) the bytes are length-prefixed JPEG frames with a checksum, ended by `ff d9 ff d9`. There is no documented way to
  interleave. Putting a frame between two file chunks would write the frame into the loop file (a broken clip, "oversize files make the
  screen show garbage" territory) and the frame would not be shown.
- Even if the firmware multiplexed them, the pipe is flash-limited at 50 KB/s while a live frame is 40 to 130 KB: a frame would take
  1 to 3 s. Live frames reach the screen about 30 times faster because they go to the display, not the flash.
- Pausing live is therefore a necessity, not a choice; the app already steps aside, resumes 1.2 s after, and hands over to the stored
  loop when live stops.

Safe options that send nothing new, in order of value: (1) skip identical sends (built); (2) a smaller Embedded budget (above, to decide);
(3) keep the existing distinction: "Use this theme" with Live on changes the live picture at once and sends nothing, so the long store
is only needed when the screen-only face should change.

**Observation for the PM (no new command, costs nothing):** during the next normal store, look at the panel at the start, after
about 1 minute and near the end, and note what it shows (old loop playing, frozen picture, black, or garbage). This tells us whether
the old loop keeps playing while the new one is written; if it does, a short note in the Store window ("The screen keeps playing while
this is written; live pauses") is enough, and if it blanks, the window should say so. I would not try to send live data during an
upload on hardware: the failure mode is a corrupt loop file.

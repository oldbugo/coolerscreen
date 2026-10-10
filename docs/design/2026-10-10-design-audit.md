# CoolerScreen design audit and redesign spec

Date: 2026-10-10. Scope: the desktop UI in `public/` (index.html, style.css, app.js) and the data it reads from
`src/` (library.js, gallery.js, server.js, slots.js, settings.js, convert.js). Reviewed by running the app in browser
mode (`COOLERSCREEN_FAKE_DEVICE=1`, port 8799) at the Electron window size 1280x860 with the real library (5 items),
plus a read of the code. No device operations were triggered.

Audience for this document: whoever implements the next UI iteration. Everything in section 4 onward is meant to be
buildable without guessing; open questions are listed explicitly in section 9.

---

## 1. Summary

The app works, but it reads as a stack of developer panels: six equally weighted cards on one long scrolling page
(Import, Screen slots, Gallery, Notch mask, Advanced, plus a modal). The one thing users come to do (pick a clip, make
it look right, put it on the screen) is spread across three of them, and the conversion step is forced on the user at
import time, with no visual preview of the result. Device concepts (slots, intro/standby, WinUSB, budgets, q values) leak
into the main view.

The redesign below turns it into a two-pane app: **Library** of originals on the left, and a **Detail** pane on the right
with a large screen-shaped preview, a **Tune** panel, and a **Versions** list. A persistent top bar shows the screen
status and **On screen now**. A single **Activity** strip shows conversion queue and the 3-minute upload. Everything rare
(notch mask, gallery folder, intro clip, driver restore) moves into a **Settings** drawer.

---

## 2. What it looks like today (observed)

- Long page at 1280x860: Import (drop zone + paragraph) and Screen slots take the whole first screen; the gallery
  starts below the fold line; the notch mask and Advanced sections need two more scrolls.
- Gallery card: thumbnail, name, 4 grey badges ("10 fps", "quality 1 (auto)", "8.91 MB", "10 s"), then 5 buttons
  (Send to screen, Convert settings..., Rename, Use as mask background, Delete). Selection (blue border) only means "this
  is the notch-mask background".
- Convert dialog: title, sliders for Start / Loop length / FPS, Quality select, "Space for loop: 9.48 MB",
  "Estimating size..." which turns into a green-bordered sentence after ~5-8 s. No picture of any kind.
- Click on a thumbnail: full-window video player of the converted preview; click outside to close.
- Screen status in header reads "Screen: not checked" until the user presses "Check screen".

---

## 3. Audit findings

Severity: **H** = blocks or misleads the main task, **M** = friction or confusion, **L** = polish.

### 3.1 Information architecture and primary flow

| # | Sev | Finding | Where |
|---|-----|---------|-------|
| IA1 | H | The primary flow (import, tune, choose, send) is split across Import card, a modal, and the Gallery card; slots and mask sit between them. Nothing on screen says "you are here, do this next". | `public/index.html:20-103` |
| IA2 | H | Import immediately opens the Convert dialog (`openDialog(j.item)`), forcing decisions before the user has even seen the clip. This is exactly what the user wants removed. | `public/app.js:276` |
| IA3 | H | One conversion per item. Re-converting overwrites `device.mp4`, preview and thumb, so there is no way to compare or keep alternatives. | `src/library.js:10, 330-356`; `src/convert.js:211-251` |
| IA4 | H | The tuning dialog has no visual preview: no picture of the trim, the crop, the notch mask or the quality. Feedback is one sentence of numbers that arrives seconds later. | `public/index.html:83-98`; `public/app.js:197-224` |
| IA5 | M | "What is on the screen now" lives only in the Slots card as "Last sent: <name>" text; gallery cards never show it. It is stored by name only, so it cannot be linked back to an item. | `public/app.js:46-69`; `src/settings.js:72-76` |
| IA6 | M | The notch mask is a global setting edited in its own card, saved separately, and then needs a manual "Re-convert"/"Re-convert all". Its preview background is chosen with a per-card "Use as mask background" button. Three steps and a hidden coupling. | `public/index.html:42-69`; `public/app.js:136-137, 297-307, 381-388` |
| IA7 | M | Intro/Loop slot model is exposed by default (Screen slots card, "Clear intro slot" / "Clear loop slot" danger buttons) although "Send to screen" already handles it. Most users only ever need "the loop". | `public/index.html:28-33` |
| IA8 | M | Setup appears twice ("Set up screen (one-time)" in header and in Advanced), and stays prominent after setup is done. "Check screen" is a manual action; status is "not checked" on every start. | `public/index.html:16-17, 74`; `public/app.js:310-322` |
| IA9 | L | "Advanced" checkbox changes controls inside every gallery card and the convert dialog (slot selector), which is surprising action at a distance. | `public/app.js:110-113, 238-239, 416` |

### 3.2 States, feedback and progress

| # | Sev | Finding | Where |
|---|-----|---------|-------|
| S1 | H | The ~3 min upload's progress is a 22 px bar appended at the bottom of one gallery card, below the buttons. It is easy to miss, has no time remaining, and is rebuilt (lost) whenever `renderGallery()` runs (e.g. toggling Advanced, or the `loadSlots().then(renderGallery)` after a clear). | `public/app.js:107, 147, 160-171, 416` |
| S2 | H | Six separate progress bars (import, dialog, per card, per slot, setup, reconnect) and no single place that shows "something is running / waiting". The queue (`gate.js`) produces good "Waiting for ..." messages but they only appear inside whichever bar started the job. | `public/index.html:25, 37-38, 79, 96`; `public/app.js:26, 56` |
| S3 | M | No "keep the app open" guidance during upload, and nothing stops the user closing the window mid-upload. | `public/app.js:160-171` |
| S4 | M | Completion is a 6 s toast ("Upload finished. The screen may restart by itself...") and the bar text "Sent". There is no persistent success state and "On screen now" does not visibly change. | `public/app.js:167-168`; `src/server.js:172` |
| S5 | M | While an upload runs, opening Convert settings yields a red error box "The screen is busy with an upload; size estimates are paused..." - a normal state presented as an error. | `src/server.js:158`; `public/app.js:223` |
| S6 | M | Item that was never converted shows "Not converted yet" inside the thumbnail area (black box, no image) and a "Set up & convert..." button; "Set up" collides with driver setup wording. | `public/app.js:96, 127` |
| S7 | M | Errors are raw strings (e.g. `ffmpeg failed: ... | ...`) in toasts and in an `error: <message>` badge on the card. No next step is offered except for failed-USB. | `public/app.js:105, 223, 258, 277`; `src/convert.js:33` |
| S8 | L | Empty state is a one-line hint below an empty grid; the large Import card above already says the same. | `public/index.html:39` |

### 3.3 Copy and terminology (non-programmer audience)

| # | Sev | Finding | Where |
|---|-----|---------|-------|
| C1 | M | Jargon in the main view: "fps", "q=", "quality 1 (auto)" (reads as broken), "standby", "slot", "budget", "2400x1080", "MJPEG", "WinUSB", "COM-port". | `public/app.js:100-102, 217-221, 255`; `public/index.html:26, 29, 80` |
| C2 | M | The same idea has several names: Loop / standby / "Loop (always shown)"; Intro / startup / "Intro (plays once at power-on)"; Gallery (in-app list) vs Gallery folder; Convert / Re-convert / Set up & convert. | `public/app.js:9, 52, 127` |
| C3 | M | Native `confirm()` / `prompt()` dialogs for Send, Delete, Rename, Clear, Setup, Restore, Reconnect: system-styled, cannot show thumbnails or time estimates, and the Send text uses shouting caps ("the INTRO clip"). | `public/app.js:58, 133, 142, 162, 409, 413, 419` |
| C4 | L | Long explanatory paragraphs under every card (`.hint`) repeated on every visit. | `public/index.html:26, 32, 64, 72, 75, 77` |

### 3.4 Bugs spotted during the audit

| # | Sev | Finding | Where |
|---|-----|---------|-------|
| B1 | M | A card shows a literal "false" text badge when `quality` is null (seen on "Coast"): `info.append(..., it.quality != null && el(...), ...)` uses native `append`, which stringifies `false`. | `public/app.js:100-102` |
| B2 | L | Gallery folder hint reads "DocumentsCoolerScreenGallery" (backslashes missing). | `public/index.html:76` |
| B3 | L | Auto quality can resolve to "quality 1" and is displayed as such; to a user that looks like a failure even when it was the only way to fit. | `public/app.js:101`; `src/convert.js:146-149` |
| B4 | L | The Start slider max is 10 until `dlgRanges()` runs; for images, Length max jumps to 30 while the global cap is `maxSeconds` (10). | `public/index.html:86-87`; `public/app.js:190-196` |

### 3.5 Visual design

| # | Sev | Finding | Where |
|---|-----|---------|-------|
| V1 | M | Flat hierarchy: h1 20 px, h2 16 px, body 14 px, hints 12 px; every card has the same weight and padding; all secondary buttons look identical (5 per card). | `public/style.css:5-6, 10-15` |
| V2 | M | Accent blue means both "primary action" and "selected as mask background". | `public/style.css:14, 24` |
| V3 | L | Borders/input outlines `#2e333b` on `#1e2127` are 1.27:1; form controls do not meet the 3:1 non-text contrast guideline. Text contrast is fine (muted text 5.2:1, primary button 7.2:1). | `public/style.css:1, 12` |
| V4 | L | No spacing or radius scale (6/8/10 px radii, assorted paddings), no type scale, colours partly hard-coded (`#2a2e36`, `#191c21`, `#3a3f49`, `#5a3d0a`). | `public/style.css` throughout |
| V5 | L | Thumbnails are 20:9 but nothing indicates the notch side or that the screen is a physical object; the red mask overlay on the canvas is barely distinguishable on colourful frames. | `public/style.css:25`; `public/app.js:361` |

### 3.6 Accessibility

| # | Sev | Finding | Where |
|---|-----|---------|-------|
| A1 | M | Modal is a plain `div`: no `role="dialog"`/`aria-modal`, no focus move/trap/return, Esc does nothing (verified). Player overlay also cannot be closed from the keyboard. | `public/index.html:83, 101`; `public/app.js:158, 247` |
| A2 | M | Thumbnails are clickable `div`s (not focusable); hover-to-preview has no keyboard equivalent. | `public/app.js:86-95` |
| A3 | M | Dialog `<label>`s are not associated with their inputs (no `for`, not wrapping), and each value has two inputs (range + number) without a shared label. | `public/index.html:86-93` |
| A4 | L | Status text changes (`#deviceStatus`, progress spans, toast) are not in live regions. | `public/index.html:15, 100` |
| A5 | L | Focus ring is the browser default (orange outline, visible on the drop zone); fine, but inconsistent and missing on custom elements. | `public/style.css` (none defined) |

---

## 4. Proposed information architecture and flow

### 4.1 Concepts and words (glossary used in all copy)

| Term in UI | Meaning | Replaces |
|---|---|---|
| **Original** (shown as the clip's name; the word "original" appears only in Detail) | A file in the Gallery folder: the untouched video/GIF/image/.ht theme. | "item" |
| **Version** | One prepared copy of an original with its own trim/quality, ready to send. An original has 0..n versions. | "conversion", "device.mp4" |
| **Library** | The in-app list of originals (mirrors the Gallery folder). | "Gallery" (in-app) |
| **Gallery folder** | `Documents\CoolerScreen\Gallery` on disk. | unchanged |
| **Screen** | The cooler display. | "device" |
| **On screen now** | The version that was last sent successfully by this app and is looping. | "Loop slot / Last sent" |
| **Send to screen** | Upload a version so it loops. | unchanged |
| **Notch** | The camera hole area on the left edge that is blacked out. | "mask" in main UI (keep "notch mask" in Settings) |
| **Space on the screen** | The ~9.5 MB storage limit. | "budget", "slot space" |
| **Intro clip** | Advanced only: plays once at power-on. | "startup slot" |

Words that must not appear outside Settings > Advanced or tooltips: slot, standby, startup, q, MJPEG, WinUSB, COM port,
budget, transpose. "fps" appears only if the device-fps decision (section 7.5) keeps it user-facing.

### 4.2 Primary flow

```
 Add clip  ->  (original appears in Library, selected, preview plays immediately; nothing converted)
    |
 Tune      ->  drag trim handles / length, quality (Auto default); preview updates instantly;
    |          fit meter shows "uses 8.5 of 9.5 MB" (estimate, refreshed after you stop dragging)
    |
 Save as version  ->  conversion queued, version row appears with progress, then "Ready"
    |
 Choose version   ->  compare versions (play each, see size/length/quality); pick one
    |
 Send to screen   ->  confirm sheet (replaces "<current>", about 3 min, keep app open)
    |                  -> Activity strip shows progress + time left -> success
    |
 Done        ->  "On screen now" in top bar and a badge on that version/original. Close the app.
```

Secondary flows: first-run setup, Reconnect after failed USB, change notch, change Gallery folder, intro clip,
restore driver, rename/delete original, delete version.

### 4.3 Screen map

```
App window
 ├─ Top bar: app name · Screen status pill · On screen now (mini thumb + name) · Settings (gear)
 ├─ Banner area (only when needed): Needs setup / Couldn't connect at startup / Screen not found
 ├─ Left pane: Library (Add clip button + drop target, list of originals, Open Gallery folder)
 ├─ Right pane: Detail of selected original
 │    ├─ Preview (screen-shaped, notch shown, plays tuned result or a version)
 │    ├─ Tune panel (start/length trim, quality, fit meter, Save as version)
 │    └─ Versions list (rows with play, size, length, status, Send, more menu)
 ├─ Activity strip (bottom, appears when anything is queued/running)
 ├─ Settings drawer (right side sheet): Screen · Notch · Gallery folder · Advanced
 └─ Dialogs: Send confirm, Delete confirm, Rename (inline), Setup confirm, Restore confirm
```

---

## 5. Screen-by-screen spec

Window: Electron 1280x860 default, min 900x640 (`electron/main.js:23`). Layout targets 900-1600 px wide.

### 5.1 Main window layout

```
+--------------------------------------------------------------------------------------------------+
| CoolerScreen        [● Screen ready]        On screen now: [thumb] Coast loop · v2      [⚙]     |  top bar 56px
+--------------------------------------------------------------------------------------------------+
| (banner, only when needed)                                                                       |
+------------------------------+-------------------------------------------------------------------+
| LIBRARY            [+ Add]   |  Coast loop                                   [Rename] [⋯]        |
| ┌──────────────────────────┐ |  Original: Coast loop.mp4 · 0:24 · 1920x1080                      |
| │ Drop files here          │ |  ┌─────────────────────────────────────────────────────────────┐  |
| └──────────────────────────┘ |  │▓▓│                                                          │  |
| ┌────┐ Coast loop         ●  |  │▓▓│              PREVIEW  (20:9, rounded corners)            │  |
| │thumb│ 3 versions · On screen| │▓▓│              notch band drawn in solid black             │  |
| └────┘                       |  │▓▓│                                                          │  |
| ┌────┐ 海岸                  |  └─────────────────────────────────────────────────────────────┘  |
| │thumb│ 1 version             |   ▶ 0:03 / 0:08    Showing: Your tuning (not saved)   [Notch ✓]   |
| └────┘                       |                                                                   |
| ┌────┐ Test pattern          |  TUNE                                          VERSIONS (3)       |
| │thumb│ No versions yet       |  Trim  [====|███████████|=========]           ┌───────────────┐  |
| └────┘                       |        Start 0:03.0   Length 8.0 s            │▶ v3 8 s · Best│  |
|  ...                         |  Quality  (•) Best that fits  ( ) Custom [—]   │  8.5 MB  Ready│  |
|                              |  Space on screen                              │  [Send to scr]│  |
|                              |  [██████████████████░░░] 8.5 of 9.5 MB  Fits  ├───────────────┤  |
|                              |  [ Save as version ]  Reset                   │▶ v2 ... ON    │  |
| [Open Gallery folder]        |                                               └───────────────┘  |
+------------------------------+-------------------------------------------------------------------+
| ⟳ Sending "Coast loop · v3" to the screen  ███████░░░░  42%  about 1 min 40 s left   [Details]   |  activity 48px
+--------------------------------------------------------------------------------------------------+
```

- Left pane: fixed 300 px (min 260). Right pane: fluid; inside it, Preview spans full width; Tune and Versions sit
  side by side at >= 1180 px (Tune 3fr, Versions 2fr), stacked (Tune then Versions) below that.
- Preview max height: 40% of window height; keep 20:9.
- Library is a vertical list (not a grid) so names are readable and status fits on a line.

### 5.2 Top bar

| Element | Spec |
|---|---|
| App name | "CoolerScreen", 16 px semibold. Drop the subtitle (move it to Settings > About). |
| Screen status pill | Dot + text, `button` that opens Settings > Screen. States in 6.1. |
| On screen now | 48x22 px thumb (20:9) + "<original name> · <version label>". Click selects that original and highlights the version. If unknown: "On screen now: unknown (sent with another app or before this version)". If nothing: hidden. |
| Settings | Icon button with `aria-label="Settings"`, opens drawer. |

### 5.3 Library pane

- Header: "Library" + primary-outline button **Add clip** (opens file picker; accepts same types as today).
- Drop zone: whole window remains a drop target (keep `app.js:293-295` behaviour); the Library pane shows a dashed
  outline with "Drop to add" while dragging. When the library is empty, show the large empty state (6.6).
- Row (64 px high): thumb 96x43 (20:9, from source frame, not from a version), name (1 line, ellipsis, full name in
  `title`), sub-line: one of
  - "No versions yet" (muted)
  - "3 versions"
  - "Preparing version..." with small spinner (if any version converting/queued)
  - "Can't read this file" (error colour) with tooltip of the plain-language reason
  - On-screen marker: a small "On screen" chip (success colour) when one of its versions is on screen.
- Row is a `button` inside a `role="listbox"`/`option` list (or plain buttons with `aria-current`); Up/Down moves
  selection, Enter focuses Detail.
- Footer: "Open Gallery folder" text button + muted line "Files you put there appear here automatically."
- Multi-file drop (P1): add all; today only the first file is imported (`app.js:289`).

### 5.4 Detail pane: header

- Title: original name (20 px). Inline rename on click of **Rename** (text field replaces title, Enter saves, Esc
  cancels; errors inline under the field, e.g. "A file with this name already exists in the Gallery folder.").
- Meta line (muted, 13 px): "<file name> · <duration m:ss or 'Still image'> · <width>x<height>".
- More menu (⋯): "Show in Gallery folder", "Remove from library..." (moves file to `_Deleted`, same as today).

### 5.5 Detail pane: Preview

The preview is the single most important new element: it is the user's stand-in for the physical screen.

- Frame: 20:9, `border-radius` 16 px (phone-like), 2 px border `--border-strong`, black background.
- Content: an HTML `<video>` (or `<img>` for stills) with `object-fit: cover` filling the 20:9 frame. This matches the
  converter's centred cover crop (`convert.js:104-107`), so the crop shown is exact.
- Notch: an absolutely positioned black element using the saved notch setting, sized in percent of 2400x1080 using the
  same `bandPx()` rounding (band: `width = bandPx/2400*100%`; circle: `left/top/width` from cx, cy, r). Toggle
  "Notch ✓" shows a thin dashed outline around it so users understand what it is; tooltip: "Your screen's camera hole
  is here. This area is kept black."
- Source label under the frame, one of:
  - "Showing: your tuning (not saved yet)" - plays the original, looping between trim start and start+length
    (`timeupdate`/`requestVideoFrameCallback`: if `currentTime >= end` set `currentTime = start`).
  - "Showing: version 2" - plays that version's `preview.mp4`.
  - "Showing: on the screen now" when that version is on screen.
- Controls: play/pause, time "0:03 / 0:08" relative to the trimmed clip, mute is irrelevant (no audio). Space toggles
  play when preview focused.
- Quality preview (P0-lite): after the size estimate finishes, show a "Check quality" link that opens a 1:1 zoom of one
  frame encoded at the chosen quality (server returns `sample.jpg` decoded from the sample MJPEG, see 7.3). P1: side by
  side with the original frame.
- Sources that the browser cannot play (avi, mkv with odd codecs, some .mov, .ht contents): at import, generate a
  low-res **source proxy** (H.264 1200x540, no mask, full length up to a cap, e.g. 120 s) and play that; see 7.2.

### 5.6 Detail pane: Tune panel

Heading "Tune". Controls bind to a **draft** (per original, auto-saved locally so leaving and coming back keeps it).

1. **Trim** (videos/GIFs only): a timeline track the width of the panel showing the source duration with a
   filmstrip (P1) or plain track (P0). Two handles (start, end) plus a draggable selected range. Below: "Start 0:03.0"
   and "Length 8.0 s" number fields (step 0.1 s). Length max = min(remaining duration, max length setting). Dragging
   seeks the preview to the handle; releasing resumes looping.
   - Images: replace Trim with "Show for [3] s" (only matters for intro; P1: hide entirely and use a fixed short loop).
2. **Quality**: radio group
   - "Best that fits (recommended)" - Auto.
   - "Custom" + slider 1-100 with labels "Smaller file" ... "Sharper". Show the number.
   Remove the "Manual quality" second row; the slider appears inline when Custom is chosen.
3. **Frame rate**: hidden when the app runs with a fixed device frame rate (7.5). If kept: "Smoothness" select
   (10, 15, 20, 25, 30 frames per second) under a "More options" disclosure.
4. **Space on the screen** (fit meter): see 5.7.
5. Actions: primary **Save as version**; text button **Reset** (back to defaults or to the last saved version's
   settings: "Start from: [Defaults | v2]" P1).
   - Disabled with reason when the estimate says it will not fit: "Too big. Shorten to about 6 s or choose Best that
     fits." (In Auto mode the converter fails if nothing fits, `convert.js:249`.)
   - If an identical version already exists (same settings + notch), button reads "Already saved as v2" and is
     disabled.

### 5.7 Fit meter

```
Space on the screen                                    Fits
[████████████████████████████░░░░░]  8.5 of 9.5 MB
Best that fits: quality 78 · estimate, final size may differ slightly
```

- Bar = total space for the loop (9.48 MB today: `slots.budgets().loop`). Fill colour: `--ok` when fits, `--warn` when
  within 3% of the limit, `--danger` when over (overflow shown as a hatched red extension past the end, capped at +50%).
- States: "Checking size..." (bar shows previous value at 40% opacity plus an indeterminate shimmer), "Fits",
  "Close to the limit", "Too big by 1.2 MB", "Size check paused while the screen is updating" (neutral, not red;
  replaces S5), "Couldn't check the size" (with Retry).
- Estimates run 600 ms after the last change (keep debounce), are superseded by newer ones (keep `estimateSeq`), and
  must not block the preview, which is purely client-side.

### 5.8 Versions list

Heading "Versions (n)". Newest first. Each row:

```
┌───────────────────────────────────────────────────────────────┐
│ [thumb] v3 · 0:03-0:11 · 8 s · Best that fits   [On screen]   │
│         8.5 MB · made today 14:02                              │
│         [▶ Preview]  [Send to screen]                   [⋯]   │
└───────────────────────────────────────────────────────────────┘
```

- Label: auto "v<n>" + summary "<start>-<end> · <length> s · <Best that fits | Quality 70>". User can rename (⋯ >
  Rename), stored as `label`.
- Status variants:
  - **Queued**: "Waiting for <other job> to finish" (from gate messages), Cancel (P1).
  - **Preparing**: progress bar + "Preparing... 40%" (plain words, never "Encoding 20 fps, q=12").
  - **Ready**: actions Preview and Send to screen.
  - **Too big** (manual quality over the limit): Send disabled, chip "Too big for the screen", hint with fix.
  - **Old notch setting**: chip "Made with an older notch setting" + action "Make updated copy" (creates a new version
    with the same trim/quality and current notch; never silently overwrites).
  - **Failed**: "Couldn't prepare this version: <plain reason>" + "Try again" + "Show details" (raw ffmpeg text in a
    collapsible `pre`).
  - **On screen**: chip "On screen" (success colour); the Send button becomes "Send again" (secondary).
- Clicking the row (not its buttons) shows it in the Preview.
- ⋯ menu: Rename, Delete version (in-app confirm; if it is on screen: "This version is on the screen now. Deleting it
  here does not remove it from the screen.").
- Empty: "No versions yet. Adjust the clip on the left and press Save as version."

### 5.9 Send flow

1. **Send to screen** on a ready version opens a confirm dialog (custom, not `confirm()`):

```
 Send to the screen?
 [thumb new]  →  replaces  [thumb current]
 "Coast loop · v3" (8.5 MB)       "海岸 · v1"

 This takes about 3 minutes. Keep CoolerScreen open until it finishes.
 The screen may go dark or restart briefly during the upload.
 (If LOVINGCOOL MONITOR is running, close it first.)

                                   [Cancel]  [Send to screen]
```

   - Time estimate = `size / UPLOAD_RATE` (UPLOAD_RATE initially 50 kB/s, i.e. 9 MB ~ 3 min; refine with the measured
     rate of the last upload, stored in settings). Show "about N minutes", rounded up to 1 min.
   - If the intro slot is not blank, the loop send also writes a blank intro first (`send.js:37-43`); add ~5 s, no
     extra text.
2. On confirm: dialog closes, the version row shows "Sending...", the **Activity strip** shows the job, and the top bar
   status pill becomes "Updating screen...".
3. Activity strip content during upload (from job messages, mapped to plain words):
   - "Waiting for the current preparation to finish" (queued behind conversions)
   - "Connecting to the screen"
   - "Sending ... 42% · about 1 min 40 s left" (time left from the observed rate since start; hide until 10 s of data)
   - "Starting playback"
4. Window close during upload (Electron): intercept `close` and show "An upload is in progress. If you close now, the
   screen may show a broken picture until you send again. [Keep open] [Close anyway]". Browser mode: `beforeunload`.
5. Success: strip turns green for 8 s "Done. Coast loop · v3 is on the screen." then collapses; On screen now updates;
   version row gets the On screen chip. If `serialWarning`: strip stays with warning style and "The screen is not set
   up yet, so it will switch off about 30 seconds after this app closes. [Set up screen]".
6. Failure: strip turns red, persists until dismissed, plain message + one action (6.3). The version stays Ready.

### 5.10 Settings drawer (right sheet, 420 px, Esc closes)

Sections, in order:

1. **Screen**: status line with details ("Connected · set up (no background program needed)" or the problem),
   button **Check again**. Device info (model, firmware) in a collapsed "Details". Primary **Set up screen** only when
   setup is needed; otherwise a text link "Run setup again".
2. **Notch**: preview of the current original's frame inside the same screen frame component with the notch drawn in
   red-tinted black and a 1 px white outline. Controls: Shape (Band on the left edge | Circle | Off). Band: "Width"
   slider 0-300 px with number; edge select under "More" (left is the only real-world case). Circle: X, Y, Size, Soft
   edge. **Save notch** applies to new versions; then a line "3 versions were made with the old notch. You can make
   updated copies from each clip's Versions list." plus P1 button "Make updated copies of all".
3. **Gallery folder**: path field + Browse (Electron dialog) + Save; "Open folder". Fix B2 copy.
4. **Advanced** (collapsed by default):
   - Intro clip: "Play a different clip once at power-on" toggle; when on, each version's ⋯ menu gains "Use as intro
     clip" and the confirm dialog shows the split of space. Clear intro / clear loop live here with explanation.
   - Restore original driver (danger button + in-app confirm with the existing explanatory text).
   - Max clip length (default 10 s).
5. **About**: version, "Unofficial tool for the Lovingcool SC70 USB screen", link to GitHub README/troubleshooting.

---

## 6. States (global)

### 6.1 Screen status pill (top bar)

| State | Pill | Banner | Primary action |
|---|---|---|---|
| Checking (on start) | grey dot, "Checking screen..." | none | - |
| Ready (WinUSB) | green, "Screen ready" | none | - |
| Needs setup (serial) | amber, "Needs one-time setup" | amber: "Your screen will switch off 30 s after this app closes until you run the one-time setup." | **Set up screen** |
| Couldn't connect at startup (failedUsb) | red, "Not connected" | red: "Windows couldn't connect to the screen when the PC started. This is a known glitch." | **Reconnect** (UAC note in button subtext: "Windows will ask for permission") |
| Not found | red, "Screen not found" | red: "Check the cooler's USB cable and power, then try again." | **Check again** |
| Busy (vendor app has port) | amber, "Screen in use" | amber: "Another program is using the screen. Close LOVINGCOOL MONITOR and try again." | **Check again** |
| Updating | blue, "Updating screen..." | none (Activity strip) | - |

Automatic check: run once on launch and after Setup/Reconnect, but only when no device job is queued (same rule as
`/api/device` today). It opens the port briefly; if that is undesirable (see 9), use a passive check (USB enumeration
only) for the pill and leave the full check to "Check again".

### 6.2 Setup and Reconnect

In-app dialog replacing `confirm()` (`app.js:409, 419`), with the same honest explanation shortened to 3 lines, then
progress inside the dialog ("Waiting for your approval in the Windows prompt...", "Switching driver...", "Done"). On
success: dialog shows a check and closes after 2 s; pill becomes Ready.

### 6.3 Error copy (plain language, one action each)

| Source | Message | Action |
|---|---|---|
| Unreadable file on import | "CoolerScreen can't read \"<file>\". It may be damaged or in a format that isn't supported." | Dismiss |
| Conversion failed | "Couldn't prepare this version." + details collapsed | Try again |
| Auto can't fit | "This part is too long to fit even at the lowest quality. Try about <n> s or less." | (focus Length) |
| Upload: screen not found | "Lost contact with the screen during the upload." | Check again, then Send again |
| Upload: failedUsb | "Windows lost the screen's USB connection." | Reconnect |
| Upload: port busy | "Another program is using the screen. Close LOVINGCOOL MONITOR." | Try again |
| Upload stalled (timeout) | "The screen stopped responding. Unplugging isn't needed: press Reconnect, then send again." | Reconnect |

### 6.4 Queue / busy

- Activity strip lists running + queued jobs (one line each, max 3 visible, "+2 more"). Clicking "Details" expands to
  show all with plain messages from `gate.js` ("Waiting for the screen update to finish").
- While a device job runs: Send buttons elsewhere change to "Send (after current update)" and queue normally;
  estimates show the neutral paused state.

### 6.5 Import

- Drop or Add: the original appears at the top of the Library immediately with a skeleton thumb and "Adding...".
  After registration it is selected and the preview plays. No dialog, no conversion.
- Duplicate name: unchanged behaviour ("name (2)"), no message.
- Multiple files (P1): all added in order.

### 6.6 Empty library (first run)

```
            ┌───────────────────────────────┐
            │      [screen-shaped outline]  │
            └───────────────────────────────┘
        Put your own picture on the cooler screen
   Drop a video, GIF, image or LOVINGCOOL theme (.ht) here,
             or [Add clip]   ·   [Open Gallery folder]
   You'll adjust it and preview it before anything is sent.
```

If the screen also needs setup, show the setup banner above; do not block adding clips.

---

## 7. Data model and backend implications

### 7.1 Originals

- Unchanged principle: each file in the Gallery folder is one original; the Gallery folder contains originals only.
  Versions are derived data and live in `%APPDATA%\CoolerScreen\library` (users don't need MJPEG portrait files; P1
  "Export version" can copy one out on demand).
- **Stable id problem**: today the id is `sha1(path|size|mtime)` (`gallery.js:39-40`). Anything that touches the file
  (re-save, sync tool, timestamp change) produces a new id, and the cache entry is dropped as "gone"
  (`gallery.js:114`). That was tolerable with one throwaway conversion; with several versions the user curated it means
  data loss. Recommendation: keep an index `library/index.json` mapping `originalId -> { file, size, mtime, contentHash }`
  with `originalId` random and permanent. On sync, match by path first; if size/mtime changed but path is the same,
  keep the id and mark versions "source changed" (versions remain sendable). A file removed from the folder hides the
  original; its versions are kept for 30 days (or until "Clean up") so an accidental move can be undone.
- Original meta (`library/<originalId>/original.json`):
  `{ id, name, file, extracted, fileSize, fileMtime, created, sourceDuration, isImage, srcWidth, srcHeight,
  proxy: 'proxy.mp4'|null, draft: { start, length, quality } , status: 'ready'|'unreadable', error }`.
- Files per original: `thumb.jpg` (from source), `frame.jpg` (unmasked, for notch editing), `proxy.mp4` (if the
  browser can't play the source natively, or always for simplicity), extracted `original.<ext>` for .ht/.zip.

### 7.2 Versions

- Directory: `library/<originalId>/versions/<versionId>/` with `device.mp4`, `preview.mp4`, `thumb.jpg`,
  `version.json`.
- `version.json`:
  ```
  { id, originalId, n (1,2,3... per original), label (null = auto), created,
    settings: { start, length, quality: 'auto'|1..100, fps },         // what the user asked for
    notch: { ...mask snapshot },  notchKey: '<hash of mask>',
    result: { size, q, quality, fps, start, length, tooBig, budgetBytes },
    status: 'queued'|'converting'|'ready'|'error', error }
  ```
- "Old notch setting" = `notchKey !== hash(settings.mask)` (replaces `maskOutdated`, `library.js:265`).
- Duplicate detection = same `settings` + `notchKey`.
- Migration: each existing `library/<id>/` with `device.mp4` becomes original + `versions/<new>/` with `n: 1`, settings
  from `requested`, result from the current meta fields. Items with status `new` become originals with no versions.

### 7.3 API (sketch)

| Method | Path | Purpose |
|---|---|---|
| GET | `/api/originals` | list with `versionCount`, `onScreen` flag, busy flag |
| GET | `/api/originals/:id` | original + versions |
| PATCH | `/api/originals/:id` | rename (file rename as today), save `draft` |
| DELETE | `/api/originals/:id` | move file to `_Deleted` |
| POST | `/api/originals/:id/estimate` | as today's estimate; also returns `sampleUrl` (one JPEG decoded from the sample at the chosen q) |
| POST | `/api/originals/:id/versions` | body = settings; creates version (status queued) and returns `{ versionId, jobId }` |
| PATCH/DELETE | `/api/versions/:vid` | rename / delete |
| POST | `/api/versions/:vid/send` | body `{ slot }` (default loop); job |
| GET | `/api/activity` | running + queued jobs with plain messages (backed by `gate.js` + `jobs`) |
| GET | `/api/screen` | last known screen state + on-screen-now record, no USB access |
| GET | `/media/o/:id/{thumb.jpg,frame.jpg,proxy.mp4,source}` and `/media/v/:vid/{preview.mp4,thumb.jpg}` | media with Range support (existing `serveFile`) |

Jobs should carry `{ kind, label, originalId, versionId, status, progress, message, startedAt, bytesTotal }` so the
Activity strip can name things and compute time left.

### 7.4 On screen now

Change `settings.slots.standby/startup` (`settings.js:72-76`) to store
`{ originalId, versionId, name, label, size, at }`. The UI resolves ids to live objects; if the version was deleted,
show the stored name with "(deleted from library)". If the last send failed midway, store
`{ ..., incomplete: true }` and show "Last upload didn't finish - the screen may show a broken picture. [Send again]".

### 7.5 Fixed device frame rate (fps disappears)

- Add `DEVICE_FPS` (constant or `settings.deviceFps`, null = user-controlled). When set:
  - Tune hides the frame-rate control; `settings.fps` in versions is always `DEVICE_FPS`.
  - Auto fitting adjusts quality only (drop the fps ladder in `convert.js:132-136, 167-174, 235-247`); if q=31 still
    doesn't fit, return `maxFittingLength` (estimated from bytes per second at q=31) so the UI can say "try about n s".
  - Version summaries omit fps.
- Keep fps in `version.json` regardless, so older versions made at other rates are identifiable ("made at 25 fps,
  may play at the wrong speed" chip if it differs from DEVICE_FPS).

---

## 8. Visual style (dark theme tokens)

Plain CSS custom properties on `:root`; no framework. All component colours must reference tokens.

### 8.1 Colour

| Token | Value | Use |
|---|---|---|
| `--bg` | `#0f1115` | window background |
| `--surface-1` | `#161920` | panes (Library, Detail) |
| `--surface-2` | `#1d2129` | cards, rows, inputs |
| `--surface-3` | `#262b35` | hover, selected row background |
| `--border` | `#2b313c` | dividers (decorative) |
| `--border-strong` | `#4a5262` | input/control outlines, preview frame (>= 3:1 on surface-2) |
| `--text` | `#e8eaee` | primary text |
| `--text-muted` | `#9aa2b1` | secondary text (>= 4.5:1 on surface-1/2) |
| `--text-faint` | `#6f7787` | timestamps only, >= 14 px |
| `--accent` | `#5aa9ff` | primary buttons, focus ring, links, progress |
| `--accent-ink` | `#04121f` | text on accent |
| `--accent-soft` | `rgba(90,169,255,.14)` | selected row tint |
| `--ok` | `#45c97a` | fits, on screen, success |
| `--warn` | `#f0b43c` | needs setup, close to limit |
| `--danger` | `#ff6b6b` | errors, too big, destructive |
| `--ok-soft` / `--warn-soft` / `--danger-soft` | same hue at 14% alpha | chips and banners backgrounds |
| `--notch` | `#000000` | notch area in previews |

Rule: accent = "do this / you are here"; green = "on screen / fits"; selection uses `--accent-soft` + a 2 px accent
left bar, never the same full border as buttons.

### 8.2 Type (system UI: `"Segoe UI Variable", "Segoe UI", system-ui, sans-serif`)

| Token | Size / line height / weight | Use |
|---|---|---|
| `--fs-title` | 20 / 28 / 600 | Detail title |
| `--fs-h` | 15 / 22 / 600 | section headings (Tune, Versions, Library) |
| `--fs-body` | 14 / 20 / 400 | default |
| `--fs-small` | 13 / 18 / 400 | meta lines, hints |
| `--fs-tiny` | 12 / 16 / 500 | chips only |
| tabular numbers | `font-variant-numeric: tabular-nums` | sizes, times, percentages |

Minimum text size 12 px; hints are 13 px, not 12.

### 8.3 Spacing, radii, elevation, motion

- Spacing scale (4 px base): `--s1 4`, `--s2 8`, `--s3 12`, `--s4 16`, `--s5 24`, `--s6 32`. Pane padding 16/24,
  row padding 8/12, section gap 24.
- Radii: `--r-sm 6` (inputs, chips), `--r-md 10` (cards, rows, buttons), `--r-lg 16` (preview frame, dialogs).
- Elevation: dialogs/drawer `0 12px 40px rgba(0,0,0,.5)`; nothing else has shadows.
- Motion: 150 ms ease-out for hover/press, 200 ms for drawer/dialog; respect `prefers-reduced-motion` (no shimmer).

### 8.4 Components

- Buttons: Primary (accent fill), Secondary (surface-2 + border-strong), Ghost (text only), Danger (danger text,
  danger-soft on hover). Height 32 px (36 px for the main Save as version / Send to screen). One primary per region.
- Chips: 20 px high, `--fs-tiny`, soft background + matching text colour.
- Progress: 6 px track in rows, 8 px in Activity strip, rounded; text label always next to it, never inside.
- Focus: `:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }` on every interactive element.
- Icons: inline SVG (16 px), `aria-hidden`, always paired with text except gear/play/more (which get `aria-label`).

---

## 9. Open questions for the user / orchestrator

1. Should the app auto-check the screen at launch (briefly opens the port), or only show passive USB presence until
   the user acts? (Spec assumes auto-check when nothing else is running.)
2. Version retention when an original file disappears from the Gallery folder: keep 30 days, keep forever, or delete
   immediately (today: delete immediately)?
3. Is "Framing" (pan the crop left/right/up/down, or Fit with black bars instead of Fill) wanted? The user's "size"
   may mean framing rather than file size. Listed as P1 below.
4. Value of `DEVICE_FPS` once the playback-speed investigation finishes.
5. Should the intro clip feature remain at all in the UI, or only in the CLI?

---

## 10. Priorities

### P0 (this iteration)

1. **Import adds an original only**: remove `openDialog` after import (`app.js:276`); generate thumb/frame/proxy at
   registration; select the new original and play it.
2. **Two-pane layout** (Library list + Detail) replacing the stacked cards; Settings drawer for notch, gallery folder,
   setup/restore, intro/clear-slot.
3. **Tune panel with instant client-side preview**: trim (start/length) looping in a 20:9 screen frame with the notch
   drawn, quality Auto/Custom, debounced size estimate, fit meter with plain states. Frame-rate control hidden behind
   `DEVICE_FPS`.
4. **Versions**: data model (7.1-7.2) with migration, "Save as version" (queued conversion), versions list with
   Preview / Send / Rename / Delete, statuses (queued, preparing, ready, too big, old notch, failed).
5. **On screen now**: store ids on send (7.4), show in top bar, Library row chip, and version chip.
6. **Send flow**: in-app confirm with replace-what and time estimate, global Activity strip with progress, time left,
   plain step names, success and failure states, close-window guard during upload.
7. **Screen status pill and banners** (6.1) with one action each; setup button only when needed.
8. **Copy pass** using the glossary (4.1) and error table (6.3); no `confirm()`/`prompt()` anywhere.
9. **Accessibility basics**: dialog semantics with focus trap/return and Esc, keyboard-operable library and preview,
   associated labels, `:focus-visible` ring, live region for Activity/status, 3:1 control borders.
10. **Bug fixes** B1-B4.
11. **Style tokens** (section 8) applied across the UI.

### P1 (next)

- Quality check zoom (sample frame vs original), filmstrip on the trim track.
- Framing: pan position and Fill/Fit.
- "Make updated copies of all" after a notch change; per-version Cancel while queued.
- Multi-file import; search/sort in Library.
- Export a version to the Gallery folder; storage use and "Clean up" in Settings.
- Version comparison (A/B toggle in the preview).
- Remember measured upload rate for better time estimates; show "last sent <date>" on the On-screen chip.
- Light theme (tokens already make this cheap).
- Keyboard shortcuts: Space play/pause, S save version, Ctrl+Enter send selected version.

---

## 11. Implementation notes (plain HTML/CSS/JS)

- No framework needed. Suggested split: `public/app.js` (bootstrap, state store with a tiny `subscribe/notify`),
  `public/ui/library.js`, `ui/detail.js`, `ui/tune.js`, `ui/versions.js`, `ui/activity.js`, `ui/settings.js`,
  `ui/dialog.js` (one accessible dialog helper used by every confirm), `ui/screenFrame.js` (the preview frame +
  notch overlay component, reused in Detail, Settings > Notch, Send dialog thumbnails). Load as ES modules
  (`<script type="module">`); Electron and modern browsers support it.
- Re-render by region, not the whole page: today `renderGallery()` destroys in-flight progress bars (S1). Keep job
  progress in the store keyed by job id and render it wherever the job is shown.
- Replace the 3 s `/api/library` polling signature trick (`app.js:437-446`) with polling `/api/activity` (fast, 1 s while
  jobs exist, 3 s idle) plus `/api/originals` on change; P1: Server-Sent Events.
- Keep all existing safety behaviour: gate queue, size limit enforcement before send, `_Deleted` instead of delete,
  no dangerous device keys.

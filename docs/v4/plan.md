# CoolerScreen 4.0 plan (shared by every 4.0 session)

Written 2026-10-18 by the 4.0 PM session. Approved by the user the same day ("Go ahead with the plan as written,
including the landing-order version numbers"). Every worker session reads this file, the repo `CLAUDE.md` standing list
and `docs/roadmap/2026-10-18-v3-release-and-v4.md` before starting.

## Scope (user's priorities, 2026-10-18)

1. **Live effects** (top): backgrounds drawn live, "similar to Wallpaper Engine where it's not the actual video itself".
2. **More widgets** (top): 3.0's widgets are "very lacking ... all the regular ones".
3. **Small improvements** ("that's nice, implement that"): FPS counter, "screen changed outside the app" check, known
   gaps from the 3.0 wrap-up.
4. **UI audit and revision**: the UX (flows, behaviour) is good and stays; the UI (visual design) is "quite a bit of a
   mess". Audit it, then revise it.

Backlog (not now): theme sharing, calibration for other users, GitHub packaging and signing.

## Sessions

| Stream | Session | Owns |
|---|---|---|
| A | Live effects | effect renderer, effect backgrounds in themes, recording the Embedded loop from an effect |
| B | More widgets | new widget kinds (engine drawing + editor controls + data sources) |
| C | Small improvements | FPS counter, changed-outside check, flaky tests, dirty "+" hash, countdown DST, GPU power dash |
| D | UI audit, then revision | `docs/v4/ui-audit.md`, `docs/v4/ui-style-guide.md`; later the restyle of `public/` |
| E | Contingency | idle until the PM assigns stuck or broken work |
| PM | this session | plan, merges into `main`, versions, changelog, builds, installs, device checks, morning report |

D's revision of `public/` starts only after A and B have landed their first milestones (the PM says when). Until then A
and B follow `docs/v4/ui-style-guide.md` for any new UI as soon as it exists (the PM tells them).

## Rules for worker sessions

- **Work on your own branch** (your worktree). Commit in small focused commits. Never commit to `main`, never push,
  never merge other branches yourself; the PM merges. When the PM says `main` moved, merge `main` into your branch.
- **Never touch the real screen.** Only the PM talks to the device and installs builds. Never run `cs2` against the
  device, never close or restart the user's running CoolerScreen.
- **Test app instances** run with all of: `COOLERSCREEN_FAKE_DEVICE=1`, `COOLERSCREEN_MULTI=1`,
  `COOLERSCREEN_HOME=<a folder in your scratchpad>`, `COOLERSCREEN_TEST_QUIET=1`. Nothing may appear on, focus or go
  fullscreen on the user's desktop. `COOLERSCREEN_DUMP_FRAMES` writes live frames to disk for checking pictures.
  **Test instances must be DEBUG builds** (2026-10-19 incident): `COOLERSCREEN_MULTI` only works in debug builds; a
  release build hands its launch to the user's running app and can raise its window. A debug build WITHOUT
  `COOLERSCREEN_MULTI=1` does the same (second incident, 2026-10-19 14:17); 4.9.1 adds an in-app guard.
- **Builds:** use your own `CARGO_TARGET_DIR` (e.g. `%TEMP%\cs-target-<stream>`). Tests:
  `cargo test --release --lib -- --test-threads=4` under a timeout. Run long builds in the background.
- **Do not edit:** `CHANGELOG.md`, the version in `Cargo.toml` / `tauri.conf.json` / `package.json`, `CLAUDE.md`,
  this plan. Write your release notes and verification evidence in `docs/v4/notes/<stream>.md` (A-effects.md,
  B-widgets.md, C-improvements.md, D-ui.md); the PM turns them into the changelog.
- **Theme contract** (`docs/v3/theme-format.md`): add your own clearly headed section; do not renumber. The PM assigns
  the contract version at merge. Old themes must load and look exactly as before.
- **Prefer new files** over growing shared ones (`public/app.js`, `public/style.css`, `src-tauri/src/live/render.rs`,
  `theme.rs`): put new code in new modules and keep edits to shared files small hooks, so merges stay clean.
- **Standing rules** (from CLAUDE.md): defaults behave like 3.0 (nothing new turns on by itself); every option is a
  setting; no Save buttons (changes apply and save at once); the stored picture is called "Embedded"; never upload
  non-MJPEG to clip slots; never send 0x05, 0x0C or 0x23; nothing hidden in the background.
- **Device facts** you must design for: live frames portrait 756x1680, JPEG 4:2:0, keep re-sending; motion along the
  landscape vertical axis tears (move sideways or fade); large numbers (48 px+ on the 2400 canvas); flash ~11 MB total.
- **Milestones:** when a milestone is done (built, tests pass, checked with a quiet test instance, notes written), end
  your turn with a report that starts `MILESTONE READY: <stream> <short name>` and gives the branch, commit, what it
  does, how you verified it and what is not verified. Then wait for the PM. Also report `BLOCKED:` with the question
  if you cannot continue; the user is asleep overnight, so make a sensible choice, record it in your notes, and only
  block on things that are truly unsafe.

## Versions (user, 2026-10-18)

Each milestone gets its own version in the order it lands on `main`: the first is 4.0.0, then 4.1.0, 4.2.0, ...;
fixes get a patch number (4.2.1). Every version gets a tag `vX.Y.Z`, a changelog entry naming its stream, a build and an
install. Only the PM sets versions.

## Seating chart (vault revisions 28-29, from 2026-10-19 afternoon)

Route each new task to the principal whose area fits (one line saying why); queue it if that one is busy; start a new
principal if none fits; a reused principal merges the latest main first. New parts tell the principal to hand groundwork
to Sonnet players and to report "Helpers used", "Verified", "Not verified".

| Principal | Area | Files it knows | Branch | Status |
|---|---|---|---|---|
| A: live effects | live rendering engine, effect backgrounds, Embedded recording, store speed | `live/effects/`, `live/render.rs`, `live/pipeline.rs`, `live/effects/record.rs`, `core/themes.rs` (store path) | `claude/determined-greider-88a800` | idle |
| B: widgets | widget kinds and data sources, per-item corner radius, snap/gap rules | `live/morewidgets.rs`, `live/render/more.rs`, `live/extsensors.rs`, `ping.rs`, `agenda.rs`, `volume.rs`, `public/js/morewidgets.js`, `layers.js` snap | `claude/recursing-matsumoto-31e4c5` | busy: game FPS counter (G1 research, then G2) |
| C: app plumbing | live host, preview on select, digital twin, screen checks, budgets, `--quit`, single instance | `core/live_host.rs`, `core/outside.rs`, `core/settings.rs` budgets, `quit_cmd.rs`, `instance_id.rs`, `public/js/screenpreview.js`, twin | `claude/vibrant-proskuriakova-d382db` | busy: drag ghosting bug |
| E: music | music and media widgets: now playing, music styles, lyrics (research first) | `live/nowplaying.rs`, the music renderer and panel, new lyrics files | its own worktree branch | busy: M4 karaoke lyrics (M3 looks in 4.16.0) |
| D: UI | look and layout of `public/`, style guide, editor shell, UI flow checks | `public/css/*`, `public/js/edshell.js`, `edhooks.js`, `icons.js`, `themes.js` layout, `tools/ui-shots/` | `claude/zealous-swanson-66219d` | idle |

Note (review 2026-10-19): E3's live preview would have fitted A (live rendering) better than C; route future live-picture
work to A.

## Editor refinement round (user, 2026-10-19; vault user flows C1, "User requests 2026-10-19")

Sequenced so two sessions never rewrite the same editor code at once (`public/js/themes.js` is the hot file):
- **E1 (stream D) editor shell, first:** resizable left and right panels; Layers moved from the bottom into the tall left
  panel under the palette with a draggable divider; palette groups as accordions; consistent alignment in every group
  (Sensors looked misaligned); the deselect rule; Properties clean-up (remove "Position and size"; "Delete this item"
  pinned bottom-right of the panel).
- **E2 (stream B) per-item corner radius + notch-aware gap:** engine, contract and a small control hook in Properties
  (after E1 lands, or coordinate the hook); snapping and gap measured from the notch band and the visible area.
- **E3 (stream C) digital twin + smooth dragging:** the editor preview shows the live picture as it is on the screen;
  dragged boxes follow the pointer every frame. Backend work can start at once; the editor-side changes wait for E1.
- **More widgets:** after E2; the user said "maybe", so stream B proposes a short list first.

Defaults (user approved 2026-10-19 "go ahead with the plan and the defaults"; change freely if the user asks):
- Panel resize follows the vault pattern [[Interactions/Panel width snapping on release]]: width follows the drag
  continuously, eases to the nearest preset on release, small resisted overshoot at the bounds. Presets: right panel
  280 / 360 / 480 px, left panel 220 / 300 / 380 px (bounds about 200 to 560). The Layers/palette divider uses the same feel
  vertically. Sizes are remembered (per viewer, no Save button).
- Accordion: several groups may be open at once; open/closed is remembered; all open on first run.
- Deselect: a click anywhere outside the selected item and outside the right-hand panel deselects (empty canvas, left
  panel blank areas, top bar). Kept: clicks in the right panel, on the item itself, and moving/resizing the app window.
  Esc also deselects.
- Radius per item: off by default = follows the theme's corner radius; turning it on sets the item's own radius; a reset
  arrow returns it to the theme value (same pattern as the theme corner field). Live-only: never marks Embedded stale
  unless the item is drawn on Embedded.
- Twin: the preview shows the real live frames at about 15 fps while the editor is open (with Live on, the frames sent to
  the screen; with Live off, rendered for the editor only, nothing sent); the editable boxes stay as outlines on top.

## Milestones (initial; the PM may reorder)

- **C1** FPS counter + quick fixes (flaky tests, dirty hash, countdown DST, GPU power dash).
- **C2** "Screen changed outside the app" check (read-only device queries only; PM verifies on hardware).
- **A1** Effect renderer in the engine with 3 or more generated effects as theme backgrounds, editor controls, CPU
  measured against the budget (live total about 5% of the PC's CPU or less with an effect; heavier ones off by default).
- **A2** Embedded face for effect themes: record a seamless loop of the effect as the stored clip (MJPEG only).
- **A3** More effects / web or scene import (only if A1 and A2 are solid).
- **B1** First batch of new widgets (the session proposes the list in its notes; widely expected ones first).
- **B2** Second batch, including CPU temperature through an opt-in source we never install or elevate.
- **D1** UI audit + style guide (documents only).
- **D2** UI revision of `public/` (after A1 and B1 land).

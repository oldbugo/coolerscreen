# E: music (stream E notes)

## M1: play/pause shown reversed (fix)

User report 2026-10-19: "when I have it playing, it actually shows paused, and when I have a pause, it actually shows it's playing"
(YouTube Music, with Nocturne running).

**Cause (verified in code):** the music card drew the *state* (a triangle while playing, two bars while paused). Every player, and
CoolerScreen's own preview button, draws the *button* convention: two bars while playing, a triangle while paused. So the card looked
reversed whatever app was playing. Not a Windows status problem: GSMTC `Playing`/`Paused` were mapped correctly.

**Fix:** `live/render/widgets.rs` `transport_icon()`: two bars while playing, triangle while paused. Same size, place and colour.
No setting: this is a bug fix, and existing themes otherwise look the same.

**Which player the card follows when several apps have a media session** (`live/nowplaying.rs` `choose()`), in order:
1. a preferred app, if one is set (internal `MusicConfig.preferred_app`; nothing in the UI sets it yet, so normally skipped);
2. Windows' current media session (the one the Windows volume flyout shows), if it is playing;
3. otherwise the first session that is playing;
4. otherwise Windows' current session (paused; the card stays for a minute after pausing, as before);
5. nothing: the card hides as before.

Before, step 2 did not exist: with two players playing at once the card followed whichever Windows listed first.

**Probe of this PC (read only, 2026-10-19):** two sessions from the same app id `308046B0AF4A39CB` (a hashed id, as browsers and
packaged apps use): one Playing ("Raindance", Dave and Tems), one Paused (a second track). The card follows the playing one, under the
old and the new rule alike, so the reversal the user saw was the icon. No session named Nocturne or a test player was present at that time.

**Known, left as is:** event hooks are keyed by app id, so when two sessions share one id, switching between them is picked up by the
1 s poll rather than an event (up to 1 s late). Harmless.

**Tests:** `transport_icon` unit test; a render test (`music_icon_tests`) drawing the card playing and paused and checking two bars
vs one shape in the icon area; five `choose` tests (preferred wins even when paused, two playing → the current one, current paused +
other playing → the playing one, none playing → current, empty → none). Full lib suite: 337 passed, 0 failed, 19 ignored.

Changelog line (for the PM): "Music: the play/pause sign now shows like a player's button (two bars while playing); with several players
open, the one Windows shows as current wins when it is playing."

## M3: more music looks

User (2026-10-19): "a few more styles. Current one, it's fine ... we're not going to remove this one, but we'll just have more."
Four new looks in Properties > Music (picture choices): **Card** (today's, default), **Cover**, **Line**, **Disc**, **Blur**; "Turn while playing"
shows for Disc only. Contract: see `docs/v3/theme-format.md` "music looks" section.

- Code: `live/render/music.rs` (looks, per-item caches: rounded cover, blurred background, pre-drawn vinyl, ticker strip), small hooks in
  `render.rs` (module, one cache field, refresh rate, motion key), `render/widgets.rs` (one early branch in `draw_music`), `theme.rs`
  (`style`, `spin`), `nowplaying.rs` (cover decode cap 640 px); editor `public/js/w-music.js` + one call in `themes.js`. No new CSS.
- Motion: Line slides sideways (60 px/s on the design canvas) and fades; Disc rotates. Both redraw at 30 Hz; nothing moves vertically.
- Cost per redraw of the widget (release, 1680x756): card 0.72 ms, cover 0.71, line 0.54, disc 1.44, blur 0.64. Disc at 30 Hz ≈ 43 ms of work per
  second (about 4 % of one core, well under 1 % of the PC on a multi-core CPU).
- For stream D: the picker reuses `.styles` / `.sty`; icons are inline SVGs in `w-music.js`.

Changelog line (for the PM): "Music: four new looks (Cover, Line, Disc, Blur) besides the current card, which stays the default."

**Verification (M3):** full lib suite 348 passed, 0 failed, 20 ignored (13 new music tests: every look draws only inside its box; no style =
pixel-identical card; Line moves only sideways; a paused Disc is still and a playing one changes only inside its square; spin by time; serde
round-trip keeps old props byte-identical). Quiet DEBUG instance (port 9781, fake device, scratch home): the panel shows the five picture choices,
"Turn while playing" only for Disc, theme.json gets `style` and loses it again on Card; live frames of all five looks with a real (paused) player
session looked right. Pictures: `E-m3-looks.png` (all five, sample render), `E-m3-panel-disc.png` (panel at 1280x860).
Not verified: a spinning Disc in a live session (nothing was playing); CPU in a release build of the app (debug build measured the same for Card and
Disc, so the look itself adds no visible load); the real screen.

## M4: lyrics widget (option A, the karaoke line; user's pick 2026-10-19)

New palette item **Lyrics** (Media). Contract: `docs/v3/theme-format.md` "lyrics" section. Design and sources: `E-lyrics-research.md`.

- Code: `live/lyrics.rs` (data: LRC parser, title cleaning for YouTube-style titles, LRCLIB lookup with strict matching, back-off, disk cache,
  `.lrc` folder, per-track offsets, smooth clock re-anchored on seeks and play/pause), `live/render/lyrics.rs` (drawing, cached line pictures),
  `public/js/w-lyrics.js` (Properties). Hooks: `render.rs` (module, cache field, cluster flag, 30 Hz while a line fills or fades, draw arm),
  `pipeline.rs` (the now-playing watcher also runs for a lyrics item), `morewidgets.rs` (type list and props cleaning), commands
  `lyrics_status` / `lyrics_offset_set` (`core/commands.rs`, `main.rs`, `build.rs`, `capabilities/main.json`), `icons.js` (one icon line),
  `morewidgets.js` (entry). No new CSS.
- Off by default: "Find lyrics online" must be switched on per item; with it off nothing is sent anywhere (a `.lrc` folder still works).
  When on: one lookup per song change, only while the item is drawn, sending title, artist and length to lrclib.net; requests are paced
  (300 ms apart), `Retry-After` is honoured, failures back off 30 s to 10 min. Lyrics are cached under the app data folder (`lyrics/`).
- No AGPL code: everything is our own (ideas only from the research notes).
- Cost (release, per redraw of a 1680x756 frame with the item filling): card 3.3 ms whole frame (1.4 ms the item), gradient 3.9 ms (1.9 ms).
  At 30 redraws/s while a line fills that is about 4 % of one core for the item, well under 1 % of the PC on a multi-core CPU.

**For the PM:**
- LRCLIB's API terms ask clients to identify themselves with app name, version and a homepage link or e-mail. We send
  `CoolerScreen/<version> (private; lyrics widget)`; there is no public page to link. Decide whether to add a contact (not the user's
  e-mail without asking them).
- Helpers made 6 read-only lookups to lrclib.net in development (well-known songs, plus one for the song that was playing during the quiet
  check); 3 of them were not planned (an ignored live test ran with `--include-ignored`).
- On a very bright, busy background the dim previous/next lines and the gap dots are faint over the Card scrim (seen on the test pattern).
  Fine on dark pictures; a "stronger card" option could follow if the user finds it hard to read.

**Verification (M4):** release lib suite 382 passed, 0 failed, 23 ignored (lyrics tests include: draws only inside its box, centred, the fill
grows left to right with identical ink rows, a cross-fade never puts ink on new rows, gap dots only change opacity, every no-lyrics state,
CJK and very long lines stay inside, preview never creates a loader; LRC parsing, matching, cache, offsets, clock). Debug bin build with the
new commands. Quiet DEBUG instance (port 9781, fake device, scratch home and gallery): palette entry with icon; panel controls as designed,
"Find lyrics online" off by default; the three backgrounds; theme.json props; live frames with the PC's real (read-only) media session:
"Lyrics lookup is off" with it off; after switching it on, one lookup found the song (`state: found, source: lrclib`) and the frame showed
the intro dots with the first line. A bug found there (the panel kept polling `lyrics_status` after it closed) is fixed: the poll stops when
the panel is gone and skips while hidden.
Not verified: a sung line filling on the live screen in real time (the check saw the intro), the editor preview with no media session
(should show the made-up demo lyric), CPU in a release build of the app, the real screen.

Changelog line (for the PM): "New Lyrics widget: synced lyrics of what is playing, the current line filling in time with the previous and
next line dim around it. Off until you switch on “Find lyrics online” (lrclib.net) or choose a folder of .lrc files."

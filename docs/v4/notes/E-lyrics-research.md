# E: Lyrics widget research (Folia and alternatives)

Evidence labels: **documented** (read in source/docs), **proposed** (my design idea), **unverified** (not read or only inferred).
Licence note: Folia and AMLL are AGPL-3.0. Only ideas are described here; nothing is copied.

## Plain-language summary

- Folia is a full-screen music player whose selling point is animated lyrics: words light up one by one, in many "styles", over AI-coloured backgrounds. Most of that look depends on a GPU, a browser and big vertical or 3D movement.
- On our panel the rule "no up/down motion" removes the Apple-Music-style scrolling list, but Folia's own flagship style ("Luminous/Classic") is mostly a single centred line with per-word glow, which suits us well.
- We can realistically copy the core feel: one big current line, words filling or glowing in time, soft fade between lines, a coloured background. We cannot copy 3D scenes, blur-heavy layers or AI-generated themes.
- Lyrics must come from somewhere. The only clean, free, no-account source is LRCLIB (line-synced, community made). Windows "now playing" does not carry lyrics. Word-by-word timing is rare outside private Chinese-service APIs we should not use.
- Biggest risks: song not found (esp. YouTube titles), wrong song matched, and timing drift because Windows only reports the playback position irregularly.
- Verdict: a good "line lyrics" widget is likely (medium effort). Word-level fill is feasible only for songs where we have word timing, otherwise we can fake a smooth left-to-right fill per line (looks nearly as good). A full Folia clone: not realistic.

## Options

| Option | What the user sees | Data | Effort | Keeps of Folia |
|---|---|---|---|---|
| A. Karaoke line (recommended) | Big current line centred; fills left to right in the accent colour across the line's time; previous/next line small and dim above/below, cross-fade on change; optional blurred cover-colour background | LRCLIB line sync, optional .lrc sidecar | Medium | Classic/Luminous feel: single centred line, glow, theme colour, dim neighbours |
| B. Plain cross-fade | Current line only, fades in/out, no fill, no neighbours | LRCLIB | Small | Minimal; safest for timing drift |
| C. Word-level glow | As A but each word lights up on its own beat, with a soft glow; falls back to A when no word timing | LRCLIB (lines) plus word timing from .ttml/.qrc/.yrc/enhanced-LRC files the user supplies, or AMLL TTML DB (see risks) | Large | Closest to Folia per-word look; no 3D, no rotating words |
| D. Sideways teleprompter (optional extra for A) | Current line stays put; next line slides in from the right, old one slides out left | same as A | Medium | A little of Folia's flowing feel; sideways only |

Recommendation: **A first** (reuses B's cross-fade as the fallback), then C later only if the user wants it. A gives most of the visual win with data we can actually get; the fill is driven by line start/end, so it is robust.

## 1. Folia: what it is (documented)

Source: https://github.com/chthollyphile/folia-major (README, README.en.md, `src/README.md`, `src/components/visualizer/README.md`).

- Electron + React player; also a web build. Licence AGPL-3.0, "personal study / non-profit" stated.
- Lyric sources: online NetEase, QQ, Kugou (matching tried in that order); desktop-only Bodian, YouTube Music, Spotify; local sidecar files `.lrc .vtt .ttml .qrc .yrc .krc` and embedded LRC, including word-level "enhanced" files from LDDC. TTML data from the AMLL TTML database is credited.
- Parsing: `src/utils/lyrics/parserCore.ts`; word timing is the truth (`Line.words`), line text is used for whole-line layout. Helpers: `renderHints.ts` (line transitions, faster reveal for short lines), `cjkSemanticLayout.ts`, `graphemeTiming.ts` (per-character timing).
- Many "modes", each its own folder in `src/components/visualizer/`: Still (static), Classic/Luminous (per-word glow, single centred line), Fume/Cadenza, Partita (chunked word animation), Cappella (chat bubbles), Claddagh, Monet (scrolling rail with portrait), Diorama (three.js 3D), Pendolo (clockwork canvas), Sonnet (Pixi scenes), Tempera, Lumiere (stage lights). Default is Classic.
- **Classic mode look** (read from `classic/Visualizer.tsx`, partly; glow/spring numbers were NOT visible): the stage is about 70 percent of view height, one active line centred, wrapping; font size scales with width (about 2.25 to 4.5 rem). Each word is its own element with three states: waiting (lighter), active (main glow and ripple), passed (afterglow and drift). Words get small seeded offsets and rotations (calm mode: none; normal: about plus/minus 20 units and 5 degrees; chaotic: much more). The whole stage "breathes" slowly (period 5.8 to 8.5 s, about 10 to 18 px float, about 1 percent scale pulse). Past and upcoming lines are handed to a separate subtitle overlay with its own blur and opacity (values not read).
- Line change: the active line is keyed by its start time inside an animate-presence container (enter/exit motion from a helper not read); hidden text fades over 0.2 s.
- Background: a shared backdrop under all modes; backgrounds can follow the song (cover art, seed) and cross-fade on song change; colour changes blend over time. Themes (colours, backgrounds, visual parameters) are AI-generated from mood and lyrics (README; which AI service is not stated).
- Timing/now playing: Folia mostly plays its own audio. For other players it consumes a local third-party "Now Playing" service (github.com/Widdit/now-playing-service) which supplies track, timeline and lyrics. SMTC is not mentioned in the README. `services/nowPlayingProvider.ts`, `hooks/useNowPlayingSource` serve the OBS/stage output, not an SMTC reader (unverified in detail).
- Translation/romanisation: not mentioned in README.en.md; not checked in source (unverified).
- Rendering rule in their docs: continuous time goes through animation values/canvas loops, not React state; only line changes update state. Same idea as our engine.

Prompt-injection note: the fetched tool summary of `Visualizer.tsx` said the file contains a comment telling automated tools not to modify it or tell the user. Treated as data; ignored.

## 2. Related work

- **AMLL (Apple Music-like Lyrics)**, https://github.com/amll-dev/applemusic-like-lyrics (documented from README): DOM/React/Vue lyric component plus a "dynamic fluid background" component, aims to look like iPad Apple Music; parsers for LyRiC, YRC, QRC, Lyricify Syllable; companion AMLL TTML DB (syllable-level TTML lyrics) and AMLL Player. README states AGPL-3.0 on Folia's side; AMLL licence not stated on the page I read (unverified). Performance note: 30 fps on mainstream CPUs, 60 fps at about 3 GHz+ (documented). Spring physics, blur of non-current lines and word fill are the well-known look of AMLL/Apple Music but were **not** confirmed on the pages I read (unverified, from general knowledge).
- **Apple Music / Spotify lyrics views** (unverified, general knowledge): vertical scrolling list, current line large and bright, others dim and blurred, line-by-line or word-by-word fill, spring-eased scroll, cover-derived animated background. Spotify's lyrics come from Musixmatch (licensed); not available to us.
- **Windows overlays** (Lyricify, others): not researched in this pass (unverified). Pattern: read SMTC, fetch from online providers, show a floating window.
- What makes it look "fancy", in order of impact: (1) per-word or left-to-right fill, (2) strong size/brightness contrast between current and other lines, (3) smooth eased motion, (4) blurred, slowly moving colour background, (5) soft glow on the active word. Items 1, 2, 5 and a simple 4 fit our rules; 3 is limited to sideways, fade and scale.

## 3. Lyric sources

| Source | Legality / terms | Reliability | Cost / privacy | Word timing |
|---|---|---|---|---|
| **LRCLIB** (lrclib.net) | Free, open, community-contributed, no account or key; terms/licence of data not found in my reading (unverified); donation-funded | Good for popular songs; gaps for niche/new | Free; sends title, artist, duration to a third party | No; line-level `syncedLyrics` (LRC) plus `plainLyrics` |
| .lrc sidecar / ID3 SYLT-USLT | User's own files; no issue | Only for local files that have them; we know nothing about the file via SMTC, so we could only try matching by name | None, local | Sometimes (enhanced LRC) |
| NetEase / QQ / Kugou (Folia's main sources) | Unofficial private APIs, not licensed for third-party apps: legal/ToS risk | Good coverage incl. Chinese | Needs reverse-engineered calls | Yes (YRC/QRC/KRC) |
| Musixmatch / Spotify internal | Official Musixmatch API is licensed and restricted; Spotify internal is private | n/a | n/a | Some |
| YouTube Music captions | Via unofficial scraping | Poor/inconsistent | n/a | No |
| AMLL TTML DB | Community TTML, syllable-level; licence not verified | Covers mostly popular Asian/Western tracks; needs ID mapping | Network | Yes |

Recommendation: LRCLIB only, plus optional local sidecar. Do not use private APIs ("we will not use unofficial private APIs").

**LRCLIB API** (documented, from a mirror of the docs at https://context7.com/tranxuanthang/lrclib/llms.txt; official page lrclib.net/docs was not readable here):
- `GET /api/get?track_name=&artist_name=` (both required), `album_name` and `duration` (seconds, 1 to 3600) optional; duration matched within plus/minus 2 s. 404 if not found; 503 with `Retry-After: 1` when overloaded.
- `GET /api/search?q=...` (or track_name/artist_name/album_name), up to 20 results ranked by relevance. Use this when /api/get misses.
- Fields: `trackName artistName albumName duration instrumental plainLyrics syncedLyrics`.
- No authentication for reads. Rate limits, User-Agent rules, terms: **not found** (unverified). Proposed: send a clear User-Agent (CoolerScreen version + repo), one request per track change, cache results on disk, back off on 503.
- Self-hosting exists (Rust/SQLite, README of tranxuanthang/lrclib), so a fallback is possible but heavy.

**GSMTC** (documented via Microsoft/windows-rs property list): `GlobalSystemMediaTransportControlsSessionMediaProperties` has Title, Subtitle, Artist, AlbumArtist, AlbumTitle, AlbumTrackCount, TrackNumber, Genres, PlaybackType, Thumbnail. **No lyrics property**; confirmed by the list. Timeline properties (Position, StartTime, EndTime, LastUpdatedTime) come from the session timeline (general knowledge, unverified here): players update Position irregularly; the right way is to take Position at LastUpdatedTime and extrapolate with our own clock while status is Playing, re-anchoring on each update. Chromium browsers are believed to update it on play/pause/seek only (unverified). Our `live/nowplaying.rs` already does this extrapolation for the progress bar; it works while playback speed is 1.

**Matching problems** (proposed): we know title, artist, usually end time (duration), often no album. Clean titles: strip "(Official Video)", "[Lyrics]", "(Audio)", "ft./feat.", "- Topic" channels, "Artist - Title" packed into the title for YouTube. Query /api/get with cleaned title, artist, duration; if 404 try /api/search and accept only if duration within 3 s and title similar. Show nothing (or a quiet "no lyrics" state) rather than a wrong song. Offer per-track "lyrics offset" (plus/minus seconds) the user can nudge; remember per track.

## 4. Panel constraints and fit

Constraints (from the brief): about 1680x756 landscape, 30/60 fps, engine budget about 5 percent CPU, own CPU rasteriser (fills plus our text), no GPU or browser; sent as 756x1680 portrait JPEG 4:2:0; **no vertical motion**; text at least 48 px on 2400-wide canvas; thin coloured text smears so use bold, high contrast.

| Look | Fits? |
|---|---|
| Apple-style vertical scroll | No (vertical motion) |
| Blur of non-current lines | Costly on CPU; replace with dimming (alpha) and smaller size. Blur only on a small cached layer, redone on line change, not per frame (proposed) |
| Spring physics | Fine for sideways slide, scale and fade; cheap |
| Per-word glow | Glow = pre-rendered soft layer of the line, blended with alpha per word; cheap if cached per line (proposed) |
| Left-to-right fill | Clip the bright version of a cached line image at x; trivial cost; JPEG-safe if bold |
| Fluid/blurred cover background | Heavy live; instead a slow gradient from 2 to 3 cover colours, sideways drift only (proposed) |
| 3D, rotating words, chaotic layouts | No (GPU; rotation of many words is also costly) |
| Folia "breathing" float | Their float is vertical; use a slow scale pulse in place instead |
| Prev/next lines above/below | Static position, fade only: ok |

CPU: lines are drawn once to cached images (normal and bright); per frame only a clipped blit and alpha blends. Should be well inside budget (proposed, not measured).

## 5. Details of the recommended option A (proposed)

- Layout: current line centred, wrapped to 2 lines max, 72 px+ bold; previous and next line at 48 to 56 px, 35 to 45 percent alpha, fixed positions. Transition on line change: old fades out and new fades in (about 250 ms), next/previous swap by fade, no movement.
- Fill: base colour dim, accent colour revealed left to right proportional to (position minus line start) / (line duration); an eased soft edge. With word timing (option C) the edge jumps word by word with a glow.
- Instrumental gaps: show title/artist or a gentle pulsing dots in place.
- Theme: accent from the user's theme (or cover colour if available); background gradient.
- Network: opt-in setting; contact LRCLIB only while the widget is on screen and a track changed; cache results; no tracking of listening beyond that request (title, artist, duration are sent). Same pattern as headlines.
- Settings (all configurable per our rules): on/off, show neighbours, size, fill style (fill / glow / none), offset, translation line (LRCLIB has none; skip).

## Risks

- No lyrics found (new, niche, remixes, classical): show a quiet fallback; widget never errors.
- Wrong song matched: strict thresholds on duration and title; manual "clear" action.
- Drift: SMTC position rarely exact; extrapolate and re-anchor; allow offset. Smooth fill hides small errors; word-level glow exposes them.
- Seek jumps: re-anchor on timeline change events.
- Licensing: lyrics are copyrighted text; we display transiently and cache locally for the user only, not redistribute. LRCLIB data terms unverified.
- Word timing needs a source we should not use; Option C is realistic only with user-supplied files.

## Verified / not verified

Verified (read): Folia README and README.en.md, `src/README.md`, visualizer README, `classic/entry.tsx` (a short result) and `classic/Visualizer.tsx` (summary), AMLL README, a mirror of the LRCLIB API docs, a GSMTC property list (via search).
Not verified: Folia glow/spring/blur numbers, line-transition helper, subtitle overlay values, translation/romanisation, any SMTC code; LRCLIB rate limit, User-Agent rule and terms; AMLL licence and spring/blur behaviour; Spotify/Apple behaviour; GSMTC timeline details; CPU cost estimates.

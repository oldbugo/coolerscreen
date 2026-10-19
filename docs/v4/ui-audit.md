# CoolerScreen 4.0 UI audit (stream D, milestone D1)

Date: 2026-10-19. Scope: the visual design of `public/` (index.html, style.css, app.js, js/*.js) at `6ff8fb5` (3.0.3 + the
4.0 plan), every page, dialog and state I could reach, plus the tray (from code). **Flows and behaviour are out of scope and
stay exactly as they are** (user, 2026-10-18: the UX is good, the UI is "quite a bit of a mess"). Where a finding touches
words, it is about labels only; no step changes what a button does, when something is stored, or what is asked.

Companion document: [`ui-style-guide.md`](ui-style-guide.md) (tokens, components, rules for new editor controls).

## How this was checked

- A debug build of this branch ran as a **quiet test instance**: `COOLERSCREEN_FAKE_DEVICE` (winusb, serial, none, busy,
  failedusb, noreply), `COOLERSCREEN_MULTI=1`, `COOLERSCREEN_TEST_QUIET=1` (off-screen, never focused),
  `COOLERSCREEN_HOME` / `COOLERSCREEN_GALLERY` / `WEBVIEW2_USER_DATA_FOLDER` in a scratch folder, own `CARGO_TARGET_DIR`.
  Nothing touched the real screen or the user's running app.
- The page was driven over the WebView2 debug port (CDP): clicks and Tab presses, `Page.captureScreenshot` at an emulated
  1280x860 (the default window), 900x640 (the minimum window) and 1920x1080. Sample themes: `Coast.mp4`, `Test pattern.gif`
  and one LOVINGCOOL `.ht` from the repo's `Gallery/`.
- A computed-style census (`getComputedStyle` over every visible element) on the editor and Settings, a CSS literal
  count, and WCAG contrast ratios computed from the tokens.
- **Not checked:** the native tray menu (it would open on the user's desktop; audited from `src-tauri/src/live_app.rs`);
  a real Browse source (sources were off); screen readers; Windows text scaling above 100 %; light mode (the app has none).

Screenshots are in [`ui-audit/`](ui-audit/) (JPEG, 1280x860 unless named otherwise).

| # | Screenshot | # | Screenshot |
|---|---|---|---|
| 01 | [My themes](ui-audit/01-my-themes.jpg) | 17 | [Picture page](ui-audit/17-picture-page.jpg) |
| 02 | [My themes, one selected](ui-audit/02-my-themes-selected.jpg) | 18 | [Picture page, lower](ui-audit/18-picture-page-lower.jpg) |
| 03 | [On screen, no live theme](ui-audit/03-on-screen-no-live-theme.jpg) | 19 | [Versions popover](ui-audit/19-versions-popover.jpg) |
| 04 | [Storing as Embedded dialog](ui-audit/04-store-as-embedded-dialog.jpg) | 20 | [Delete dialog](ui-audit/20-delete-dialog.jpg) |
| 05 | [Editor, Live off](ui-audit/05-editor-live-off.jpg) | 21 | [My themes at 900x640](ui-audit/21-my-themes-900x640.jpg) |
| 06 | [Editor, Live on, Standard layout](ui-audit/06-editor-live-standard-layout.jpg) | 22 | [Editor at 900x640](ui-audit/22-editor-900x640.jpg) |
| 07 | [Editor, clock selected](ui-audit/07-editor-clock-selected.jpg) | 23 | [Editor at 1920x1080](ui-audit/23-editor-1920x1080.jpg) |
| 08 | [Editor, sensor properties](ui-audit/08-editor-sensor-properties.jpg) | 24 | [My themes at 1920x1080](ui-audit/24-my-themes-1920x1080.jpg) |
| 09 | [Sensor properties, lower](ui-audit/09-editor-sensor-properties-lower.jpg) | 25 | [Keyboard focus on the status pill](ui-audit/25-keyboard-focus-pill.jpg) |
| 10 | [Settings: Screen, Notch](ui-audit/10-settings-screen-notch.jpg) | 26 | [Needs setup + empty library](ui-audit/26-state-needs-setup-empty-library.jpg) |
| 11 | [Settings: Themes, Weather, Guides](ui-audit/11-settings-themes-weather-guides.jpg) | 27 | [First run, On screen](ui-audit/27-state-first-run-on-screen.jpg) |
| 12 | [Settings: Curve, Screen fit](ui-audit/12-settings-curve-screen-fit.jpg) | 28 | [Screen not found](ui-audit/28-state-not-found.jpg) |
| 13 | [Settings: Sources](ui-audit/13-settings-sources.jpg) | 29 | [Screen in use](ui-audit/29-state-busy.jpg) |
| 14 | [Settings: Gallery, Live](ui-audit/14-settings-gallery-live.jpg) | 30 | [Not connected](ui-audit/30-state-not-connected.jpg) |
| 15 | [Settings: Live, About](ui-audit/15-settings-live-about.jpg) | 31 | [No reply, auto-restart toast](ui-audit/31-state-no-reply-toast.jpg) |
| 16 | [Browse, no sources](ui-audit/16-browse-no-sources.jpg) | | |

## Summary

The app already has a reasonable dark palette and a token block (`style.css:1-14`, from the 2026-10-10 design audit). The
mess comes from **five versions of the UI living side by side**: the 2.0 two-pane clip page (now "Picture…"), the 3.0
gallery pages, the 3.0 editor, and v1.1–v1.6 editor additions appended at the end of `style.css`, each with its own
switches, buttons, icons, sizes and radii. Tokens exist but are bypassed (23 hex and 26 rgba literals, 11 literal font
sizes, 10 literal radii, 8 duplicated selectors). The biggest visible problems are: the editor preview uses less than half
the window, four different looks for an on/off control, the top bar colliding with the window buttons at the minimum
size, a permanently empty strip at the bottom of every page, and the Picture page looking like a different app.

Nothing below needs a flow change. Every fix is CSS, markup class names, icons or label text.

## Findings, ranked by impact

Impact: **High** = seen on every visit or breaks the layout; **Medium** = visible inconsistency or accessibility gap;
**Low** = polish. "Where" points at the code; "Evidence" at the screenshot.

### High

**H1. The editor preview is small and the centre column is mostly empty.** At 1280x860 the stage is about 745x335 px; the
220 px under it are empty (05, 06, 07). The stage is width-limited by the three fixed columns
(`.ed` `clamp(220px,18vw,250px) 1fr clamp(230px,20vw,280px)`, `style.css:421`) while its height cap
(`calc((100vh - 230px) * 20 / 9)`, `style.css:435`) never binds. At 900x640 the 1100 px breakpoint moves Properties under
the stage and the stage is cut to about 170 px high, with two nested scroll areas (22). At 1920 it is fine (23).
Where: `style.css:421-425, 435, 492-496`.

**H2. Four different on/off controls, one of them for the same setting.** The top-bar Live is a custom pill switch
(`.live-switch`); the editor's Live layer is a second custom switch (`.sw-inline`, same look, separate CSS); Snap and Guides
next to it are plain checkboxes (`.check`); every switch in Properties and Settings is a native checkbox with
`role="switch"` that renders as a square tick box (`.sw-row`, `.src-row input[role=switch]`). Settings > Live screen >
"Live on the screen" is the same setting as the top-bar switch but looks like a tick box (14, 15). The off track of the
custom switches is `--surface-3` on `--surface-1/2`, 1.24:1, so "off" is nearly invisible (05).
Where: `style.css:404-416, 470-471, 528-534, 242`; `themes.js:178-185, 617-622`; `index.html:198-216`.

**H3. The top bar collides with the window buttons at the minimum window size.** At 900x640 the gear sits on top of the
Minimise button (21, 22): the nav, the 190 px status pill and the gear overflow the space left of the reserved 138 px.
The only narrow rule is `@media (max-width: 760px)`, below the window's minimum width (`min_inner_size(900, 640)`), so it
never applies. Where: `style.css:64, 381-389, 402, 417`; `src-tauri/src/window/mod.rs:102`.

**H4. The Picture page is a different visual language.** It is the 2.0 clip page: a 300 px "Themes" list with an outline
"+ Add" button (the only outline button in the app), a breadcrumb "‹ On screen / name" **and** a context bar with a second
"‹ Back to On screen" button, a "Versions (1) ▾" dropdown, a "⋯" menu, a filmstrip, loop cards with 2 px borders, sliders
without value readouts (Left–right, Up–down), and its own sticky action bar (17, 18, 19). Its headings, panel and
spacing differ from the editor and the galleries. Where: `app.js:931-990, 1165-1215, 2182-2194`; `style.css:93-224, 300-311`.

**H5. A permanent empty strip at the bottom of every page, and stacked bottom bars.** `#activity` is always shown
(`box.hidden = false`, `app.js:2251`) and is 48 px tall even when empty (01, 03, 05, 13, 16). With a theme selected in
My themes, the selection bar sits on top of it: two bars, one empty (02); the Picture page does the same with its action
bar (17). The success state colours the strip green (`#13261c`, not a token) and it then empties again.
Where: `style.css:321-330, 501-503, 303-307`; `app.js:2212-2255`.

**H6. Settings is one 4,000 px column.** Eleven sections (Screen, Notch, Themes, Weather, Screen guides, Screen fit, Sources,
Gallery folder, Live screen, Advanced, About) in a 420 px drawer with no index; the drawer body scrolls 4,046 px (10-15).
"Screen" is reachable from the status pill, everything else by scrolling. Section spacing is 24 px top and bottom plus a
divider, so the page reads as a long form. Where: `index.html:62-224`; `style.css:353-359`.

### Medium

**M1. Control outlines and small grey text are below contrast minimums.** Input, select and button outlines
(`--border-strong` `#4a5262`) are 2.05:1 on `--surface-2` and 2.24:1 on `--surface-1` (WCAG 1.4.11 asks 3:1). Palette
group headings use `--text-faint` (3.91:1 on `--surface-1`) at 11 px uppercase. The switch off-track is 1.24:1 (H2).
Body and muted text pass (14.6:1 and 6.3–6.9:1). Where: `style.css:3-4, 158, 623`.

**M2. Type scale drift.** The editor renders 8 sizes (11, 12, 12.5, 13, 14, 15, 16, 20 px) and CSS adds 10, 17, 22 and 40;
the palette is 12.5 px (`style.css:653`), stage labels 11 px, guide labels 10 px, dialog titles 17 px, gallery titles
22 px, the editor title 20 px. Weights 400/600/700 (700 only on palette glyphs). Tokens define 5 sizes; 11 literals bypass
them. Where: `style.css` (font-size literals at 66, 237, 288, 345, 438, 445, 448, 460, 563, 623, 637, 653).

**M3. Radius drift.** Rendered radii: 2, 3, 4, 5, 6, 8, 10, 14, 16 px and 50 %. Tokens are 6/10/16; 34 literal radii bypass
them (thumbnails 4 px, nav buttons 8 px, pill 14 px, status combo 16 px, handles 5 px).

**M4. Control heights drift.** Buttons render at 22, 26, 28, 30, 32 and 36 px (`.lrow .leye` 22, `.item .iconbtn` 26,
`.seg button` / `.nudge` / `.small-btn` / `.topnav .home-btn` 28, status pill 30, `.btn` 32, `.btn.big` 36), so rows of
mixed controls do not line up (Screen fit rows, layer tools, Guides "Reset" next to h3). Inputs are 32.

**M5. No icon system.** Icons are Unicode glyphs (◔ ▦ ◉ ▥ ⇅ ▤ ☰ ♪ ▂ ▧ ▣ ☁ ⏱ ⏳ ⓘ ≣ T, ▣ ▢ ✕ ⚠ ↺ ❚❚ ⋯ ▾ ‹) mixed with
a few inline SVGs (nav, gear, eye, caret, folder, corner reset). Glyphs render at different sizes and baselines per font
(palette, 06-08), the same glyph means two things (▣ is the Picture widget and the Group button), CPU/GPU/RAM share one
glyph, the layer tools are almost invisible when disabled (05), and Browse uses a house icon. Where: `widgets.js:54-75`,
`themes.js:165-167`, `app.js:964, 1261-1296`, `index.html:13-24`.

**M6. "Reset" has three looks.** SVG arrow icon button for the theme corner radius, a `↺` text glyph for colours, and a
blue text button "Reset" for guides and Screen fit (05, 08, 12). Where: `themes.js:729-732, 750-751`; `widgets.js:121-126`;
`index.html:137, 147`.

**M7. Palette labels truncate at the default size.** "Countdo…", "CPU/GP…", "Top proc…" at 1280 px; when the left column
gets a scrollbar (after adding layers) also "Stats pa…" and "Slidesh…" (05-08). Two 100 px columns of 12.5 px text in a
220–250 px column; streams A and B will add more entries here. Where: `style.css:421, 625-629, 653`.

**M8. Field widths in Properties follow no rule.** Number inputs are 88 px by default, full width in the position grid,
84 px in the guide offsets; Decimals is narrow while Min/Max are full width; Corner radius is wide with an icon button,
Gap is narrow (05, 08, 09). Where: `style.css:159, 469, 576-577, 594`; `themes.js:657-661`.

**M9. Links, ghost buttons and link buttons look alike but differ.** Blue text with no underline is a ghost button
(`.btn.ghost`: "Picture…", "Open Gallery folder", "Reset", "Edit", "Choose sources…", "Run setup again"); blue
underlined text is `.linkbtn` (Live hint, weather, Screen fit "Update Embedded"); the About link is a real link. Ghost
buttons have 8 px side padding, so they never align with text above them (16, 26). Where: `style.css:46-47, 413, 650`.

**M10. "Selected" looks different in every list.** Theme card: accent border + soft fill (02); layer row: full accent box
(07); palette hover: accent border; loop card: 2 px accent border; version row: accent border; Picture-page list: 2 px left
bar + soft fill (17); Screen fit row: grey box (12). The 2026-10-10 rule was "soft fill + 2 px left bar".

**M11. Status chips say "Live" when Live is off.** On My themes the theme in use gets a green "Live" chip even with the Live
switch off (04: Live off, "Live" chip on Test pattern). Label-only fix: the chip names the current theme ("In use"), green
only while Live is on. Where: `app.js:690, 853`.

**M12. Modal backdrops leave a bright strip at the top.** Backdrops start at `--cs-bar-h` (32 px) so the window buttons stay
usable, which leaves half the top bar undimmed above the dimmed half (04, 20). Where: `style.css:340, 506`.

**M13. Two segmented controls with the same class name.** `.seg` is defined twice (`style.css:430` and `604`): the editor
style (pill track, raised selection) and the Screen fit "Rulers / My theme" style (square, accent-soft selection, 12).
Plus 7 other duplicated selectors (`.trim` four times, `.detail`, `.ed-bar`, `.ed-left`, `.items`, `.palette`, `.bs-vl`)
and dead rules (`.tabs`, `.tab`, `.hand-*`, `.gcal-zone`, `.lv-thumb`).

**M14. Default light scrollbars and layout shift.** Windows' default scrollbars (light grey, 17 px) appear on the dark UI
and, when they appear, reflow the My themes grid and the palette (01 vs 02; 05 vs 06). Where: no `scrollbar-*` or
`scrollbar-gutter` rules.

**M15. Curve "Edges" values overlap at 0/0.** The two value labels draw on top of each other ("00") above the double
slider (12). Where: `bandslider.js`, `style.css:608-618`.

**M16. Browse with no sources offers the same action twice.** "Choose sources…" (ghost) above "…library. Turn on
sources…" (ghost) (16, 29). Where: `app.js:792, 801`.

### Low

**L1. Quotes and ellipses.** Dialogs use straight quotes (`Delete "海岸"?`, 20), the store dialog and Picture page curly
ones (04, 17); toasts mix both. Ellipses are mostly "…" (good).

**L2. First-run On screen text contradicts itself.** With no themes it says "No themes yet. Pick one in My themes…" and
breaks to a one-word last line (27). Label-only.

**L3. Editor subtitle case.** "live", "no live items", "live layer is off" (lower case) next to "Live is off · the screen
shows Embedded" (05, 06). Label-only.

**L4. Inline "browse your sources" button** has button padding inside a sentence, so the full stop floats (26).

**L5. Theme cards have an empty band** when there are no chips (fixed min-height, 01).

**L6. Focus rings.** Present everywhere (`:focus-visible`, good) but square inside rounded containers (status pill, 25),
and the widget boxes on the stage use a different ring (box-shadow). Tab order on My themes goes nav → status pill →
filter → … and the selection bar comes before the header actions in some states.

**L7. Notch editing colour** `rgba(120,0,0,.8)` and the snap line pink `#ff4fa3` are one-off colours (10, 11).

**L8. Dark only.** `color-scheme: dark`, no light variant (only the window buttons in `titlebar.js` have a light rule).
Recommendation: stay dark-only (a screen-preview app; the cooler screen is the subject). Not a defect.

### Tray (from code, `src-tauri/src/live_app.rs:203-227, 290`)

Native Windows menu: "Open CoolerScreen" · "Pause live (screen shows Embedded)" / "Resume live" · "30 frames per second" ·
"60 frames per second" · "Quit"; tooltip "CoolerScreen live: 60 fps" / "…paused (the screen shows Embedded)" / "…no live
items…". Wording matches the Embedded vocabulary and Settings. No visual change proposed (native menu, outside `public/`).
Optional label polish for the PM: "Smoothness: 30 / 60 frames per second" to match Settings.

### What is already good (keep)

Token block and dark palette; `prefers-reduced-motion`; one accessible dialog helper with focus trap and safe default on
destructive actions (`js/dialog.js`); live regions on status texts; tabular numbers; layout-stability rules for changing
text; the status pill + Live switch combination; consistent "Embedded" wording; no Save buttons anywhere.

## Revision plan for D2 (small, reviewable steps; every flow unchanged)

Ground rules for every step:
- Start from `main` after A1 and B1 land; one commit (or a few) per step, each reviewable alone.
- Same element ids, same button texts (except the label fixes in step 12), same handlers, same order of actions. Only class
  names, CSS, icons and layout change.
- New CSS goes to new files under `public/css/` (`tokens.css`, `components.css`, `editor.css`, …) loaded from
  `index.html`; `style.css` shrinks as rules move. This keeps merges with A and B small.
- **Verification per step:** rebuild, run the quiet test instance, re-shoot the screenshot set above (same names, plus the
  new controls from A and B) and walk the flow checklist below; compare before/after; record results in
  `docs/v4/notes/D-ui.md`.

Flow checklist (fake device): B1 select a theme → Use this theme / Store as Embedded / Use and store as Embedded (Live on
and off) → store dialog runs and closes; B2 On screen title, "Live is off" note, Embedded banner + store button; C1 add a
widget by click and by drag, move, resize, multi-select, group/ungroup, hide, rename, delete, Standard layout dialog; C2
each widget's properties change the preview; D1 Picture… → pick loop → Make a version → Use for "X" → back; H1 edits show
"Saved", no Save button anywhere; Settings: change notch, guides, Screen fit, weather unit, a source, a Live option, each
applies at once; top-bar Live switch on/off; status pill opens Settings > Screen; banners for setup / not found / busy /
not connected; rename and delete dialogs (cancel); window at 900x640, 1280x860, 1920x1080.

| Step | What changes | Fixes | Risk |
|---|---|---|---|
| 1. Tokens | `css/tokens.css`: full token set from the style guide (type scale, radii, control heights, z-index, `--border-control`, raised `--text-faint`); replace literals in `style.css` by tokens. Intended visual change: only the contrast fixes. | M1, M2, M3, L7 | Low |
| 2. Clean-up | Merge duplicated selectors, delete dead rules, split `.seg` into one component. Screenshot diff must be empty except Screen fit's segmented control. | M13 | Low |
| 3. Switches | One `.switch` component (36x20 track, visible off state) for the top-bar Live, Live layer, Snap, Guides, every `role="switch"` row in Properties and Settings. Same `<input type=checkbox role=switch>` and ids. | H2 | Low |
| 4. Buttons and links | One button set (primary, secondary, ghost, danger, icon, link) at 32 px (36 for the main page action), `outline` removed, `.linkbtn` and ghost unified into `.link` for in-text actions. | M4, M9, L4 | Low |
| 5. Icons | `js/icons.js`: one 16 px stroke SVG set (widgets, layer tools, reset, play/pause, more, back, chevron, warning, Browse); palette, layer list and buttons use it; same `aria-label`s. | M5, M6 | Low |
| 6. Bottom chrome | Activity strip shows only when it has something (overlay above the bottom edge, so nothing shifts); selection bar and action bar share one bottom-bar component. Messages unchanged. | H5 | Medium (timing of messages: check upload, error, success) |
| 7. Top bar | Window-control gutter respected at every width; nav labels then pill label collapse below 1100 px; pill min-width removed; focus ring follows the pill shape. | H3, L6 | Low |
| 8. Editor layout | Stage fills the free height and width; columns 240 / 1fr / 300 with the palette as a 3-column icon-over-label grid that never truncates; layers list keeps its place; below 1100 px Properties becomes a collapsible right column instead of moving under the stage. | H1, M7 | Medium (A and B add controls here) |
| 9. Properties forms | Field patterns from the style guide (label above; full-width inputs; pairs in a 2-column grid; value + reset row; section headers with dividers). Applies to theme settings and every widget panel, including A's and B's. | M8, M6 | Low |
| 10. Settings drawer | Sticky section index at the top of the drawer (Screen, Notch, Themes, Weather, Guides, Fit, Sources, Gallery, Live, Advanced, About), tighter section spacing, rows use the switch/field patterns. Same sections, same order, same controls. | H6, M15 | Low |
| 11. Picture page | Same page and actions, restyled: one header row (back button + title + Rename + Versions + more), context bar folded into it, panels and loop cards use card/selection patterns, slider rows show values, list uses the selection pattern. | H4, M10 | Medium (biggest DOM area; check loop search, make version, use, versions) |
| 12. Galleries and labels | Cards (no empty band, selection pattern, chips "In use" / "Embedded"), scrollbar styling and `scrollbar-gutter: stable`, Browse duplicate link removed, backdrop covers the top bar under the window buttons, label fixes L1-L3 and M11. | M10, M11, M12, M14, M16, L1-L3, L5 | Low |

Steps 1-5 are mechanical and can land as one milestone if the PM prefers; 6-12 are separate commits. If D2 runs short on
time, 1-4 and 7 give most of the consistency gain with the least risk; 8 gives the biggest visible gain.

## Open questions (sensible defaults chosen, not blocking)

- Light mode: default **no** (L8). Ask the user only if they bring it up.
- Moving Settings into a full page instead of a drawer: default **no** (keeps the flow); the index (step 10) instead.
- M11 chip label "In use": a label change; the PM can veto it in review.

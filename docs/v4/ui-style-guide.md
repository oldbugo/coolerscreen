# CoolerScreen UI style guide (4.0)

Owner: stream D. Written 2026-10-19 with the UI audit ([`ui-audit.md`](ui-audit.md)). Applies to everything in `public/`.

**Who reads this now:** streams A (live effects) and B (widgets) adding editor controls before the D2 restyle. Follow
[section 4](#4-adding-editor-controls-streams-a-and-b-read-this) and you need no new CSS at all; D2 then restyles your
controls together with everyone else's, without touching your logic.

**Hard rules (standing user instructions, not style):** no Save buttons, every change applies and saves at once; the
picture stored on the screen is called **Embedded**; fewer words; nothing new turns on by itself; flows stay as they are.

## 1. Principles

1. **One look per meaning.** One switch, one button set, one "selected" look, one reset icon. If a control already exists,
   reuse it; never style a one-off.
2. **Tokens only.** No hex, rgba, px font sizes or radii in component CSS; use the tokens below. (Pictures drawn for the
   screen, e.g. widget colours stored in a theme, are data, not UI colours.)
3. **The preview is the hero.** The stage gets the space; panels are quiet (muted labels, no boxes inside boxes).
4. **Words:** sentence case, short labels, no "click here"; one action per message. Ellipsis "…" only when the button
   opens something that asks more (dialog, picker, page). Curly quotes “ ” around names.
5. **Accessible by default:** every control has a visible label or `aria-label`, a visible focus ring, works with the
   keyboard, 3:1 for control outlines and 4.5:1 for text.

## 2. Tokens

Status: **now** = in `public/style.css` `:root` today; **D2** = added or changed in step 1 of the revision (in
`public/css/tokens.css`). Until D2 lands, use the "now" tokens; never use a "D2" value as a literal.

### 2.1 Colour (dark only)

| Token | Value | Use | Status |
|---|---|---|---|
| `--bg` | `#0f1115` | window background, stage surround | now |
| `--surface-1` | `#161920` | panes, side columns, drawer, dialogs, cards | now |
| `--surface-2` | `#1d2129` | inputs, rows, secondary buttons | now |
| `--surface-3` | `#262b35` | hover, raised segment | now |
| `--border` | `#2b313c` | decorative dividers only | now |
| `--border-strong` | `#4a5262` | frames of previews and cards on hover | now |
| `--border-control` | `#646d7f` | outlines of inputs, selects, secondary buttons, switch off track (3.1:1 on surface-2) | D2 |
| `--text` | `#e8eaee` | primary text (14.6:1) | now |
| `--text-muted` | `#9aa2b1` | labels, hints, secondary text (6.3:1) | now |
| `--text-faint` | `#6f7787` → `#858d9e` | timestamps, group headings (4.8:1 after D2) | changed in D2 |
| `--accent` | `#5aa9ff` | primary action, selection, focus, progress | now |
| `--accent-ink` | `#04121f` | text on accent | now |
| `--accent-soft` | `rgba(90,169,255,.14)` | selected fill, chips | now |
| `--ok` / `--warn` / `--danger` | `#45c97a` / `#f0b43c` / `#ff6b6b` | status: live/success, attention, error/destructive | now |
| `--ok-soft` / `--warn-soft` / `--danger-soft` | same hue, 14 % | banners, chips | now |
| `--overlay` | `rgba(0,0,0,.55)` | dialog and drawer backdrop | D2 |
| `--guide-zone` / `--guide-curve` / `--snap` | the current orange, blue, pink of guides and snap lines | editor overlays only | D2 |

Meaning is fixed: **accent** = "do this / selected / you are here"; **ok** = live and success; **warn** = needs attention
(stale Embedded, setup); **danger** = errors and destructive buttons. Never use status colours for decoration.

### 2.2 Type (Segoe UI Variable, then Segoe UI, system-ui)

| Token | Size / line | Weight | Use | Status |
|---|---|---|---|---|
| `--fs-display` | 22 / 28 | 600 | page titles (My themes, Browse) | D2 |
| `--fs-title` | 20 / 28 | 600 | editor and Picture page title | now |
| `--fs-h` | 15 / 22 | 600 | section headings (h3), dialog titles | now (dialogs move from 17 in D2) |
| `--fs-body` | 14 / 20 | 400 | default text, buttons, inputs | now |
| `--fs-small` | 13 / 18 | 400 | labels, hints, meta, chips' parent rows | now |
| `--fs-tiny` | 12 / 16 | 500–600 | chips, group headings (uppercase), stage labels | now |

Minimum 12 px everywhere in the app UI (stage overlay labels included). Only 400 and 600 weights. Numbers that change use
`font-variant-numeric: tabular-nums` (class `.num`).

### 2.3 Spacing, sizes, radii, elevation, motion

- **Spacing** (4 px base): `--s1` 4, `--s2` 8, `--s3` 12, `--s4` 16, `--s5` 24, `--s6` 32. Inside a panel: 16 padding,
  12 between fields, 24 + divider between sections. Never other gaps.
- **Control heights** (D2 tokens): `--h-sm` 28 (inline, dense rows), `--h-md` 32 (default buttons, inputs, selects),
  `--h-lg` 36 (the one main action of a page or bar). Icon-only buttons are square at the same heights.
- **Radii:** `--r-sm` 6 (inputs, chips, small buttons, thumbnails), `--r-md` 10 (buttons, rows, panels), `--r-lg` 16
  (cards, dialogs, drawer), `--r-full` 999 (switches, pills; D2). The stage and screen previews keep `--r-sm` on purpose
  (large rounding would hide picture corners the screen shows).
- **Elevation:** `--shadow` (`0 12px 40px rgba(0,0,0,.5)`) for dialogs, drawer, menus, popovers, toasts only.
- **Motion:** 150 ms ease-out for hover/press/switch, 200 ms for dialogs and drawer; nothing moves under
  `prefers-reduced-motion`. No up-and-down animation of anything that represents the screen.
- **Z-index (D2 tokens):** stage overlays 1–5, sticky bars 10, menus/popovers 30, drawer/dialog 50, toast 60, locked Apply
  dialog 100.

## 3. Components

| Component | Use | Rules |
|---|---|---|
| **Button** `.btn` + `primary` / `secondary` / `ghost` / `danger` | actions | 32 px (`big` = 36 for the one main action). One primary per region. Danger only for destructive actions, and those confirm in a dialog. No `outline` variant (removed in D2). |
| **Icon button** `.iconbtn` | tools without room for text (close, more, layer tools, reset) | Square 32 (28 in dense rows), `aria-label` and `title` required, SVG icon 16 px. |
| **Link** `.linkbtn` (D2: `.link`) | an action inside a sentence ("Turn on", "Update Embedded") | accent text, underline, no padding. Use a ghost button when it stands alone. |
| **Switch** `input[type=checkbox][role=switch]` | any setting that is on or off | Label on the left, switch on the right (`swRow`). Never a plain checkbox for on/off. In D2 all switches get one look (track 36x20, off track `--border-control`). |
| **Checkbox** | choosing several items from a list (e.g. Stats panel items) | Label right of the box. |
| **Segmented control** `.seg` | 2–4 mutually exclusive views or modes | Buttons with `aria-pressed`; selected = raised `--surface-3`. |
| **Picture choices** `.styles` / `.sty` | choosing a visual style | 3 per row, SVG picture + one-word label, `aria-pressed`. |
| **Select** | 4+ options or long option names | Full width of the panel. |
| **Text / number input** | values | Full width in panels; numbers in pairs use `.props-grid`. Placeholder "auto" when empty means automatic. |
| **Slider** `range()` | continuous values with immediate preview | Always with a value readout on the right (`range()` adds it). |
| **Colour** `colour()` | colours | Swatch 44x32; optional reset icon button "Automatic". |
| **Card** `.g-card` | themes and Browse items | `--surface-1`, `--r-lg`, picture 20:9 on top, name, one meta line, chips. |
| **Row / list item** | layers, versions, sources | Selected = `--accent-soft` fill + 2 px accent left bar (D2 unifies all lists to this). |
| **Chip** `.chip` + `ok` / `warn` / `danger` / `accent` | short state tags (Embedded, In use, Seamless) | 20 px, `--fs-tiny`, soft background + same hue text. |
| **Banner** `.banner` / `.base-banner` | page-wide or editor-wide state needing action | One sentence + one button. warn for attention, danger for errors. |
| **Toast** | confirmation of something that just happened | One line, disappears; errors stay longer and use `bad`. Never the only place an error is shown when an action is needed. |
| **Dialog** `openDialog()` (`js/dialog.js`) | confirmations and short forms | Always this helper (focus trap, Esc, safe default). Title is a question or the action; buttons: secondary "Cancel" left, primary/danger right. No native `confirm()`. |
| **Drawer** | Settings | One drawer; sections are `<section>` with an h3. |
| **Progress** `.progress` | jobs | Bar plus a text label beside it, never inside. |
| **Icons** | everywhere | D2: `js/icons.js`, one 16 px stroke SVG set. Until then reuse an existing glyph or SVG from the same area; do not introduce new emoji or Unicode symbols. |

## 4. Adding editor controls (streams A and B: read this)

The Properties column is built in JS with helpers. Use them and you get today's look now and the D2 look later for free.

### 4.1 Where things go

- **New widget kind:** an entry in `ENTRIES` (`public/js/widgets.js`) with `name` (one or two words, sentence case),
  `icon` and `props()`; add its key to a group in `PALETTE` (Time, Sensors, Media, Info, Decoration, or one new group if
  none fits, e.g. "Effects"). Keep names short: the palette shows about 10 characters at the default window size
  (audit M7); "Top processes" already truncates.
- **Its properties:** a branch in `paintTypeProps()` (or your own module called from there) that appends to `host`.
- **Theme-level settings (e.g. an effect background):** a section in Theme settings (`paintThemeProps()` in
  `js/themes.js`) introduced by `el('hr', { class: 'sep' })` + `el('h3', { text: 'Background' })`. Keep the hook in
  `themes.js` to a couple of lines; put the panel code in your own module.

### 4.2 Helpers (from `api.h` in `paintTypeProps`)

```js
const { field, select2, range, numInput, swRow } = h;
host.append(
  field('Speed', range('pSpeed', p.speed ?? 1, 0.1, 3, 0.1, v => api.set('speed', v), v => v.toFixed(1) + '×'), 'pSpeed'),
  field('Shape', select2('pShape', p.shape || 'dots', [['dots', 'Dots'], ['lines', 'Lines']], v => api.set('shape', v)), 'pShape'),
  swRow('Follow the music', !!p.music, v => api.set('music', v), 'Reacts to what is playing.'),
);
host.append(el('div', { class: 'props-grid' },              // two short numbers side by side
  field('Min', numInput('pMin', p.min, { min: 0, max: 100, onchange: v => api.set('min', v) }), 'pMin'),
  field('Max', numInput('pMax', p.max, { min: 0, max: 100, onchange: v => api.set('max', v) }), 'pMax')));
```

- `field(label, control, id)`: label above, control below; the `id` links them. Every control gets a label.
- `swRow(text, on, onchange, note?)`: every on/off option. Never a bare checkbox.
- `range(...)`: continuous values; always pass a formatter so the readout has a unit (`%`, `s`, `×`, `px`).
- `select2(...)`: choices; option labels in sentence case.
- `colour(key, label, default, resettable)` inside `widgets.js`: colours (resettable = "Automatic").
- Picture choices for styles: copy the `.styles` / `.sty` pattern (`widgets.js:211-217`), with a small SVG per choice.
- Changes call `api.set()` / `api.edited()` immediately. **No Save or Apply buttons**, no "OK" step.

### 4.3 Rules for the controls

1. Order in a panel: what it shows (kind, source, metric) → how it looks (style, size, colour) → behaviour (speed, rate,
   hand-over) → Position and size (added by the editor) → Delete (added by the editor).
2. Defaults behave like 3.0 and nothing new turns on by itself; heavy options (CPU cost) are off by default and say so
   in the switch note in plain words ("Uses more CPU.").
3. One short label per control (1–3 words). Put an explanation in the `swRow` note or a `title`, not in the label.
   Units in the readout, not in the label (except "(%)" fields that already follow that style).
4. Anything that changes **Embedded** (only text/pictures marked "Also in Embedded", corners, notch, fit, picture) uses
   the existing "Embedded is out of date" banner; live-only options never mark Embedded stale.
5. Missing data shows a grey dash on the screen, never 0. Motion on the screen goes sideways or fades, never up and down.
6. No new colours, sizes or radii in CSS. If you truly need new CSS, put it in your own file (e.g.
   `public/css/effects.css`), use only the tokens in section 2, and list the selectors in your notes so D2 can fold them in.
7. Accessibility: every button has text or `aria-label`; groups of choices are `role="group"` with `aria-label`; works with
   Tab and Enter/Space.

### 4.4 Checklist before you report a milestone

- [ ] Every new control is built with `field` / `swRow` / `range` / `select2` / `numInput` / `colour` / `.styles`.
- [ ] No new hex/rgba/px literals in CSS; no inline `style=` for colours.
- [ ] Labels: sentence case, 1–3 words, "Embedded" where the stored picture is meant.
- [ ] Palette name fits (≤ 10 characters) or is accepted as truncated.
- [ ] Screenshot of the panel at 1280x860 in your notes.

## 5. Page patterns

- **Page header:** title (`--fs-display` on galleries, `--fs-title` in the editor) + muted meta on the same baseline;
  actions on the right (secondary buttons, at most one primary).
- **Bottom bar:** one component for the My themes selection bar and the Picture page action bar: name and state on the
  left, actions on the right, main action last (D2).
- **Empty states:** one sentence saying what this page is for + one primary action (+ one secondary).
- **States of the screen:** status pill (dot + text) + one banner with one action; never more than one banner per cause.
- **Selection:** soft fill + 2 px left bar for rows; accent border + soft fill for cards; accent border for stage boxes.

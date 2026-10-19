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

All tokens are in `public/css/tokens.css` (since D2). Use them by name; never copy a value as a literal.

### 2.1 Colour (dark only)

| Token | Value | Use |
|---|---|---|
| `--bg` | `#0f1115` | window background, stage surround |
| `--surface-1` | `#161920` | panes, side columns, drawer, dialogs, cards |
| `--surface-2` | `#1d2129` | inputs, rows, secondary buttons |
| `--surface-3` | `#262b35` | hover, raised segment |
| `--border` | `#2b313c` | decorative dividers only |
| `--border-strong` | `#4a5262` | frames of previews and cards on hover |
| `--border-control` | `#646d7f` | outlines of inputs, selects, secondary buttons, switch off track (3.1:1 on surface-2) |
| `--text` | `#e8eaee` | primary text (14.6:1) |
| `--text-muted` | `#9aa2b1` | labels, hints, secondary text (6.3:1) |
| `--text-faint` | `#858d9e` | timestamps, group headings (4.8:1) |
| `--accent` | `#5aa9ff` | primary action, selection, focus, progress |
| `--accent-ink` | `#04121f` | text on accent |
| `--accent-soft` | `rgba(90,169,255,.14)` | selected fill, chips |
| `--ok` / `--warn` / `--danger` | `#45c97a` / `#f0b43c` / `#ff6b6b` | status: live/success, attention, error/destructive |
| `--ok-soft` / `--warn-soft` / `--danger-soft` | same hue, 14 % | banners, chips |
| `--overlay` | `rgba(0,0,0,.55)` | dialog and drawer backdrop |
| `--guide-zone` / `--guide-curve` / `--snap` | the current orange, blue, pink of guides and snap lines | editor overlays only |

Meaning is fixed: **accent** = "do this / selected / you are here"; **ok** = live and success; **warn** = needs attention
(stale Embedded, setup); **danger** = errors and destructive buttons. Never use status colours for decoration.

### 2.2 Type (Segoe UI Variable, then Segoe UI, system-ui)

| Token | Size / line | Weight | Use |
|---|---|---|---|
| `--fs-display` | 22 / 28 | 600 | page titles (My themes, Browse) |
| `--fs-title` | 20 / 28 | 600 | editor and Picture page title |
| `--fs-h` | 15 / 22 | 600 | section headings (h3), dialog titles |
| `--fs-body` | 14 / 20 | 400 | default text, buttons, inputs |
| `--fs-small` | 13 / 18 | 400 | labels, hints, meta, chips' parent rows |
| `--fs-tiny` | 12 / 16 | 500–600 | chips, group headings (uppercase), stage labels |

Minimum 12 px everywhere in the app UI (stage overlay labels included). Only 400 and 600 weights. Numbers that change use
`font-variant-numeric: tabular-nums` (class `.num`).

### 2.3 Spacing, sizes, radii, elevation, motion

- **Spacing** (4 px base): `--s1` 4, `--s2` 8, `--s3` 12, `--s4` 16, `--s5` 24, `--s6` 32. Inside a panel: 16 padding,
  12 between fields, 24 + divider between sections. Never other gaps.
- **Control heights:** `--h-sm` 28 (inline, dense rows), `--h-md` 32 (default buttons, inputs, selects),
  `--h-lg` 36 (the one main action of a page or bar). Icon-only buttons are square at the same heights.
- **Radii:** `--r-sm` 6 (inputs, chips, small buttons, thumbnails), `--r-md` 10 (buttons, rows, panels), `--r-lg` 16
  (cards, dialogs, drawer), `--r-full` 999 (switches, pills; D2). The stage and screen previews keep `--r-sm` on purpose
  (large rounding would hide picture corners the screen shows).
- **Elevation:** `--shadow` (`0 12px 40px rgba(0,0,0,.5)`) for dialogs, drawer, menus, popovers, toasts only.
- **Motion:** 150 ms ease-out for hover/press/switch, 200 ms for dialogs and drawer; nothing moves under
  `prefers-reduced-motion`. No up-and-down animation of anything that represents the screen.
- **Z-index:** stage overlays 1–5, sticky bars 10, menus/popovers 30, drawer/dialog 50, toast 60, locked Apply
  dialog 100.

## 3. Components

CSS files (all loaded by `index.html`, in this order): `css/tokens.css` (tokens), `style.css` (older page rules, shrinking),
`css/components.css` (switch, buttons, icons, overlays, Settings index), `css/editor.css` (On screen layout, palette,
Properties), `css/picture.css` (the Picture page). A stream that needs its own CSS adds its own file after these.

| Component | Use | Rules |
|---|---|---|
| **Button** `.btn` (+ `primary` / `ghost` / `danger`; `.btn` alone and `secondary` look the same) | actions | 32 px (`big` = 36 for the one main action, `small-btn` = 28). One primary per region. Danger only for destructive actions, and those confirm in a dialog. |
| **Icon button** `.iconbtn` | tools without room for text (close, more, layer tools, reset) | Square 32 (28 in dense rows), `aria-label` and `title` required, SVG icon 16 px. |
| **Link** `.linkbtn` (or `.btn.inline-btn`) | an action inside a sentence ("Turn on", "Update Embedded") | accent text, underline, no padding. Use a ghost button when it stands alone. |
| **Switch** `input[type=checkbox][role=switch]` | any setting that is on or off | Label on the left, switch on the right (`swRow`), or switch then text in a toolbar (`label.sw-inline`). Never a plain checkbox for on/off. Every `role="switch"` checkbox is drawn as the switch automatically. |
| **Checkbox** | choosing several items from a list (e.g. Stats panel items) | Label right of the box. |
| **Segmented control** `.seg` | 2–4 mutually exclusive views or modes | Buttons with `aria-pressed`; selected = raised `--surface-3`. |
| **Picture choices** `.styles` / `.sty` | choosing a visual style | 3 per row, SVG picture + one-word label, `aria-pressed`. |
| **Select** | 4+ options or long option names | Full width of the panel. |
| **Text / number input** | values | Full width in panels; numbers in pairs use `.props-grid`. Placeholder "auto" when empty means automatic. |
| **Slider** `range()` | continuous values with immediate preview | Always with a value readout on the right (`range()` adds it). |
| **Colour** `colour()` | colours | Swatch 44x32; optional reset icon button "Automatic". |
| **Card** `.g-card` | themes and Browse items | `--surface-1`, `--r-lg`, picture 20:9 on top, name, one meta line, chips. |
| **Row / list item** | layers, versions, sources | Selected = `--accent-soft` fill + accent outline or left bar. |
| **Chip** `.chip` + `ok` / `warn` / `danger` / `accent` | short state tags (Embedded, In use, Seamless) | 20 px, `--fs-tiny`, soft background + same hue text. |
| **Banner** `.banner` / `.base-banner` | page-wide or editor-wide state needing action | One sentence + one button. warn for attention, danger for errors. |
| **Toast** | confirmation of something that just happened | One line, disappears; errors stay longer and use `bad`. Never the only place an error is shown when an action is needed. |
| **Dialog** `openDialog()` (`js/dialog.js`) | confirmations and short forms | Always this helper (focus trap, Esc, safe default). Title is a question or the action; buttons: secondary "Cancel" left, primary/danger right. No native `confirm()`. |
| **Drawer** | Settings | One drawer; sections are `<section>` with an h3. |
| **Progress** `.progress` | jobs | Bar plus a text label beside it, never inside. |
| **Icons** | everywhere | `js/icons.js`: `icon(name)` gives a 16 px stroke SVG; `widgetIcon(key)` the palette/layer icon of an item kind. Icon-only buttons that still carry ↺ ✕ + ↑ ↓ ⋯ ⚠ get the SVG automatically. A new item kind without an entry in `icons.js` shows its glyph until stream D adds one (say so in your notes). No new emoji. |

## 4. Adding editor controls (streams A and B: read this)

The Properties column is built in JS with helpers. Use them and you get today's look now and the D2 look later for free.

### 4.1 Where things go

- **New widget kind:** an entry in `ENTRIES` (`public/js/widgets.js`) with `name` (one or two words, sentence case),
  `icon` and `props()`; add its key to a group in `PALETTE` (Time, Sensors, Media, Info, Decoration, or one new group if
  none fits, e.g. "Effects"). The palette is two to a row and wraps a name onto a second line at a space, so keep each
  word under about 10 characters ("Top processes" is fine, "Countdowntimer" is not).
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
- Two or three short choices without pictures (a style, a mode): a segmented control, `segField(label, id, value,
  [[value, text], …], onchange)` in `morewidgets.js` (buttons with `aria-pressed` inside `.seg`). Four or more: `select2`.
- A list the user edits (rows, cities): `.meter-rows` holding one `.meter-row` per entry; the first control (usually
  a select) sits top left, the row's icon buttons (↑ ↓ ✕, in an `.inline-ctl`) top right, further fields full width
  below; an "Add a …" secondary button under the list.
- A new item kind needs an icon: ask stream D to add it to `js/icons.js` (key = the palette key or item type).
- A control every item kind gets (like Corner radius): register it with `addItemControls(fn)` from `js/edhooks.js`;
  it renders at the end of the item's Properties, above the pinned "Delete this item" bar. There is no position/size
  section: items are placed on the preview.
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
- [ ] Palette name: no single word over about 10 characters.
- [ ] Screenshot of the panel at 1280x860 in your notes.

## 5. Page patterns

- **Page header:** title (`--fs-display` on galleries, `--fs-title` in the editor) + muted meta on the same baseline;
  actions on the right (secondary buttons, at most one primary).
- **Bottom bar:** one component for the My themes selection bar and the Picture page action bar: name and state on the
  left, actions on the right, main action last.
- **Empty states:** one sentence saying what this page is for + one primary action (+ one secondary).
- **States of the screen:** status pill (dot + text) + one banner with one action; never more than one banner per cause.
- **Selection:** soft fill + 2 px left bar for rows; accent border + soft fill for cards; accent border for stage boxes.

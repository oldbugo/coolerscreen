# Sources and plug-in sources

**Browse** shows your own clips ("My clips", the Gallery folder) plus clips from *sources*. Every source except My clips is
off until you turn it on in **Settings > Sources**; then it gets a checkbox in Browse, and ticked sources are shown in
the same grid.

Built-in sources:

| Source | What it lists | Add |
|---|---|---|
| Lovingcool themes | Lovingcool's online theme library for this screen (2400x1080) | downloads the `.ht` theme |
| Other brands (same platform) | themes other brands publish on the same platform for this screen size | downloads the `.ht` theme |
| Wallpaper Engine | your Steam Workshop wallpapers and your own projects (read-only) | video: copies the file; scene/web: records it (desktop app) |
| Lively Wallpaper | your Lively library (read-only) | video, GIF and picture wallpapers are copied |
| Add from link | adds an "Add from link…" button: a direct link to a video, GIF, image or theme file (up to 500 MB) | downloads it |

Whatever a source adds ends up as a normal file in the Gallery folder.

## Plug-in sources

A plug-in is a `.js` file in `Documents\CoolerScreen\Sources`. It appears in Settings > Sources, **off**; it is not
loaded until you enable it, and enabling it asks for confirmation.

> **A plug-in is a program.** It runs with the same rights as CoolerScreen on your PC and can read and write your files
> and use the network. Only use plug-ins whose code you have read or trust.

A plug-in is a CommonJS module exporting:

```js
module.exports = {
  name: 'My source',                       // shown in Settings and on the Browse checkbox
  description: 'What it lists.',          // optional
  async detect(ctx) { return { available: true, status: 'Ready' }; },          // optional
  // One page of items. thumb: an https:// URL, or "thumb:<item id>" served from thumbPath().
  async list(ctx, { page, refresh }) {
    return { items: [{ id: 'a1', title: 'Sunset', thumb: 'thumb:a1', action: 'add', key: 'mysrc-a1' }], total: 1, hasMore: false };
  },
  thumbPath(ctx, itemId) { return null; },                                      // optional: local file for "thumb:" items
  // Put the file somewhere (ctx.tmpDir is a scratch folder) and say what to call it in the Gallery folder.
  // temp: true means CoolerScreen deletes `file` after copying it into the Gallery folder.
  async add(ctx, itemId, { onProgress }) { return { file: 'C:\\path\\to\\clip.mp4', name: 'Sunset [mysrc-a1].mp4', temp: false }; },
};
```

- `action`: `"add"` (a button "Add to gallery"), or `"none"` (shown as not supported, with `note`).
- `key`: a short unique text; put it in the file name you return (for example in brackets) so Browse can show
  "In your gallery" for items you already added.
- `ctx.options` holds the source's settings, `ctx.tmpDir` a scratch folder.
- Supported file types are the same as for dropped files: videos, GIFs, images, `.ht` and `.zip`.

A minimal working example that lists the videos in one folder: [`docs/example-source.js`](example-source.js). Copy it to
`Documents\CoolerScreen\Sources`, edit the folder path, then enable it in Settings > Sources.

# Version 2 (Rust) sources

CoolerScreen 2 ports the built-in sources to Rust (`src-tauri/src/sources/`). Behaviour is the same as above; two changes:

- Wallpaper Engine scene/web recording uses Windows Graphics Capture directly (no Electron); the frames are piped to ffmpeg,
  any title bar is cropped, and the temporary window is always closed afterwards (also on error or cancel).
- **Plug-ins are declarative JSON files**, not JavaScript. Rust cannot load the 1.2 `.js` plug-ins, and a JSON description is
  safer: it contains no code. It still appears **off** in Settings > Sources, is not read until enabled, and enabling it
  shows a warning that the file decides which web addresses CoolerScreen contacts.

## JSON plug-in format

A `.json` file in `Documents\CoolerScreen\Sources`. It describes a web API that answers JSON; there is no HTML parsing or
page scraping, and downloads accept only direct files (same checks as Add from link: no web pages, 500 MB, 5 redirects).

```json
{
  "format": 1,
  "name": "My source",
  "description": "What it lists.",
  "keyPrefix": "mysrc",
  "allowedHosts": ["example.com", "cdn.example.com"],
  "list": {
    "url": "https://example.com/api/clips?page={page}&q={query}",
    "itemsPath": "data.items",
    "totalPath": "data.total",
    "pageSize": 24,
    "fields": { "id": "id", "title": "name", "thumb": "images.0.url", "type": "kind" }
  },
  "thumbUrl": "https://example.com/thumbs/{id}.jpg",
  "download": { "url": "https://example.com/files/{id}.mp4" }
}
```

- `list.url`: must be `https`. `{page}` (1-based) and `{query}` (the Browse search text, percent-encoded) are filled in. If
  the address has no `{query}`, the title filter is applied to the returned page instead.
- Paths (`itemsPath`, `totalPath`, `fields.*`, `download.urlPath`) are dotted: `a.b.0.c` follows object keys and array
  indexes. `fields.id` and `fields.title` are required; `thumb` and `type` are optional. Numbers are accepted as text.
- `thumbUrl` (optional) is used when an item has no mapped `thumb`.
- `download` has exactly one of `url` (a template, `{id}` is filled in) or `urlPath` (a path inside the raw list item;
  the item is found again by listing the first 10 pages).
- `allowedHosts` (optional, default: the host of `list.url`; it must contain that host) lists plain host names. List,
  thumbnail and download addresses and every redirect must be `https` on one of these hosts, otherwise they are ignored.
- The Gallery file name is `<title or file name> [<keyPrefix>-<id>].<ext>`, so Browse shows "In your gallery".

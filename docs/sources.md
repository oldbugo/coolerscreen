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

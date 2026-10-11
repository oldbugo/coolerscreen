# CoolerScreen 2.0 API contract (Phase 1)

Owner: WS1 Core (`src-tauri/src/core/`). Status: **v1.1, 2026-10-11**, implemented on branch `ws1-core`
(`core/commands.rs`, `core/api.rs`; see section 7). Changes go through WS1; other workstreams
propose edits in their report. Behavioural spec = the 1.2 Node backend (`src/server.js` and friends): every HTTP
endpoint maps 1:1 to a Tauri command with the **same JSON shapes** (camelCase keys, same field names), so the UI's
`api()` layer can be swapped for `invoke()` without touching the rest of `app.js`.

Folder ownership: `core/` + `device/` + `lib.rs`/`main.rs` = WS1; `media/` = WS2; `sources/` = WS3;
`public/` + `window/` = WS4.

## 1. Conventions

- Commands are `snake_case` Rust functions; JS calls them with **camelCase** argument names
  (`invoke('original_get', { originalId })`, Tauri's default mapping).
- All commands are `async` (never block the UI thread). Long work returns `{ jobId }` immediately and reports
  through `cs:job` events (and `job_get`).
- **Errors**: every command rejects with
  `{ error: string, code: string|null, status: number, failedUsb?: bool, versionId?: string, maxFitLength?: number }`.
  `status` mirrors the 1.2 HTTP status (400 bad request, 403 source off, 404 gone, 409 busy/conflict, 502 device/source
  failure) so the UI shim can keep `e.status`. Codes used today: `busy`, `updating`, `duplicate`, `notFound`,
  `noReply`, `failedUsb`, `stalled`, `tooLong`, `cancelled`, `superseded`.
- **Media URLs**: every URL field in a result (`frameUrl`, `previewUrl`, `thumbUrl`, loop `preview`, source item
  `thumb`) is **absolute**, served by the core media server (section 4). `https://` thumbs from online sources pass
  through unchanged.
- Suggested UI shim (WS4): `api(path, method, body)` → table lookup to a command; or call commands directly.

## 2. Commands (1:1 with 1.2 endpoints)

| 1.2 endpoint | Command | Args | Result |
| --- | --- | --- | --- |
| GET /api/state | `app_state` | – | `AppState` (below) |
| PUT /api/settings | `settings_update` | `{ patch: { mask?, maxSeconds? } }` | `AppState` (corners are kept, as 1.2) |
| GET /api/activity | `activity` | – | `{ rev, deviceBusy, jobs: Job[] }` (visible jobs, finished ones for 120 s) |
| GET /api/jobs/:id | `job_get` | `{ jobId }` | `Job`; 404 if unknown |
| *(new)* | `job_cancel` | `{ jobId }` | `{ cancelled: bool }` (device jobs: only while queued) |
| *(new)* | `set_focus` | `{ originalId: string\|null, visible: string[] }` | `null`. Re-prioritises queued work: the selected clip first, then visible cards, then background. Call on selection change and when the gallery scrolls. |
| GET /api/originals | `originals_list` | – | `{ rev, originals: Summary[], longMissing: [{id,name,since}] }` |
| POST /api/import (body) | `import_bytes` | raw body (bytes), header `x-cs-name: <file name, URI-encoded>` | `{ id }` |
| POST /api/import-path | `import_path` | `{ path }` | `{ id }` |
| POST /api/missing/delete | `missing_delete` | `{ ids }` | `{ deleted }` |
| GET /api/originals/:id | `original_get` | `{ originalId }` | `Detail` |
| PATCH /api/originals/:id | `original_patch` | `{ originalId, patch: { name?, draft? } }` (`draft: null` clears; wrapped because Tauri cannot tell a missing arg from null) | `Detail` |
| DELETE /api/originals/:id | `original_delete` | `{ originalId }` | `{ ok: true }` (file → Recycle Bin; cancels its jobs first) |
| POST …/reveal | `original_reveal` | `{ originalId }` | `{ ok: true }` |
| POST …/estimate | `estimate` | `{ originalId, settings }` | `Estimate` or `{ stale: true }` when a newer estimate for the same clip superseded it; 409 `busy` while a screen job is active (as 1.2, unless `allowMediaDuringUpload`). |
| POST …/loopsearch | `loop_search` | `{ originalId, minLen?, maxLen? }` (null = Auto) | `{ jobId }` (result in `job.result`, as 1.2) |
| POST …/looppoint | `loop_point` | `{ originalId, start, length }` | `{ best, current, alternatives, searched }` |
| POST …/versions | `version_create` | `{ originalId, settings }` (settings = 1.2 body incl. `force`) | `{ versionId, jobId }`; 400 `duplicate` + `versionId` |
| PATCH /api/versions/:vid | `version_patch` | `{ versionId, label }` | `Version` |
| DELETE /api/versions/:vid | `version_delete` | `{ versionId }` | `{ ok: true }`. **Change:** a queued/converting version is cancelled, then deleted (1.2 refused with 409). |
| POST …/send | `version_send` | `{ versionId }` | `{ jobId }` |
| POST …/remake | `version_remake` | `{ versionId }` | `{ versionId, jobId }` |
| POST …/retry | `version_retry` | `{ versionId }` | `{ versionId, jobId }` |
| GET /api/sources | `sources_list` | – | `{ sources, ticked, desktopApp, pluginsDir }` |
| PUT /api/browse | `browse_set` | `{ ticked }` | sources config `{ enabled, options, ticked }` |
| POST /api/link | `link_add` | `{ url }` | `{ jobId }` (403 if "Add from link" is off) |
| PUT /api/sources/:sid | `source_set` | `{ sourceId, enabled?, options? }` | sources config |
| GET /api/sources/:sid/items | `source_items` | `{ sourceId, page, refresh }` | `{ page, total, hasMore, items }` (403 if off, 502 on provider error) |
| GET /api/sources/:sid/thumb | *(media server)* | `…/sources/<sid>/thumb?item=` | image |
| POST /api/sources/:sid/items/:item/add | `source_add` | `{ sourceId, itemId, record?, seconds?, title? }` | `{ jobId }` |
| PUT /api/gallery | `gallery_set_dir` | `{ dir }` | `AppState` |
| POST /api/gallery/open | `gallery_open` | – | `{ dir }` |
| POST /api/gallery/sync | `gallery_sync` | – | `{ ok: true }` |
| GET /api/transport | `device_state` | – | `{ transport: 'winusb'\|'serial'\|'none', needsSetup, warning?, failedUsb, fake? }`; 409 `updating` while a device job runs |
| GET /api/device | `device_check` | – | screen info + `_transport`, `_needsSetup`, `_warning`, `_restarted`, `_capacity`; rejects 502 with `code`/`failedUsb`; 409 `updating` while busy. Restarts the screen once on "no reply". |
| POST /api/restart | `device_restart` | – | `{ jobId }` (kind `setup`) |
| POST /api/setup | `device_setup` | – | `{ jobId }` (UAC prompt) |
| POST /api/restore | `device_restore` | – | `{ jobId }` |
| POST /api/reconnect | `device_reconnect` | – | `{ jobId }` |
| /media/… | *(media server)* | | |

The Phase 0 commands `device_info` and `device_upload` are removed (replaced by `device_check` / `version_send`).
Window commands (`win_*`) belong to WS4 (`window/`).

### Shapes (unchanged from 1.2 unless noted)

- `AppState`: `{ appVersion, desktopApp, deviceFps: 18, imageSeconds: 3, fake, mask, cornersDefault, maxSeconds, budget,
  total, budgetMeasured, capacityAt, uploadRate, galleryDir, onScreen, missingNoticeDays: 30, notchKey,
  oldNotchCount, mediaBase, backend: 'rust' }` (**new**: `mediaBase` = media server root, `backend`).
- `Summary`, `Detail`, `Version` (decorated: `oldNotch`, `speed`, `tooBig`, `onScreen`, `onScreenIncomplete`),
  `Estimate` (`{ length, start, budget, duration, fps, mode, q, detail, bytes, fits, maxFitLength?, quality, settings }`),
  loop-search result (`{ frames, searched, candidates[{start,startFrame,length,frames,end,endFrame,cost,ratio,similarity,blend,preview}], suggestedMax, minLen, maxLen }`):
  exactly as `library.js` builds them.

## 3. Jobs and the scheduler

### 3.1 Job (what the UI sees)

```
Job {
  id: string,                 // 12 hex chars
  kind: 'version'|'loopsearch'|'send'|'setup'|'source'   // visible kinds (1.2)
      | 'estimate'|'assets'|'probe'|'gallery'|'splash'|'cards',  // internal kinds, hidden from activity
  label: string,
  originalId?, versionId?, sourceId?, itemId?,
  priority: 'user'|'selected'|'visible'|'background',
  state: 'queued'|'running'|'done'|'failed'|'cancelled',
  status: 'waiting'|'running'|'done'|'error',   // 1.2 alias: queued→waiting, failed/cancelled→error
  progress: 0..1, message: string,               // queued jobs get a "Waiting for …" message
  error?, code?, failedUsb?,                      // code 'cancelled' / 'superseded' when cancelled
  result?,                                        // loopsearch/source: full; others: {warning, serialWarning, name, label, restarted, restartFailed}
  bytesTotal?, bytesSent?,                        // send
  startedAt, runningSince?, finishedAt?           // ms since epoch
}
```

### 3.2 Events (Tauri `emit` to all windows)

| Event | Payload | When |
| --- | --- | --- |
| `cs:job` | `Job` (visible kinds only) | created, state change, progress (≤ 10 per second per job) |
| `cs:library` | `{ rev }` | originals/versions changed (debounced 100 ms) |
| `cs:app` | `{}` | settings, capacity, slot records or gallery folder changed → call `app_state` |
| `cs:device` | `{ busy: bool }` | a device job started/finished (replaces `deviceBusy` polling) |

The UI may keep polling `activity` (cheap, in-memory) while migrating; events let it stop polling.

### 3.3 Scheduler rules (core/scheduler.rs)

- **Resource classes** with limits: `cpu` (encodes / CPU decodes, default max(1, cores/2), setting), `gpu`
  (NVDEC/NVENC, 2), `device` (1, exclusive), `net` (4), `capture` (1, scene recording), `io` (8: probes,
  thumbnails, gallery scans). A task asks for units of one or more classes and starts only when all are free.
- **Priority**: `user` (save version, send, setup) > `selected` > `visible` > `background`; FIFO within a level.
  `set_focus` re-ranks queued tasks by their `originalId`. No head-of-line blocking: a waiting task whose class is
  full never stops lower-priority tasks for free classes; when units free up the highest-priority waiter gets them.
- **Preemption** (priority inversion): a `selected`/`user` task blocked on `cpu`/`gpu` held by `background` tasks
  marked `preemptible` cancels the youngest of them; it is re-queued and runs again later (assets, cards, background
  loop frames are preemptible; version conversions are not).
- **Cancellation**: by job id, by original, by version. A cancelled task's token is set and every child process it
  spawned through `TaskCtx::spawn` is killed (ffmpeg); its job ends `cancelled`. Leaving a clip cancels its
  preemptible/estimate work (`set_focus`), deleting a clip/version cancels all of its work.
- **Coalescing**: tasks with the same `coalesceKey` (e.g. `estimate:<oid>`): a new one replaces queued older ones
  (they resolve `superseded` → `estimate` returns `{ stale: true }`) and cancels a running older one.
- **Device vs media**: while a device task runs, new `cpu`/`gpu` work does not start (1.2 saw an encode stall an
  upload); running work finishes. Setting `allowMediaDuringUpload` (default off) lifts this after hardware testing.
  The screen is only ever touched from tasks holding `device`.

## 4. Media server (decision)

A tiny Rust HTTP/1.1 server in core on `127.0.0.1:<random port>`, every path prefixed with a random 128-bit token
(`http://127.0.0.1:PORT/TOKEN/…`), GET/HEAD only, `Range` (206/416) as 1.2's `serveFile`, `Accept-Ranges: bytes`,
`Access-Control-Allow-Origin: *` (token-protected; lets canvases read frames). Routes:
`/TOKEN/media/o/<oid>/<thumb.jpg|frame.jpg|proxy.mp4|proxy.jpg|loop-N.mp4|source>`,
`/TOKEN/media/v/<vid>/<preview.mp4|thumb.jpg|device.mp4>`, `/TOKEN/sources/<sid>/thumb?item=…`.
Why not the asset protocol: the perf report flags WebView2 seeking on custom protocols as a risk, it needs an
fs scope that follows the user's Gallery folder, and the Range server is what 1.2 already proved.
**WS4:** if a CSP is set, allow `img-src media-src connect-src http://127.0.0.1:*`.

## 5. Interfaces other workstreams implement

All in `core::api` (Rust). Errors are `CoreError { message, code, status }`; `TaskCtx` gives
`progress(f, msg)`, `is_cancelled()`, `check()?` (returns `cancelled`), `spawn(Command) -> ChildGuard`
(killed on cancel, `CREATE_NO_WINDOW`), `wait(ChildGuard)`.

### 5.1 WS2: `MediaEngine` (`media/`)

```rust
pub trait MediaEngine: Send + Sync + 'static {
    fn probe(&self, ctx: &TaskCtx, file: &Path) -> CoreResult<ProbeInfo>;           // as convert.probe
    fn source_assets(&self, ctx: &TaskCtx, file: &Path, dir: &Path, info: &ProbeInfo) -> CoreResult<SourceAssets>; // thumb.jpg, frame.jpg, proxy
    fn loop_search(&self, ctx: &TaskCtx, req: &LoopSearchReq) -> CoreResult<serde_json::Value>; // candidates incl. blend + loop-N.mp4 cards in req.dir
    fn loop_point(&self, ctx: &TaskCtx, req: &LoopPointReq) -> CoreResult<serde_json::Value>;
    fn bytes_per_second(&self, ctx: &TaskCtx, req: &EncodeReq, quality: u32) -> CoreResult<f64>; // "Auto" max loop length
    fn estimate(&self, ctx: &TaskCtx, req: &EncodeReq) -> CoreResult<serde_json::Value>;  // Estimate without quality/settings
    fn convert(&self, ctx: &TaskCtx, req: &EncodeReq, out_dir: &Path) -> CoreResult<serde_json::Value>; // device.mp4, preview.mp4, thumb.jpg
    fn splash(&self, ctx: &TaskCtx, out: &Path) -> CoreResult<()>;                      // SPLASH v1 MJPEG
    fn resources(&self, task: MediaTask, info: Option<&ProbeInfo>) -> Resources { /* default: 1 cpu */ }
}
```
`EncodeReq = { file, dir (original cache dir), start, length, quality: 'auto'|1..100, detail, frame, mask (notch),
corners, blend, budgetBytes, maxSeconds, srcFps, isImage, duration }`; convert errors use code `tooLong` with
`maxFitLength`. WS2 may keep per-clip caches (raw frames, loop features) in `dir`; core deletes `dir` with the clip.
Estimate→convert reuse is WS2-internal (key it by the request). Core never calls ffmpeg itself.
`core::mask::band_px` / `notch_key` are in core (needed for `notchKey`); WS2 uses the same function.

### 5.2 WS3: `SourceProvider` (`sources/`)

```rust
pub trait SourceProvider: Send + Sync + 'static {
    fn id(&self) -> &str;  fn name(&self) -> &str;  fn description(&self) -> &str;
    fn kind(&self) -> &str;                                // online | local | action | plugin
    fn options(&self) -> Vec<serde_json::Value>;           // [{ key, label, type }]
    fn detect(&self, ctx: &SourceCtx) -> Detect;           // { available, status }
    fn list(&self, ctx: &SourceCtx, page: u32, refresh: bool) -> CoreResult<ItemPage>; // items as 1.2 (thumb: "thumb:<id>" | https URL)
    fn thumb_path(&self, ctx: &SourceCtx, item: &str) -> Option<PathBuf>;
    fn add(&self, ctx: &SourceCtx, task: &TaskCtx, item: &str, opts: &AddOpts) -> CoreResult<AddedFile>; // { file, name, temp }
    fn download(&self, ctx: &SourceCtx, task: &TaskCtx, url: &str) -> CoreResult<AddedFile> { /* only 'link' */ }
    fn resources(&self, item: &str, opts: &AddOpts) -> Resources { /* default: 1 net; recording: 1 capture */ }
}
pub trait SourceCatalog: Send + Sync + 'static {
    fn providers(&self) -> Vec<Arc<dyn SourceProvider>>;  // built-ins + plug-ins (WS3 decides the plug-in format)
    fn plugins_dir(&self) -> PathBuf;
    fn desktop_capture(&self) -> bool;                    // recording available (AppState.desktopApp)
}
```
`SourceCtx = { options (this source's settings), tmpDir (%APPDATA%\CoolerScreen\downloads) }`. Core does the
settings (`sources.enabled/options/ticked` in settings.json, same rules), the `inGallery` mapping, the jobs, and the
import of `AddedFile` into the Gallery folder (deleting it when `temp`).

### 5.3 Core provides to everyone

`Library` (originals/versions on the 1.2 disk layout), `Settings`, `Scheduler`, `paths` (honours
`COOLERSCREEN_HOME`, `COOLERSCREEN_GALLERY`), `.ht/.zip` theme extraction (`core::theme`), Recycle Bin delete,
device service. Wiring: `cs_core::backends()` in `src/lib.rs` returns `Backends { media, sources }`; until WS2/WS3
land it returns stand-ins (`NoMedia` fails with code `unavailable`, `NoSources` lists nothing).

## 6. Modes and test hooks

- `COOLERSCREEN_BACKEND=node` (default until WS2–WS4 land): Phase 0 behaviour, Node sidecar serves the UI; the Rust
  core does not start its gallery watcher (one writer only). `COOLERSCREEN_BACKEND=rust`: Rust commands + media server,
  no sidecar.
- `COOLERSCREEN_FAKE_DEVICE=1|winusb|serial|none|failedusb|busy|noreply|noreply-stuck`, `COOLERSCREEN_FAKE_FAIL=1`:
  same simulation as 1.2; never touches USB, the COM port or PowerShell.
- `COOLERSCREEN_HOME`, `COOLERSCREEN_GALLERY`: scratch library/gallery for tests.
- `COOLERSCREEN_FAKE_MEDIA=1`: core's fake media engine (no ffmpeg; placeholder thumbnails, quick fake
  conversions) so WS4 can run the whole UI before WS2 lands. It always forces the fake screen: its files are not
  real videos and must never reach the cooler. `COOLERSCREEN_FAKE_SPEED=0.1` speeds up the fake screen.

## 7. Implementation notes (WS1, 2026-10-11)

- Code: `core/scheduler.rs` (scheduler + `TaskCtx`), `core/api.rs` (traits and stand-ins), `core/service.rs`
  (every endpoint as an `App` method), `core/commands.rs` (Tauri commands, `cs:*` events; `build.rs` and
  `capabilities/main.json` list the same command names), `core/media_server.rs`, `core/library.rs`, `settings.rs`,
  `gallery.rs` (1.2 disk layout), `core/device_service.rs`, `core/theme.rs`, `core/fake_media.rs`.
- Capacity: real info replies nest the fields (`{cmd:"info",data:{i_block_free,...}}`); 1.2 read the top level only,
  so its measured capacity never worked on hardware. 2.0 reads `data` (and the top level, for the fake screen).
- `cs2 core-check | core-state | core-restart | core-send <library>/<oid>/versions/<vid>` run the same core paths
  from the CLI (core-send requires `COOLERSCREEN_HOME` = a scratch folder; it copies the version there and never
  writes the real library or settings).

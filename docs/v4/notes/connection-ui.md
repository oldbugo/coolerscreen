# Connection UI reliability (2026-10-26)

Base: `52f1eec` (4.18.0). Worker branch: `codex/coolerscreen-connection-ui`.
Scope: UI only; backend connection state is supplied by the backend worker.

## Built

- The pill, failure banner, Screen settings and Live message derive from the
  current `app.live.status.connection` while Live owns the device. A successful
  reconnect clears the old failure and keeps that recovered status when Live is
  switched off. Real device jobs and manual checks take precedence.
- Failed USB enumeration offers Reconnect with explicit Windows approval. The
  wording does not assume the failure happened at startup. Live failures show
  their reason and retry delay instead of an indefinite generic Connecting.
- Visible UI refreshes `app_state` every two seconds. This reads app state and
  does not open USB. Refresh changes status text, not focused form inputs; a
  countdown edits the existing banner text instead of recreating its buttons.
  A settings/app response received during a status read supersedes that read.
- With Live off, a failed/missing screen watches `device_presence` every ten
  seconds while the UI is visible. This endpoint must enumerate only, without
  opening a USB interface. Each arrival/transport transition triggers one normal
  scheduled check. A still-present failure does not trigger repeated checks.
  Healthy streams, device jobs, checks and intentional holds are not probed.
- Null editor previews are never treated as proof that the USB screen is
  connected. A real temporary preview can provide current connection state.

## Verification

Verified by the UI worker:

- `node --no-warnings tools/connection-status.test.mjs`: pass. Covers stale
  failure to recovery, recovery followed by Live off, failures replacing stale
  info, jobs/manual checks/holds, setup, temporary preview and Null twin, invalid
  retry values, correct Windows approval wording and presence-transition latch.
- `node --check public/app.js` and `public/js/connection-status.js`: pass.
- `node --no-warnings tools/layers.test.mjs`: pass (existing regression checks).
- `git diff --check`: pass.

No hardware command, driver/settings change or app instance was run. Integrated
quiet WebView rendering and real-screen reconnect/sleep/reboot remain for the
parent to verify. The implementation depends on the backend worker's typed
connection state and no-open `device_presence` endpoint.

Helpers used: no additional helper slots available during this bounded UI
implementation; parent/backend workers provide integration review.

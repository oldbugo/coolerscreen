# Connection reliability backend (2026-10-26)

Based on private GitHub main `52f1eec` (4.18.0). Built in an isolated worktree;
no screen, running app, user settings, or installation touched by this worker.

## Behavior

- `live.status.connection` exposes `state`, nullable `message`, `retryInSeconds`
  and `attempts`. States: connecting, connected, notFound, failedUsb, busy,
  noReply, error, paused, off. Existing status/hold fields remain available.
- Sender retries use 1/2/4/8/15 second backoff. Repeated identical failures log
  only on change or a five-minute reminder. Success and pause clear old errors,
  counts, countdown and backoff. Resume starts a new connection attempt.
- Live ends an old stream, requires a genuine info JSON reply before Live
  entry, and checks holds/quit during the handshake. Unrelated acknowledgements
  do not establish health. The normal three-second reply window is retained;
  each handshake write has a short deadline. Frame writes now have a total
  one-second deadline across all chunks, rather than twenty seconds per chunk.
- Missing-device failures start a background Windows probe at most once per
  thirty seconds. No sender ownership is held while PowerShell probes. A failed
  descriptor is attributed to the screen only if its parent and location match
  the same remembered screen node; unrelated/crossed USB nodes do not qualify.
- Only a verified info noReply can request automatic firmware recovery. It
  queues one visible Activity job through the existing exclusive device
  scheduler, rechecks firmware, and restarts only if that check also has no
  reply. No automatic UAC, USB port cycling, uploads or driver changes occur.
  User/system/idle holds are checked before the probe and immediately before
  restart. Recovery participates in quit/install guards. A rejected submission
  preserves eligibility; an accepted request consumes the conservative limit
  of one recovery per pipeline lifetime, including through later pauses.
- An explicit `device_check` always pauses Live through the scheduler and
  queries firmware, rather than returning cached/synthetic Live health.
  Routine status and new `device_presence` snapshots never pause or claim USB.
  Presence is enumeration/driver hint only, not firmware health.

## Evidence and limits

Helper used: inherited-model review agent, read-only. Review found and prompted
fixes for per-chunk timeout accumulation and consumed-but-rejected recovery.
Final source commit `972855b` compiled in the dedicated
`cs-target-connection-backend` release target. Targeted connection tests: 7 passed.
The same release library executable then passed the full suite with four test
threads: **430 passed, 0 failed, 23 ignored**, 453 total, 30.10 seconds. Tests cover
ACK rejection and handshake cancellation; exact versus crossed/unrelated USB
port fixtures; fake presence without recovery; backoff, state clearing and log
suppression; manual-check pause versus routine snapshots; and fake scheduler
ownership/recovery. The ignored tests retain their existing hardware/live-service
requirements and were not run.

No physical verification by this worker. Connection-time info proves firmware
answers then; it does not prove every frame appears or detect all later silent
firmware failures while USB writes still succeed. Explicit Check again now gives
a real diagnosis in that case. USB descriptor failure still requires Windows/
hardware recovery; application retries cannot manufacture an enumerated device.
OS open/claim operations retain existing Windows behavior. Firmware support for
the added pre-Live info exchange must be verified on the installed release.

One test observation initially timed out because the recovery job was hidden
from the visible-job recorder. Making recovery visible both fixes that observation
and prevents an idle installer from quitting during recovery; the test is rerun
against the final source.

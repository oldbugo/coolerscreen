# Connection reliability, 2026-10-26

User report: Embedded keeps playing, but live data is frequently absent and the
app cannot find the cooler screen. The user expects normal use, including other
development work, without repeated connection troubleshooting. Investigation
and the recovery/fix/test/install plan were approved in the same chat.

## Baseline evidence

- Private GitHub main, clean local source and installed/running app matched
  4.18.0, `52f1eec`.
- Windows reported a present descriptor-failure USB node, problem code 43, with
  no normal screen identity. Its parent and port exactly matched a remembered
  screen node. No personal device identifiers are reproduced here.
- Retained logs show thousands of missing-screen retries. No successful Live
  connection is recorded after 2026-10-20 in those retained logs.
- The actual application recovery log from 2026-10-22 shows a successful Windows
  port-cycle request followed by the screen failing to enumerate. A successful
  reset request is not a successful recovery and may not remove screen power.
- Live retries USB opening with backoff capped at 15 seconds. Its old startup
  sequence accepted outbound writes as a successful connection; the independent
  UI screen check could stay stale. Neither behavior proves displayed frames.

These observations do not establish whether firmware, power, cable/header,
boot/resume, or application behavior initially caused the enumeration failure.
Other development work is not established as the cause.

## Implementation and verification gates

1. Make port recovery select one exact remembered parent/location pair. Refuse
   ambiguous or unknown targets; never reset every failed USB device. Try once,
   wait for normal enumeration and report the actual result.
2. Give Live typed connection state, error, retry timing and attempt count;
   suppress repetitive identical logs while preserving transitions.
3. Verify a genuine info response before entering Live. Any automatic firmware
   recovery must run through the exclusive device scheduler after Live releases
   ownership, with a bounded number of attempts. No automatic elevation.
4. Derive UI state from current Live status. Protect device jobs and intentional
   pauses; detect device return while disconnected without opening healthy links.
5. Run isolated automated tests, build the installer and install only when idle.
   Keep fake app instances debug-only, hidden, with isolated app-data and gallery.

Real disconnect/reconnect, sleep/wake, PC restart and extended use require
separate evidence. Do not reboot or suspend the user's PC during development.
Do not call this fault fixed merely because code tests pass.

## Verification record

- Eight pure PowerShell target-selection/effect-boundary checks passed.
- Real-device dry run resolved exactly the remembered cooler port and its hub
  interface, with no USB operation performed.
- After Windows approval, one targeted port reset succeeded at 15:17:52 Sydney.
  Windows restored the normal screen identity; the existing 4.18.0 app resumed
  Live at 15:18:01 without an app restart. This verifies this recovery attempt,
  not the initiating cause or future sleep/reboot behavior.
- The user then confirmed that live data was visibly updating on the physical
  cooler screen. This confirmation applies to the recovered 4.18.0 session.
- Baseline Rust suite: 419 passed, 23 ignored, no failures. Connection UI mapping
  and arrival/hold tests plus existing layers tests passed.
- Completed backend release suite: 430 passed, 23 ignored, no failures. This
  includes synthetic firmware replies/cancellation, exact USB attribution and
  exclusive recovery/manual-check scheduling. No real USB was used by tests.
- Combined debug app built successfully. Ten guarded WebView checks passed
  with fake hardware and isolated app-data: rendered failure/recovery, focused
  control preservation, arrival detection and no repeat checks. No page errors;
  failure/recovered screenshots were visually inspected. Test process stopped.
- Installer and new-release hardware acceptance: pending.

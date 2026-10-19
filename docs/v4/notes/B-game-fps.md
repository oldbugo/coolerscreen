# G1: reading the frame rate of the game in front (research, 2026-10-19)

Stream B. Research only; nothing built. User request (2026-10-19): an FPS counter "that works for everything" for games, because the NVIDIA built-in one
does not work in Monster Hunter Wilds (MHW). This is the game's frame rate, not CoolerScreen's own live fps. Web research by a Sonnet helper (URLs below,
claims it could not confirm are marked UNCONFIRMED); facts about this PC were checked by me.

## Facts about this PC (checked 2026-10-19, read-only)
- Standard (UAC-filtered) token: `Administrators` is "deny only"; **not** a member of "Performance Log Users".
- Starting an ETW session as this user fails: `logman create trace ... -p Microsoft-Windows-DXGI -ets` gives "Access is denied" (the session was never created).
  The providers exist (DXGI, DxgKrnl, D3D9, Dwm-Core are listed by `logman query providers`).
- HWiNFO64 is running, but its shared memory is not enabled (`Global\HWiNFO_SENS_SM2` not found). RTSS and MSI Afterburner are not installed.
  `C:\Program Files\NVIDIA Corporation\FrameViewSDK` exists (NVIDIA App component).

## The methods
| Method | Works for MHW? | Needs | Measures | Verdict |
|---|---|---|---|---|
| 1. Own ETW present counter (what Intel PresentMon does) | very likely; UNCONFIRMED for MHW itself | once: user added to "Performance Log Users" (needs admin once), then sign out/in. No install, nothing hooked. | per-process present rate, frame time, 1 % low | **recommended default** |
| 1b. PresentMon 2.x service + API | same | installs a Windows service (admin) | same | rejected: breaks "no background service" |
| 2. RTSS shared memory | UNCONFIRMED (one anecdote that Afterburner works for MHW) | RTSS running (installed by the user); it injects into the game | fps (0.1 steps), 1024-frame time ring | fallback if the user already runs it |
| 3. HWiNFO sensors "Framerate (Presented)/(Displayed)", Frame Time | probably (it uses PresentMon/ETW itself) | HWiNFO running with shared memory on (free edition: switches off after 12 h) | same | fallback; we already read HWiNFO shared memory for temperatures |
| 4. NVIDIA FrameView / FrameView SDK | UNCONFIRMED | already installed here | frame rate, frame time, power | no public API found; skip |
| 5. Xbox Game Bar FPS | UNCONFIRMED | adds you to Performance Log Users | display only | no API for another app; but it is a quick test of method 1 |
| 6. DWM timing, perf counters, NVAPI/NVML | no | | not per game / not fps | dead ends |

## Method 1 in detail (recommended)
- **How it works:** an ETW consumer in our own process. It never touches the game. Events: DXGI `Present_Start/Stop` and D3D9 `Present_Start/Stop` (DX9-12 apps,
  incl. MHW which is DX12), plus DxgKrnl flip/present events (`PresentHistory_Start`, `Flip_Info`, `MMIOFlip_Info`, `QueuePacket`, `Present_Info`) which are the only way to see
  Vulkan and OpenGL (PresentMon reports those as runtime "Other", less accurate). The process id is in the event header. Frame time = time between consecutive presents of
  the process (`MsBetweenPresents`); `MsBetweenDisplayChange` needs the full DxgKrnl state machine. Win32: `StartTraceW` (real-time mode), `EnableTraceEx2` per provider,
  `OpenTraceW`, `ProcessTrace` on a thread, TDH to decode (the `ferrisetw` crate wraps the session handling; the present event layouts must be coded or taken from PresentMon).
  PresentMon is MIT licensed (Intel): its logic may be reused with the notice kept.
- **Permission (documented by Microsoft):** only administrators, members of "Performance Log Users" and system services can control and consume real-time ETW sessions;
  the NT Kernel Logger is admin only (we must not use it; PresentMon's providers are normal manifest providers).
- **What the user does once (Windows Home has no `compmgmt.msc`; the command works everywhere):** open PowerShell **as administrator** and run
  `net localgroup "Performance Log Users" "%USERNAME%" /add`, then sign out and in. In plain words: this lets that account start and read Windows' diagnostic event
  sessions (including which programs present frames) and read performance counters. It does not give admin rights, file access or the right to change settings.
  (Microsoft publishes no further risk statement; UNCONFIRMED beyond the group's right list.) The app never opens the elevated prompt itself.
- **Anti-cheat (important):** PresentMon issue #573: after EA AntiCheat starts, `StartTraceW` fails with access denied while `EnableTraceEx2` still works; Intel's fix
  (v2.5.0 "Anticheat Mitigation") creates the session **at service start with no providers** and enables providers on demand. So: create our own named session when CoolerScreen
  starts (before any game), keep it empty until the feature is on, enable providers on demand, stop it on quit, remove an orphan of our own name at start. Nothing found about
  Denuvo itself blocking ETW; nothing found either way for MHW (UNCONFIRMED).
- **Accuracy caveats:** presents include generated frames (MHW supports frame generation: the count can double the rendered rate); first-second warm-up; borderless vs fullscreen differ in
  present mode but both emit events; `dwm.exe` can be mistaken for the game (ignore it and our own pid).
- **Foreground game:** `GetForegroundWindow` -> `GetWindowThreadProcessId` -> `QueryFullProcessImageNameW` for the name; prefer the foreground pid if it presents, else the process with the
  highest present rate; fullscreen test = window rect equals the monitor rect.

## Recommendation
1. **Default:** own ETW present counter (method 1). Off by default (opt-in switch in the widget, nothing runs without a Game FPS widget). Shows: game name, FPS, 1 % low, frame-time graph.
2. **Reason lines** (never a silent dash): "Needs a one-time Windows setting" (not in the group; shows the exact command and what it means), "Blocked while the game runs" (session could not start
   after an anti-cheat loaded: restart CoolerScreen before the game), "No game in front".
3. **Fallbacks, in order:** RTSS shared memory if RTSS runs (zero setup; the user accepts RTSS injecting), HWiNFO "Framerate" sensors if HWiNFO shares them (we already read that memory),
   else the reason line. Do not use the PresentMon service or the FrameView SDK.
4. **Quick check for the user before we build (optional, 1 minute):** the Xbox Game Bar (Win+G) performance widget FPS tab uses the same ETW route; if it shows a number in MHW,
   method 1 should work there too.

## Questions only a test on the user's PC can answer (G2 will test them)
1. After joining the group: does `StartTraceW` succeed with MHW running, started before the game and after it (the #573 failure)? `EnableTraceEx2`?
2. Does MHW (DX12, Denuvo) deliver DXGI present events to a non-admin group member in borderless and in fullscreen?
3. Any `EnableTraceEx2` error 1450 (seen in one report with a different game)?
4. With frame generation on, present rate vs the rendered rate?
5. Does the user's RTSS/HWiNFO route work in MHW, if they would rather run one of those?
6. Is the user already in Performance Log Users (`whoami /groups`)? (Not on this session's token.)

## Sources (from the helper)
- ETW permission: https://learn.microsoft.com/en-us/windows/win32/api/evntrace/nf-evntrace-starttracew and https://learn.microsoft.com/en-us/windows/win32/api/evntrace/nf-evntrace-opentracew
- PresentMon: https://github.com/GameTechDev/PresentMon (README, README-ConsoleApplication.md, README-Service.md, PresentData/PresentMonTraceConsumer.cpp, PresentData/ETW/Microsoft_Windows_DxgKrnl.h), release v2.5.0 (anti-cheat mitigation), issue #573
- ETW group rights: https://geoffchappell.com/notes/windows/etw/security.htm; crate: https://docs.rs/crate/ferrisetw/latest
- RTSS shared memory: https://forums.guru3d.com/threads/rtss-sdk-help-how-do-i-get-the-focus-games-framerate.401703/ and https://github.com/pozitronik/steelclock-go/pull/52 (the official header was not found)
- HWiNFO: https://videocardz.com/newz/hwinfo-7-63-integrates-presentmon-adds-framerate-frametime-and-gpu-busy-metrics, https://www.hwinfo.com/forum/threads/presentmon.9170/
- NVIDIA FrameView: https://www.nvidia.com/en-us/geforce/technologies/frameview/
- Game Bar adds the user to Performance Log Users: Microsoft Answers thread "FPS counter not showing in Xbox Game Bar"
- MHW anti-tamper: https://insider-gaming.com/monster-hunter-wilds-denuvo-answered/ ; one Steam anecdote that Afterburner works for MHW

## Helpers used / verified / not verified
Helpers: one Sonnet research helper (web). Verified by me: the PC facts above. Not verified: everything marked UNCONFIRMED; nothing was tried with a game running.

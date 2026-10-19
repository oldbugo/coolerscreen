# Stream B: more widgets (notes)

Branch `claude/recursing-matsumoto-31e4c5`. Written by the stream B worker session, 2026-10-18.

## What 3.0.3 already has (so the list below is only what is missing)

clock (digital / analogue / minimal), date (line / calendar / big), sensor (CPU load, one core, CPU clock, GPU load /
temp / clock / VRAM / power / fan, RAM, net down, net up, disk used, disk activity; as number / bar / ring / gauge /
graph / line), stats panel, music (cover + progress), weather (now / forecast), visualizer, slideshow (GIF), picture,
text, info (uptime, CPU/GPU name, countdown, top processes).

The gap: every sensor widget shows **one** reading. PC-monitor users expect panels that show several at once (all cores,
all drives, up + down together, a list of their own choosing), plus the time tools (world clocks, timers).

## Proposed widget list (ordered by what PC-monitor screen users expect)

| # | Widget | Why | Source | Opt-in source? | Milestone |
|---|---|---|---|---|---|
| 1 | **Meters** (own list of 2-8 rows, any metric, each a bar with big value) | The AIDA64 / NZXT "system monitor" panel; the fixed Stats panel cannot be customised | existing metrics hub | no | B1 |
| 2 | **CPU cores** (every logical core as bars or a heat grid + average) | Per-core view is in every monitor app; 16-32 threads need a compact form | NtQuerySystemInformation (as `cpu.core`) | no | B1 |
| 3 | **Network** (download + upload together: two big numbers + one two-colour graph) | Up and down next to each other is the standard network tile | GetIfTable2 (existing hub) | no | B1 |
| 4 | **Disks** (all fixed drives: used bar, free space) | "Disk usage" overview; today one drive per widget | GetLogicalDrives + GetDiskFreeSpaceEx | no | B1 |
| 5 | **World clocks** (2-4 cities, time + day offset) | Very common, cheap, no data source | Windows time-zone registry (no network) | no | B1 |
| 6 | **Timer** (stopwatch, countdown timer, Pomodoro) | Desk-screen staple; started/paused from the editor | system clock | no | B1 |
| 7 | CPU temperature (+ fans, other board sensors) | The most asked-for reading; no safe built-in source | HWiNFO shared memory / LibreHardwareMonitor web server the user already runs | **yes** (user runs HWiNFO or LHM; we never install or elevate) | B2 |
| 8 | Agenda / calendar events | Day overview next to the clock | a local `.ics` file or the user's published ICS link | **yes** (file or link chosen by the user) | B2 |
| 9 | Disk read / write speed, network totals today | "Disk activity" and "data used" | PDH counters; daily counter kept in the app folder | no | B2 |
| 10 | Volume (master level + mute) | Visible feedback of the PC's volume | WASAPI endpoint volume | no | B2 |
| 11 | Ping / latency to a host (and a connection light) | Gamers' favourite | ICMP to a host the user types | **yes** (the user names the host) | B2 |
| 12 | Text with live words (`{cpu}`, `{time}`) | Custom readouts | existing sources | no | B2 |
| - | Not now: battery (desktop PC), public IP (needs a web call), per-app volume (large API surface), notifications | | | | later |

Rules for all of them: nothing appears in existing themes (new types only; no existing widget changes behaviour),
defaults are sensible, every option is a setting that saves and applies at once, a missing source shows a grey dash (never
0), big numbers (>= 48 px on the 2400 canvas), no motion along the vertical axis (bars grow sideways, fades only).

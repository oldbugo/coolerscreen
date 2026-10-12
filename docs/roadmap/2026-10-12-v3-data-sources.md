# 3.0 data sources: sensors and now-playing (research, 2026-10-12)

Desk research by a helper (partly web-checked: PawnIO/LHM, HWiNFO shared-memory terms; crate versions from memory —
check before pinning). Not yet tested on the user's PC.

| Data | Approach | Admin | Crate |
|---|---|---|---|
| CPU load / per-core, RAM, disk, network | `sysinfo` (reuse one `System`, refresh only what's shown); PDH for disk activity and real CPU clock (`% Processor Performance` × base) | no | `sysinfo`, `windows` (Win32_System_Performance) |
| GPU temp, load, clocks, fan, power, VRAM | NVML (`nvml.dll` ships with the NVIDIA driver; keep one handle; each metric may be `NotSupported`) | no | `nvml-wrapper` |
| Now playing (title, artist, cover, progress) | Windows GSMTC on an MTA worker thread; events + extrapolate position while playing; re-fetch properties ~400 ms after a track change; "preferred app" setting | no | `windows` (Media_Control, Storage_Streams, Foundation) |
| CPU temperature (Ryzen 9800X3D) | **No reliable way without a kernel driver.** Default: hidden / "N/A". Opt-in: read HWiNFO shared memory (`Global\HWiNFO_SENS_SM2`, user runs HWiNFO with shared memory on; free version turns it off after 12 h) or LibreHardwareMonitor's web server (`localhost:8085/data.json`, user runs LHM as admin with PawnIO). We never install or elevate anything ourselves, and never ship HWiNFO's DLL. | the user's tool needs it | — |
| Audio-reactive visuals | WASAPI loopback (no mic permission), FFT into 16–32 bands | no | `wasapi` or `cpal`, `realfft` |

Avoid WinRing0-based tools (Defender flags the driver) and the ACPI thermal zone (static/bogus on Ryzen boards).
Players with GSMTC: Spotify, browsers (YouTube etc., each tab a session), Apple Music, VLC, Media Player;
foobar2000 only with a plugin.

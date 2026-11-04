# UC shared hardware engine

2026-11-05, engine4.19.0. UC-only inherited launch mode enables a strict, private hardware exchange before Live starts.
Standalone mode ignores that UC-specific mode and keeps its existing local readers. No standalone installer is changed.

Visible legacy stats and modern aggregate sensor/info widgets request CPU/RAM/full NVIDIA readings with expiring demand
and1/2/4Hz rate leases. Smoothing an empty request does not activate a source. Legacy sampler startup returns before
native/thread setup in shared mode; the modern hub routes migrated metrics to injected state, with a local-source backstop.
Per-core, CPU clock, disk, network and external/admin sources keep their existing ownership outside this slice.

Frames retain optional CPU/RAM/GPU fields: product name, temperature/load/clock, used/total memory, power and fan.
Missing and expired readings render as dashes with empty bars. Snapshot validation matches UC: original timestamps,
three-second expiry, strict channel generation and advancing sequence, finite/range/paired-memory bounds and no unknown fields.
Diagnostic reads never create demand, sample hardware or renew an observation. State exposes original expiry and availability
flags/counters, not user/device identifiers or IPC payloads.

Verified with fake/pure tests: seven shared-hardware cases (one ignored helper is invoked in its isolated parent test)
and four worker cases. The isolated actual legacy sampler/metrics hub/renderer test reports zero local polls, rich injected
values, visible demand and requested4Hz. The UC native integration separately verifies one collector, both workers,
expiry/failure/recovery, no duplicate local reads, demand removal and cooperative Quit. No real source or physical screen
was operated by these tests. Installed verification is recorded in UC's acceptance notes after its release gate.

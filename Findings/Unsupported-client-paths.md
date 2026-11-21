# Unsupported & Deferred Client Paths

This document records Forsaken World client features, server-only data, and deferred systems that **Forsaken World Explorer** does not implement, with reasons and linked issues.

| Path / Feature | Reason | Tracking |
| --- | --- | --- |
| Game networking, login, server packets, combat, gameplay | Viewer-only scope; not a game client or server | Project brief |
| World editing, saving modified client files | Client folder is strictly read-only | Project brief |
| Hardware ray-traced renderer ("Lighter" equivalent) | Explicitly deferred past Phases 0–7 | [Status.md](Status.md) |
| `surfaces.pck` skybox texture decoding | Phase 5 uses procedural `EC_SunMoon`/`ECBSDFILEBOXAREA15` sky gradient (`approximate`) | [#15](https://github.com/kasperfriend/Forsaken-World-Explorer/issues/15) |
| SpeedTree distant billboard impostor atlas & runtime-sampled 64-frame leaf rocking | Phase 6 uses 3 discrete geometry LODs and documented vertex-shader sine wind (`approximate`) | [#6](https://github.com/kasperfriend/Forsaken-World-Explorer/issues/6) |
| Per-cell `BSData/*.dat` grass density bitmasks | Phase 6 streams `ECBSDFILEGRASS7` area placements directly without inventing grass in empty areas; per-cell bitmask unpacking deferred until watery/grassy zones are surveyed locally | [grass-formats.md](grass-formats.md) |
| Water shoreline foam strips & underwater caustics | Water is `implemented-partial` at the end of Phase 5 | [water-status.md](water-status.md) |

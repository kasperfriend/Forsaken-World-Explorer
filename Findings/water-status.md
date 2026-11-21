# Water Subsystem Status (Phase 5)

Water is **not complete** at the end of Phase 5. Each sub-feature is tracked individually below using the project status values (`implemented`, `implemented-partial`, `verified-cpu`, `not-verified-without-gpu`, `not-started`).

| Water Sub-Feature | Status | Details & Known Limits |
| --- | --- | --- |
| Water plane visibility culling & screen-space scissor math | `verified-cpu` | Unit-tested in `src/atmosphere.rs` and `--self-test`; reflection/refraction passes run only when at least one water plane is visible |
| Animated water normals | `implemented-partial` | Dual-layer scrolling tangent-space normal perturbation in `src/shaders/water.wgsl` (validated with `naga`); uses documented default scroll speeds where client water animation timing is not yet parsed from `BSData/*.dat` |
| Planar reflection pass (scissored & quality-scaled) | `implemented-partial` | Renders a mirrored-camera reflection pass into a scaled texture (`Low`=25%, `Medium`=50%, `High`=100% of scaled 3D viewport) when enabled and water is visible; can be toggled off in Settings → Graphics; not visually verified on a hardware GPU |
| Refraction pass (scissored & quality-scaled) | `implemented-partial` | Captures refracted underwater scene texture scaled by resolution scale + water quality when enabled and water is visible; can be toggled off in Settings → Graphics; not visually verified on a hardware GPU |
| Shoreline soft edge blend | `implemented-partial` | Depth-based soft edge alpha fade where water height meets terrain depth (`src/shaders/water.wgsl`); exact client shoreline wave foam strips deferred |
| Underwater tint & fog | `implemented-partial` | Switches to `ECBSDFILEBOXAREA15` underwater fog color/start/end and subtle blue-green tint when `camera.position.y < active_water_height`; caustic projection deferred |
| Real-zone `BSData/*.dat` water surface decoding | `implemented-partial` | `ECBSDFILEWATER` reference parsing verified against `EC_WorldFile.h`; `maps/s0/` has `0` water areas, so watery zones (`s1`–`s40`) remain to be surveyed locally via `water-survey` |

# GPU Stack Decision & Verification Limits

## 1. Graphics Stack Comparison vs. Reference (`Perfect-World-Explorer`)

The reference project (`kasperfriend/Perfect-World-Explorer`) uses:
- `wgpu = "0.19"`
- `winit = "0.29"`
- `egui = "0.27"` (`egui-wgpu`, `egui-winit`)
- `glam = "0.27"`
- `bytemuck = "1"`

**Forsaken World Explorer** uses the exact same `wgpu` + `winit` + `egui` + `glam` + `bytemuck` architecture, upgraded to current audit-clean versions:
- `eframe = "0.31"` (wrapping `wgpu = "24"`, `winit = "0.30"`, `egui = "0.31"`)
- `glam = "0.29"`, `bytemuck = "1"`, `naga = "24"` (for offline WGSL shader validation)

### Rationale for Version Upgrade
1. **Dependency security audit (`rustsec/audit-check`):** `wgpu 0.19` pulls older transitive crates that trigger security/unmaintained advisories; `wgpu 24` + `eframe 0.31` passes `rustsec/audit-check` with zero vulnerabilities.
2. **Backend parity:** `wgpu 24` supports DX12 on Windows (including WARP fallback), Vulkan on Linux (including Mesa `lavapipe` software Vulkan for headless CI offscreen smoke tests), and native WGSL validation via `naga 24` with zero version skew between offline validation and runtime pipelines.

## 2. Resolution Scale Architecture (10%–200%)

- The 3D terrain scene renders into an offscreen color + depth target sized to `floor(viewport * scale_pct / 100)` (clamped to `10%..=200%` and `>= 1x1`).
- `src/shaders/blit.wgsl` upscales/downscales that 3D target onto the swapchain/viewport while `egui` UI, menus, and overlays render at native window resolution.

## 3. Verification Limits Without a Hardware GPU

- **Offline WGSL validation (`shader-validate`):** Parses and validates every `.wgsl` shader in `src/shaders/` with `naga`. Proves shader syntax, types, and binding layouts are valid.
- **Linux Mesa `lavapipe` offscreen render smoke test (`terrain-render-smoke`):** Creates a real `wgpu` device on Mesa's CPU Vulkan driver (`lavapipe`), streams synthetic v6 terrain blocks through `TerrainStore` (`cache/terrain-v1`), renders one frame at a fixed camera, runs the resolution-scale blit pass, reads back the RGBA8 pixels, and checks that the image is non-empty, not a flat color, and has nonzero terrain coverage.
- **Important limit:** `lavapipe` proves pipelines compile and execute cleanly on CPU. **It does not establish visual parity with the Forsaken World client.** Only the user running on a real GPU and client can mark visual rendering `verified-by-user`.

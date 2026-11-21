//! Offscreen GPU terrain render smoke test (designed for Mesa `lavapipe` on Linux CI).
//!
//! Creates a `wgpu` device, streams synthetic v6 terrain blocks through `TerrainStore`
//! (`cache/terrain-v1`), renders one frame at a fixed camera with resolution scaling,
//! reads back the final color buffer, and asserts that the frame is non-empty, not a
//! single flat color, and has nonzero terrain coverage.
//!
//! Note: Software rendering via `lavapipe` verifies pipeline compilation and execution
//! only — it is **not** visual parity with a real GPU or client.

use std::process::ExitCode;

use forsaken_world_explorer::terrain_gpu;

fn main() -> ExitCode {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        ..Default::default()
    });

    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .or_else(|| {
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: true,
        }))
    });

    let Some(adapter) = adapter else {
        eprintln!("error: no wgpu adapter found (install mesa-vulkan-drivers for lavapipe)");
        return ExitCode::FAILURE;
    };

    let info = adapter.get_info();
    println!(
        "Adapter: {} ({:?}, {:?})",
        info.name, info.device_type, info.backend
    );

    let (device, queue) = match pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("terrain-render-smoke"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
        },
        None,
    )) {
        Ok(pair) => pair,
        Err(err) => {
            eprintln!("error: request_device failed: {err}");
            return ExitCode::FAILURE;
        }
    };

    let width = 256u32;
    let height = 256u32;
    let scale_pct = 75u16;

    let metrics =
        match terrain_gpu::render_offscreen_smoke(&device, &queue, width, height, scale_pct) {
            Ok(m) => m,
            Err(err) => {
                eprintln!("error: offscreen terrain smoke render failed: {err}");
                return ExitCode::FAILURE;
            }
        };

    let coverage_pct = (metrics.non_clear_pixels as f64 / metrics.total_pixels as f64) * 100.0;
    println!(
        "Rendered {}x{} (3D pass {}x{} @ {}% scale), {} streamed blocks, {}/{} non-clear pixels ({:.1}%), {} distinct colors",
        metrics.width,
        metrics.height,
        metrics.scaled_width,
        metrics.scaled_height,
        scale_pct,
        metrics.streamed_blocks,
        metrics.non_clear_pixels,
        metrics.total_pixels,
        coverage_pct,
        metrics.distinct_colors
    );

    if metrics.total_pixels != (width * height) as usize {
        eprintln!("error: unexpected pixel count {}", metrics.total_pixels);
        return ExitCode::FAILURE;
    }
    if metrics.non_clear_pixels < metrics.total_pixels / 10 {
        eprintln!(
            "error: terrain coverage too low ({:.1}%, expected >= 10%)",
            coverage_pct
        );
        return ExitCode::FAILURE;
    }
    if metrics.distinct_colors <= 16 {
        eprintln!(
            "error: rendered image looks flat ({} distinct colors, expected > 16)",
            metrics.distinct_colors
        );
        return ExitCode::FAILURE;
    }

    println!(
        "[PASS] Offscreen terrain smoke test passed (verifies pipeline execution only; not visual parity)."
    );
    ExitCode::SUCCESS
}

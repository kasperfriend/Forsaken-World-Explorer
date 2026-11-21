//! Offscreen GPU world render smoke test (designed for Mesa `lavapipe` on Linux CI).
//!
//! Renders sky, streamed terrain (day+night lightmap blend), opaque static `.bmd` building,
//! translucent test quad, and an animated water plane with scissored reflection/refraction
//! passes. Checks that all expected pixel regions are non-empty and that the water region
//! differs from the non-water region.
//!
//! Note: Software rendering via `lavapipe` proves pipeline compilation and execution only;
//! it does **not** prove visual parity with the real game or a hardware GPU.

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
                eprintln!("error: offscreen world smoke render failed: {err}");
                return ExitCode::FAILURE;
            }
        };

    println!(
        "Rendered {}x{} (3D pass {}x{} @ {}% scale), {} blocks, sky={} px, non_clear={} px, bmd={} px, translucent={} px, water={} px (differs={}), distinct_colors={}",
        metrics.width,
        metrics.height,
        metrics.scaled_width,
        metrics.scaled_height,
        scale_pct,
        metrics.streamed_blocks,
        metrics.sky_pixels,
        metrics.non_clear_pixels,
        metrics.scenery_pixels,
        metrics.translucent_pixels,
        metrics.water_pixels,
        metrics.water_differs_from_non_water,
        metrics.distinct_colors
    );

    if metrics.total_pixels != (width * height) as usize {
        eprintln!("error: unexpected pixel count {}", metrics.total_pixels);
        return ExitCode::FAILURE;
    }
    if metrics.sky_pixels < 100 {
        eprintln!("error: sky region too small ({} px)", metrics.sky_pixels);
        return ExitCode::FAILURE;
    }
    if metrics.scenery_pixels < 50 {
        eprintln!(
            "error: static .bmd mesh pixels not detected ({} px)",
            metrics.scenery_pixels
        );
        return ExitCode::FAILURE;
    }
    if metrics.translucent_pixels < 30 {
        eprintln!(
            "error: translucent test quad pixels not detected ({} px)",
            metrics.translucent_pixels
        );
        return ExitCode::FAILURE;
    }
    if !metrics.water_differs_from_non_water {
        eprintln!("error: water region did not differ from non-water region");
        return ExitCode::FAILURE;
    }
    if metrics.distinct_colors <= 24 {
        eprintln!(
            "error: rendered image looks flat ({} distinct colors)",
            metrics.distinct_colors
        );
        return ExitCode::FAILURE;
    }

    println!(
        "[PASS] Offscreen sky + terrain + static/transparent .bmd + water smoke test passed (pipeline execution only; not visual parity)."
    );
    ExitCode::SUCCESS
}

// Forsaken World Explorer — Phase 5 Sky Dome / Gradient Shader
// Renders a fullscreen time-of-day zenith/horizon gradient with sun/moon disc.
// Documented approximation until full client surfaces.pck skybox textures are decoded.

struct CameraUniform {
    view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,
    fog_color: vec4<f32>,
    fog_params: vec4<f32>,
    sky_zenith: vec4<f32>,
    sky_horizon: vec4<f32>,
    sun_color: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> cam: CameraUniform;

struct SkyOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> SkyOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(3.0, 1.0),
    );
    let p = positions[vid];
    var out: SkyOut;
    out.pos = vec4<f32>(p, 0.9999, 1.0);
    out.uv = vec2<f32>(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    return out;
}

@fragment
fn fs_main(in: SkyOut) -> @location(0) vec4<f32> {
    // Vertical gradient: uv.y = 0 is top of screen (zenith), uv.y = 1 is bottom (horizon)
    let t = clamp(1.0 - in.uv.y, 0.0, 1.0);
    let curve = pow(t, 0.65);
    var col = mix(cam.sky_horizon.rgb, cam.sky_zenith.rgb, curve);

    // Subtle sun/moon glow near upper-center sky
    let sun_uv = vec2<f32>(0.5 + cam.sun_dir.x * 0.25, clamp(0.42 - cam.sun_dir.y * 0.35, 0.08, 0.75));
    let d = length(in.uv - sun_uv);
    let disc = smoothstep(0.055, 0.015, d);
    let halo = smoothstep(0.22, 0.02, d) * 0.25;
    col = mix(col, cam.sun_color.rgb, clamp(disc + halo, 0.0, 1.0));

    // Underwater tint when camera is below water surface
    if (cam.fog_params.w > 0.5) {
        col = mix(col, cam.fog_color.rgb, 0.65);
    }
    return vec4<f32>(col, 1.0);
}

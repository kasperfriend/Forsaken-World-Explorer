// Forsaken World Explorer — Phase 5 Experimental Water Shader
// Features dual-scrolling normal perturbation, planar reflection/refraction sampling,
// shoreline edge blend, underwater tint, and consistent environmental fog.

struct CameraUniform {
    view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,
    fog_color: vec4<f32>,
    fog_params: vec4<f32>,
    sky_zenith: vec4<f32>,
    sky_horizon: vec4<f32>,
    sun_color: vec4<f32>,
};

struct WaterUniform {
    water_color: vec4<f32>,
    // x = reflection_enabled (0/1), y = refraction_enabled (0/1), z = wave_scale, w = shore_fade
    flags: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> cam: CameraUniform;

@group(1) @binding(0)
var normal_tex: texture_2d<f32>;

@group(1) @binding(1)
var refl_tex: texture_2d<f32>;

@group(1) @binding(2)
var refr_tex: texture_2d<f32>;

@group(1) @binding(3)
var water_sampler: sampler;

@group(1) @binding(4)
var<uniform> water_u: WaterUniform;

struct WaterVertexIn {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    // edge_factor: 0.0 at shoreline boundary, 1.0 in deep interior
    @location(2) edge_factor: f32,
};

struct WaterVertexOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) edge_factor: f32,
    @location(2) screen_uv: vec2<f32>,
    @location(3) depth_dist: f32,
};

@vertex
fn vs_main(in: WaterVertexIn) -> WaterVertexOut {
    let clip = cam.view_proj * vec4<f32>(in.position, 1.0);
    let ndc = clip.xy / max(clip.w, 0.001);
    var out: WaterVertexOut;
    out.clip_pos = clip;
    out.uv = in.uv;
    out.edge_factor = in.edge_factor;
    out.screen_uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    out.depth_dist = clip.w;
    return out;
}

@fragment
fn fs_main(in: WaterVertexOut) -> @location(0) vec4<f32> {
    let t = cam.fog_color.a;
    // Dual-layer scrolling normal perturbation (documented default scroll speeds)
    let uv_a = in.uv * 6.0 + vec2<f32>(t * 0.030, t * 0.020);
    let uv_b = in.uv * 9.0 + vec2<f32>(-t * 0.022, t * 0.028);
    let n_a = textureSample(normal_tex, water_sampler, uv_a).xyz * 2.0 - vec3<f32>(1.0);
    let n_b = textureSample(normal_tex, water_sampler, uv_b).xyz * 2.0 - vec3<f32>(1.0);
    let n = normalize(vec3<f32>((n_a.x + n_b.x) * 0.25, 1.0, (n_a.y + n_b.y) * 0.25));

    let perturb = n.xz * 0.025 * water_u.flags.z;
    let suv_refl = clamp(vec2<f32>(in.screen_uv.x, 1.0 - in.screen_uv.y) + perturb, vec2<f32>(0.01), vec2<f32>(0.99));
    let suv_refr = clamp(in.screen_uv + perturb, vec2<f32>(0.01), vec2<f32>(0.99));

    let refl_sample = textureSample(refl_tex, water_sampler, suv_refl).rgb;
    let refr_sample = textureSample(refr_tex, water_sampler, suv_refr).rgb;

    let dn = clamp(cam.sun_dir.w, 0.0, 1.0);
    let base_water = water_u.water_color.rgb * mix(1.0, 0.35, dn);
    let refl_col = mix(cam.sky_horizon.rgb, refl_sample, water_u.flags.x);
    let refr_col = mix(base_water, mix(refr_sample, base_water, 0.45), water_u.flags.y);

    // Fresnel approximation + sun specular highlight
    let sun = normalize(cam.sun_dir.xyz);
    let spec = pow(max(dot(n, normalize(sun + vec3<f32>(0.0, 1.0, 0.0))), 0.0), 32.0) * mix(0.6, 0.15, dn);
    var col = mix(refr_col, refl_col, 0.42) + cam.sun_color.rgb * spec;

    // Shoreline soft edge blend
    let shore_alpha = clamp(in.edge_factor * water_u.flags.w, 0.18, water_u.water_color.a);

    if (cam.fog_params.z > 0.5) {
        let fog_span = max(cam.fog_params.y - cam.fog_params.x, 1.0);
        let fog_f = clamp((in.depth_dist - cam.fog_params.x) / fog_span, 0.0, 0.92);
        col = mix(col, cam.fog_color.rgb, fog_f);
    }
    return vec4<f32>(col, shore_alpha);
}

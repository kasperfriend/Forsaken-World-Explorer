// Forsaken World Explorer — Phase 5 Terrain Shader
// Blends day (t2lm) and night (t2lm1) lightmaps using time-of-day dn_factor and
// applies consistent environmental + underwater fog.

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

@group(1) @binding(0)
var day_lightmap_tex: texture_2d<f32>;

@group(1) @binding(1)
var night_lightmap_tex: texture_2d<f32>;

@group(1) @binding(2)
var lightmap_sampler: sampler;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) uv: vec2<f32>,
};

struct VertexOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) depth_dist: f32,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    let clip = cam.view_proj * vec4<f32>(in.position, 1.0);
    out.clip_pos = clip;
    out.world_normal = normalize(in.normal);
    out.color = in.color;
    out.uv = in.uv;
    out.depth_dist = clip.w;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let day_lm = textureSample(day_lightmap_tex, lightmap_sampler, in.uv).rgb;
    let night_lm = textureSample(night_lightmap_tex, lightmap_sampler, in.uv).rgb;
    let dn = clamp(cam.sun_dir.w, 0.0, 1.0);
    let lm = mix(day_lm, night_lm, dn);

    // Unscaled diffuse * lightmap product (lm already carries composited terrain diffuse * t2lm/t2lm1).
    // Do not multiply by an extra synthetic hemisphere shade on top of the baked client lightmap.
    let _n = normalize(in.world_normal);
    let base_rgb = in.color.rgb * lm;

    var final_rgb = base_rgb;
    if (cam.fog_params.z > 0.5) {
        let fog_span = max(cam.fog_params.y - cam.fog_params.x, 1.0);
        let fog_f = clamp((in.depth_dist - cam.fog_params.x) / fog_span, 0.0, 0.92);
        final_rgb = mix(final_rgb, cam.fog_color.rgb, fog_f);
    }
    if (cam.fog_params.w > 0.5) {
        final_rgb = mix(final_rgb, cam.fog_color.rgb, 0.35);
    }
    return vec4<f32>(final_rgb, 1.0);
}

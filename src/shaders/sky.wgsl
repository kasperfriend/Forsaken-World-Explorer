// Forsaken World Explorer — Client A3DSkySphere Shader (Cap + SurroundF + SurroundB)
// Source: EC_World.cpp:865-888, 1367-1374 and ECBSDFILEBOXAREA15 (+280 sky texture strings).
// Only invoked when real client sky textures resolve; never draws a procedural gradient fallback.

struct CameraUniform {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
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
var cap_tex: texture_2d<f32>;

@group(1) @binding(1)
var surr_f_tex: texture_2d<f32>;

@group(1) @binding(2)
var surr_b_tex: texture_2d<f32>;

@group(1) @binding(3)
var sky_sampler: sampler;

struct SkyOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
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
    out.ndc = p;
    return out;
}

@fragment
fn fs_main(in: SkyOut) -> @location(0) vec4<f32> {
    let near_p = cam.inv_view_proj * vec4<f32>(in.ndc, 0.0, 1.0);
    let far_p = cam.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir = normalize(far_p.xyz / far_p.w - near_p.xyz / near_p.w);

    let pi = 3.14159265;
    let elev = asin(clamp(dir.y, -1.0, 1.0));
    let v_surr = clamp(1.0 - elev / (pi * 0.42), 0.0, 1.0);

    var surr_col: vec3<f32>;
    if (dir.z >= 0.0) {
        let u_f = clamp(atan2(dir.z, dir.x) / pi, 0.0, 1.0);
        surr_col = textureSample(surr_f_tex, sky_sampler, vec2<f32>(u_f, v_surr)).rgb;
    } else {
        let u_b = clamp(atan2(-dir.z, -dir.x) / pi, 0.0, 1.0);
        surr_col = textureSample(surr_b_tex, sky_sampler, vec2<f32>(u_b, v_surr)).rgb;
    }

    let cap_uv = dir.xz / max(dir.y + 0.65, 0.35) * 0.45 + vec2<f32>(0.5, 0.5);
    let cap_col = textureSample(cap_tex, sky_sampler, cap_uv).rgb;

    let cap_blend = smoothstep(0.28, 0.46, dir.y);
    var col = mix(surr_col, cap_col, cap_blend);

    if (cam.fog_params.w > 0.5) {
        col = mix(col, cam.fog_color.rgb, 0.65);
    }
    return vec4<f32>(col, 1.0);
}

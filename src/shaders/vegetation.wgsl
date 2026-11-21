// Forsaken World Explorer — Phase 6 Instanced Vegetation Shader (SpeedTree + Grass)
// Supports alpha-tested tree leaves/grass blades and opaque trunks with documented
// vertex-shader sine wind approximation, day/night lighting, and environmental fog.

struct CameraUniform {
    view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,
    fog_color: vec4<f32>,
    fog_params: vec4<f32>,
    sky_zenith: vec4<f32>,
    sky_horizon: vec4<f32>,
    sun_color: vec4<f32>,
};

struct VegMaterialUniform {
    base_color: vec4<f32>,
    // x = alpha_cutoff (0.0 trunk, 0.5 alpha-tested leaves/grass), y = wind_amplitude
    params: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> cam: CameraUniform;

@group(1) @binding(0)
var veg_tex: texture_2d<f32>;

@group(1) @binding(1)
var veg_sampler: sampler;

@group(1) @binding(2)
var<uniform> veg_mat: VegMaterialUniform;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) wind_weight: f32,
};

struct InstanceIn {
    @location(4) model_c0: vec4<f32>,
    @location(5) model_c1: vec4<f32>,
    @location(6) model_c2: vec4<f32>,
    @location(7) model_c3: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) depth_dist: f32,
};

@vertex
fn vs_main(v: VertexIn, inst: InstanceIn) -> VertexOut {
    let model = mat4x4<f32>(inst.model_c0, inst.model_c1, inst.model_c2, inst.model_c3);
    var world_pos = (model * vec4<f32>(v.position, 1.0)).xyz;
    let world_nrm = (model * vec4<f32>(v.normal, 0.0)).xyz;

    // Documented sine wind approximation:
    // sway = sin(t * 2.2 + world_x * 0.17 + world_z * 0.13) * amp * wind_weight
    let t = cam.fog_color.a;
    let sway = sin(t * 2.2 + world_pos.x * 0.17 + world_pos.z * 0.13) * veg_mat.params.y * v.wind_weight;
    world_pos.x = world_pos.x + sway;
    world_pos.z = world_pos.z + sway * 0.5;

    let clip = cam.view_proj * vec4<f32>(world_pos, 1.0);
    var out: VertexOut;
    out.clip_pos = clip;
    out.world_normal = normalize(world_nrm);
    out.uv = v.uv;
    out.depth_dist = clip.w;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let tex_sample = textureSample(veg_tex, veg_sampler, in.uv);
    if (veg_mat.params.x > 0.0 && tex_sample.a < veg_mat.params.x) {
        discard;
    }

    let dn = clamp(cam.sun_dir.w, 0.0, 1.0);
    let n = normalize(in.world_normal);
    let sun = normalize(cam.sun_dir.xyz);
    let ndotl = max(dot(n, sun), 0.0);
    let shade = mix(0.42 + 0.58 * ndotl, 0.22 + 0.24 * ndotl, dn);

    var final_rgb = tex_sample.rgb * veg_mat.base_color.rgb * shade;
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

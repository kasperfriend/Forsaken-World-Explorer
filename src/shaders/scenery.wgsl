// Forsaken World Explorer — Phase 4 Instanced Static Scenery Shader (.bmd)
// Renders opaque and alpha-tested submeshes depth-tested against terrain.
// Uses approximate directional + hemisphere lighting until Phase 5 regional lighting.

struct CameraUniform {
    view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,
    fog_color: vec4<f32>,
};

struct MaterialUniform {
    diffuse_rgba: vec4<f32>,
    // x = alpha_cutoff (0.0 for opaque, 0.5 for alpha-tested)
    params: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> cam: CameraUniform;

@group(1) @binding(0)
var base_tex: texture_2d<f32>;

@group(1) @binding(1)
var base_sampler: sampler;

@group(1) @binding(2)
var<uniform> mat_u: MaterialUniform;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) uv: vec2<f32>,
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
    @location(1) color: vec4<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) depth_dist: f32,
};

@vertex
fn vs_main(v: VertexIn, inst: InstanceIn) -> VertexOut {
    let model = mat4x4<f32>(inst.model_c0, inst.model_c1, inst.model_c2, inst.model_c3);
    let world_pos = model * vec4<f32>(v.position, 1.0);
    let world_nrm = (model * vec4<f32>(v.normal, 0.0)).xyz;
    let clip = cam.view_proj * world_pos;

    var out: VertexOut;
    out.clip_pos = clip;
    out.world_normal = normalize(world_nrm);
    out.color = v.color;
    out.uv = v.uv;
    out.depth_dist = clip.w;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let tex_sample = textureSample(base_tex, base_sampler, in.uv);
    if (mat_u.params.x > 0.0 && tex_sample.a < mat_u.params.x) {
        discard;
    }

    let n = normalize(in.world_normal);
    let sun = normalize(cam.sun_dir.xyz);
    let ndotl = max(dot(n, sun), 0.0);
    let hemi = 0.35 + 0.65 * ndotl;

    let albedo = tex_sample.rgb * in.color.rgb * mat_u.diffuse_rgba.rgb;
    let lit = albedo * hemi;

    let fog_factor = clamp((in.depth_dist - 400.0) / 1400.0, 0.0, 0.85);
    let final_rgb = mix(lit, cam.fog_color.rgb, fog_factor);
    return vec4<f32>(final_rgb, 1.0);
}

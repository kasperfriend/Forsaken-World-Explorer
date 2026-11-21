// Forsaken World Explorer — Phase 5 Instanced Static Scenery Shader (.bmd)
// Supports opaque, alpha-tested, and alpha-blended transparent submeshes.
// Blends per-vertex day_diffuse and night_diffuse using time-of-day dn_factor,
// with directional+hemisphere fallback only for meshes without vertex light data.

struct CameraUniform {
    view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,
    fog_color: vec4<f32>,
    fog_params: vec4<f32>,
    sky_zenith: vec4<f32>,
    sky_horizon: vec4<f32>,
    sun_color: vec4<f32>,
};

struct MaterialUniform {
    diffuse_rgba: vec4<f32>,
    // x = alpha_cutoff (0.0 opaque/blended, 0.5 alpha-tested)
    // y = has_vertex_lighting (1.0 = client day/night vertex light, 0.0 = approximate fallback)
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
    @location(2) day_color: vec4<f32>,
    @location(3) night_color: vec4<f32>,
    @location(4) uv: vec2<f32>,
};

struct InstanceIn {
    @location(5) model_c0: vec4<f32>,
    @location(6) model_c1: vec4<f32>,
    @location(7) model_c2: vec4<f32>,
    @location(8) model_c3: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) day_color: vec4<f32>,
    @location(2) night_color: vec4<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) depth_dist: f32,
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
    out.day_color = v.day_color;
    out.night_color = v.night_color;
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

    let dn = clamp(cam.sun_dir.w, 0.0, 1.0);
    var lit_rgb: vec3<f32>;
    if (mat_u.params.y > 0.5) {
        // Real per-vertex day/night lighting from .bmd
        let v_light = mix(in.day_color.rgb, in.night_color.rgb, dn);
        lit_rgb = tex_sample.rgb * mat_u.diffuse_rgba.rgb * v_light;
    } else {
        // Approximate directional + hemisphere fallback for v2 meshes without vertex light arrays
        let n = normalize(in.world_normal);
        let sun = normalize(cam.sun_dir.xyz);
        let ndotl = max(dot(n, sun), 0.0);
        let hemi = mix(0.38 + 0.62 * ndotl, 0.20 + 0.25 * ndotl, dn);
        lit_rgb = tex_sample.rgb * mat_u.diffuse_rgba.rgb * hemi;
    }

    var final_rgb = lit_rgb;
    if (cam.fog_params.z > 0.5) {
        let fog_span = max(cam.fog_params.y - cam.fog_params.x, 1.0);
        let fog_f = clamp((in.depth_dist - cam.fog_params.x) / fog_span, 0.0, 0.92);
        final_rgb = mix(final_rgb, cam.fog_color.rgb, fog_f);
    }
    if (cam.fog_params.w > 0.5) {
        final_rgb = mix(final_rgb, cam.fog_color.rgb, 0.35);
    }

    let out_alpha = clamp(tex_sample.a * mat_u.diffuse_rgba.a, 0.0, 1.0);
    return vec4<f32>(final_rgb, out_alpha);
}

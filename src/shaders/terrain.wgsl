// Forsaken World Explorer — Multi-Layer GPU Terrain Splatting Shader
// Samples real 512x512 mipmapped layer DDS textures tiled by t2mk fScaleU/fScaleV per world meter,
// blends layers 1..7 over base layer 0 using real t2mk 8-bit masks, and multiplies by unscaled
// day/night t2lm/t2lm1 lightmaps.

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

struct TerrainLayerUniform {
    scales_0: vec4<f32>,
    scales_1: vec4<f32>,
    params: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> cam: CameraUniform;

@group(1) @binding(0)
var day_lightmap_tex: texture_2d<f32>;

@group(1) @binding(1)
var night_lightmap_tex: texture_2d<f32>;

@group(1) @binding(2)
var lightmap_sampler: sampler;

@group(1) @binding(3)
var layer_tex_array: texture_2d_array<f32>;

@group(1) @binding(4)
var layer_sampler: sampler;

@group(1) @binding(5)
var mask_tex_0: texture_2d<f32>;

@group(1) @binding(6)
var mask_tex_1: texture_2d<f32>;

@group(1) @binding(7)
var<uniform> layer_u: TerrainLayerUniform;

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
    @location(4) world_pos: vec3<f32>,
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
    out.world_pos = in.position;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let day_lm = textureSample(day_lightmap_tex, lightmap_sampler, in.uv).rgb;
    let night_lm = textureSample(night_lightmap_tex, lightmap_sampler, in.uv).rgb;
    let dn = clamp(cam.sun_dir.w, 0.0, 1.0);
    let lm = mix(day_lm, night_lm, dn);

    var diffuse = vec3<f32>(1.0, 1.0, 1.0);
    if (layer_u.params.y > 0.5) {
        let cnt = i32(layer_u.params.x);
        let xz = in.world_pos.xz;
        let m0 = textureSample(mask_tex_0, lightmap_sampler, in.uv);
        let m1 = textureSample(mask_tex_1, lightmap_sampler, in.uv);

        diffuse = textureSample(layer_tex_array, layer_sampler, xz * layer_u.scales_0.x, 0).rgb;
        if (cnt > 1) {
            let c1 = textureSample(layer_tex_array, layer_sampler, xz * layer_u.scales_0.y, 1).rgb;
            diffuse = mix(diffuse, c1, m0.r);
        }
        if (cnt > 2) {
            let c2 = textureSample(layer_tex_array, layer_sampler, xz * layer_u.scales_0.z, 2).rgb;
            diffuse = mix(diffuse, c2, m0.g);
        }
        if (cnt > 3) {
            let c3 = textureSample(layer_tex_array, layer_sampler, xz * layer_u.scales_0.w, 3).rgb;
            diffuse = mix(diffuse, c3, m0.b);
        }
        if (cnt > 4) {
            let c4 = textureSample(layer_tex_array, layer_sampler, xz * layer_u.scales_1.x, 4).rgb;
            diffuse = mix(diffuse, c4, m0.a);
        }
        if (cnt > 5) {
            let c5 = textureSample(layer_tex_array, layer_sampler, xz * layer_u.scales_1.y, 5).rgb;
            diffuse = mix(diffuse, c5, m1.r);
        }
        if (cnt > 6) {
            let c6 = textureSample(layer_tex_array, layer_sampler, xz * layer_u.scales_1.z, 6).rgb;
            diffuse = mix(diffuse, c6, m1.g);
        }
        if (cnt > 7) {
            let c7 = textureSample(layer_tex_array, layer_sampler, xz * layer_u.scales_1.w, 7).rgb;
            diffuse = mix(diffuse, c7, m1.b);
        }
    }

    let _n = normalize(in.world_normal);
    var final_rgb = in.color.rgb * diffuse * lm;

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

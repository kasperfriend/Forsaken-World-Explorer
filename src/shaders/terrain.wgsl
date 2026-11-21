struct CameraUniform {
    view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,
    fog_color: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

@group(1) @binding(0)
var lightmap_tex: texture_2d<f32>;

@group(1) @binding(1)
var lightmap_sampler: sampler;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) uv: vec2<f32>,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) uv: vec2<f32>,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip_position = camera.view_proj * vec4<f32>(in.position, 1.0);
    out.normal = normalize(in.normal);
    out.color = in.color;
    out.uv = in.uv;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let sun = normalize(camera.sun_dir.xyz);
    let ndotl = max(dot(n, sun), 0.0);
    let lm = textureSample(lightmap_tex, lightmap_sampler, in.uv).rgb;
    let lit = in.color.rgb * lm * (0.35 + 0.65 * ndotl);
    return vec4<f32>(lit, 1.0);
}

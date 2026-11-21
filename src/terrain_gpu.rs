//! GPU terrain renderer using `wgpu`, streaming blocks through `TerrainStore` (`cache/terrain-v1`)
//! and rendering 3D world geometry at the configured resolution scale (10%–200%).

use std::{
    env, fs,
    time::{SystemTime, UNIX_EPOCH},
};

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

use crate::{
    camera::{scaled_render_target, streaming_blocks, FreeCamera},
    dxt,
    terrain_block::{self, T2bkFile},
    terrain_store::{TerrainPage, TerrainStore, DEFAULT_HOT_RAM_BYTES},
};

pub const TERRAIN_WGSL: &str = include_str!("shaders/terrain.wgsl");
pub const BLIT_WGSL: &str = include_str!("shaders/blit.wgsl");

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct TerrainVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 4],
    pub uv: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct CameraUniform {
    pub view_proj: [[f32; 4]; 4],
    pub sun_dir: [f32; 4],
    pub fog_color: [f32; 4],
}

/// Build an indexed triangle mesh for one terrain block from a streamed `TerrainPage`.
pub fn build_block_mesh(
    page: &TerrainPage,
    day_colors: Option<&[[u8; 4]]>,
    row: u32,
    col: u32,
    block_size: f32,
    grid: u32,
) -> (Vec<TerrainVertex>, Vec<u32>) {
    let side = (grid as usize) + 1;
    let step = block_size / (grid as f32);
    let origin_x = (col as f32) * block_size;
    let origin_z = (row as f32) * block_size;

    let mut vertices = Vec::with_capacity(side * side);
    for gz in 0..side {
        for gx in 0..side {
            let idx = gz * side + gx;
            let h = page.heights.get(idx).copied().unwrap_or(0.0);
            let n = page.normals.get(idx).copied().unwrap_or([0, 127, 0]);
            let normal = [
                n[0] as f32 / 127.0,
                n[1] as f32 / 127.0,
                n[2] as f32 / 127.0,
            ];
            let color = if let Some(cols) = day_colors.and_then(|c| c.get(idx)) {
                [
                    cols[0] as f32 / 255.0,
                    cols[1] as f32 / 255.0,
                    cols[2] as f32 / 255.0,
                    cols[3] as f32 / 255.0,
                ]
            } else {
                // Elevation-tinted neutral ground color when vertex color section is absent
                let t = ((h - 80.0) / 160.0).clamp(0.0, 1.0);
                [0.32 + 0.35 * t, 0.48 + 0.25 * t, 0.26 + 0.20 * t, 1.0]
            };
            vertices.push(TerrainVertex {
                position: [
                    origin_x + (gx as f32) * step,
                    h,
                    origin_z + (gz as f32) * step,
                ],
                normal,
                color,
                uv: [(gx as f32) / (grid as f32), (gz as f32) / (grid as f32)],
            });
        }
    }

    let mut indices = Vec::with_capacity((grid as usize) * (grid as usize) * 6);
    for gz in 0..(grid as usize) {
        for gx in 0..(grid as usize) {
            let i0 = (gz * side + gx) as u32;
            let i1 = i0 + 1;
            let i2 = ((gz + 1) * side + gx) as u32;
            let i3 = i2 + 1;
            indices.extend_from_slice(&[i0, i2, i1, i1, i2, i3]);
        }
    }

    (vertices, indices)
}

#[derive(Debug, Clone, Copy)]
pub struct OffscreenSmokeMetrics {
    pub width: u32,
    pub height: u32,
    pub scaled_width: u32,
    pub scaled_height: u32,
    pub streamed_blocks: usize,
    pub total_pixels: usize,
    pub non_clear_pixels: usize,
    pub distinct_colors: usize,
}

/// Render one offscreen terrain frame at a fixed camera using a `wgpu` device (such as
/// Mesa `lavapipe` on Linux CI), streaming synthetic v6 terrain blocks through `TerrainStore`
/// (`cache/terrain-v1`) and executing both the terrain pass and resolution-scale blit pass.
///
/// Proves pipelines compile and execute on CPU; does **not** establish visual parity.
pub fn render_offscreen_smoke(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    width: u32,
    height: u32,
    scale_pct: u16,
) -> Result<OffscreenSmokeMetrics, String> {
    let (scaled_w, scaled_h) = scaled_render_target(width, height, scale_pct);

    // Stream 4 synthetic v6 terrain blocks (2x2 grid) through TerrainStore (`cache/terrain-v1`).
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let cache_root =
        env::temp_dir().join(format!("fw-terrain-smoke-{}-{nanos}", std::process::id()));
    let mut store = TerrainStore::open(&cache_root, DEFAULT_HOT_RAM_BYTES)?;

    let grid = 16u32;
    let block_size = 64.0f32;
    let t2bk_bytes = terrain_block::synthetic::build_t2bk_v6(grid, 4);
    let t2bk = T2bkFile::parse_header(&t2bk_bytes)?;

    let camera = FreeCamera {
        position: Vec3::new(10.0, 155.0, 10.0),
        yaw: 0.78,
        pitch: -0.52,
        ..Default::default()
    };
    let visible = streaming_blocks(camera.position, block_size, 2, 2, 2);

    let mut all_verts = Vec::new();
    let mut all_indices = Vec::new();
    for &(row, col) in &visible {
        let block_idx = (row * 2 + col) as usize;
        let key = TerrainStore::content_key(&format!("smoke-block-{block_idx}").into_bytes());
        let page = store.get_or_insert_with(&key, || {
            let decoded = t2bk.decode_block(&t2bk_bytes, block_idx, grid)?;
            Ok(TerrainPage {
                heights: decoded.heights,
                normals: decoded.normals,
            })
        })?;
        let base_vert = all_verts.len() as u32;
        let (verts, idxs) = build_block_mesh(&page, None, row, col, block_size, grid);
        all_verts.extend(verts);
        all_indices.extend(idxs.into_iter().map(|i| i + base_vert));
    }
    let _ = fs::remove_dir_all(&cache_root);

    // Decode a synthetic DXT1 lightmap via Phase 2 `dxt.rs` and upload as RGBA8 texture.
    let dxt1_block = [0xef, 0xf7, 0x20, 0x84, 0xe4, 0x1b, 0xe4, 0x1b];
    let dds = dxt::synthetic::build_dds(4, 4, b"DXT1", &dxt1_block);
    let surf = dxt::parse_dds(&dds)?;
    let lm_rgba = dxt::decode_rgba8(&surf);

    let lm_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("smoke-lightmap"),
        size: wgpu::Extent3d {
            width: surf.width,
            height: surf.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &lm_tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &lm_rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(surf.width * 4),
            rows_per_image: Some(surf.height),
        },
        wgpu::Extent3d {
            width: surf.width,
            height: surf.height,
            depth_or_array_layers: 1,
        },
    );
    let lm_view = lm_tex.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("smoke-sampler"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    // Uniforms
    let vp: Mat4 = camera.view_proj(width as f32 / height as f32);
    let uniform = CameraUniform {
        view_proj: vp.to_cols_array_2d(),
        sun_dir: [0.4, 0.85, 0.35, 0.0],
        fog_color: [0.15, 0.25, 0.40, 1.0],
    };
    let cam_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("camera-uniform"),
        contents: bytemuck::bytes_of(&uniform),
        usage: wgpu::BufferUsages::UNIFORM,
    });

    let cam_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("camera-bgl"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let cam_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("camera-bg"),
        layout: &cam_bgl,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: cam_buf.as_entire_binding(),
        }],
    });

    let tex_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("tex-bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let lm_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("lm-bg"),
        layout: &tex_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&lm_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });

    let terrain_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("terrain-shader"),
        source: wgpu::ShaderSource::Wgsl(TERRAIN_WGSL.into()),
    });
    let terrain_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("terrain-pipeline-layout"),
        bind_group_layouts: &[&cam_bgl, &tex_bgl],
        push_constant_ranges: &[],
    });
    let terrain_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("terrain-pipeline"),
        layout: Some(&terrain_layout),
        vertex: wgpu::VertexState {
            module: &terrain_shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<TerrainVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: 0,
                        shader_location: 0,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: 12,
                        shader_location: 1,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x4,
                        offset: 24,
                        shader_location: 2,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 40,
                        shader_location: 3,
                    },
                ],
            }],
        },
        fragment: Some(wgpu::FragmentState {
            module: &terrain_shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        multiview: None,
        cache: None,
    });

    let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("blit-shader"),
        source: wgpu::ShaderSource::Wgsl(BLIT_WGSL.into()),
    });
    let blit_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("blit-pipeline-layout"),
        bind_group_layouts: &[&tex_bgl],
        push_constant_ranges: &[],
    });
    let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("blit-pipeline"),
        layout: Some(&blit_layout),
        vertex: wgpu::VertexState {
            module: &blit_shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &blit_shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview: None,
        cache: None,
    });

    let scene_color = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scaled-scene-color"),
        size: wgpu::Extent3d {
            width: scaled_w,
            height: scaled_h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let scene_depth = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scaled-scene-depth"),
        size: wgpu::Extent3d {
            width: scaled_w,
            height: scaled_h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let final_color = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("final-output-color"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });

    let scene_view = scene_color.create_view(&wgpu::TextureViewDescriptor::default());
    let depth_view = scene_depth.create_view(&wgpu::TextureViewDescriptor::default());
    let final_view = final_color.create_view(&wgpu::TextureViewDescriptor::default());
    let blit_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("blit-bg"),
        layout: &tex_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&scene_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });

    let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("terrain-vbuf"),
        contents: bytemuck::cast_slice(&all_verts),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("terrain-ibuf"),
        contents: bytemuck::cast_slice(&all_indices),
        usage: wgpu::BufferUsages::INDEX,
    });

    let clear_r = 0.08;
    let clear_g = 0.14;
    let clear_b = 0.24;

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("terrain-smoke-encoder"),
    });
    {
        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("terrain-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &scene_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: clear_r,
                        g: clear_g,
                        b: clear_b,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        rpass.set_pipeline(&terrain_pipeline);
        rpass.set_bind_group(0, &cam_bg, &[]);
        rpass.set_bind_group(1, &lm_bg, &[]);
        rpass.set_vertex_buffer(0, vbuf.slice(..));
        rpass.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint32);
        rpass.draw_indexed(0..all_indices.len() as u32, 0, 0..1);
    }
    {
        let mut bpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("blit-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &final_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });
        bpass.set_pipeline(&blit_pipeline);
        bpass.set_bind_group(0, &blit_bg, &[]);
        bpass.draw(0..3, 0..1);
    }

    let bytes_per_pixel = 4u32;
    let unpadded_bpr = width * bytes_per_pixel;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded_bpr = unpadded_bpr.div_ceil(align) * align;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("smoke-readback"),
        size: (padded_bpr as u64) * (height as u64),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &final_color,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bpr),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    let slice = readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |res| {
        let _ = tx.send(res);
    });
    device.poll(wgpu::Maintain::Wait);
    rx.recv()
        .map_err(|e| format!("readback channel error: {e}"))?
        .map_err(|e| format!("readback map error: {e:?}"))?;

    let data = slice.get_mapped_range();
    let clear_px = [
        (clear_r * 255.0).round() as i32,
        (clear_g * 255.0).round() as i32,
        (clear_b * 255.0).round() as i32,
    ];
    let mut non_clear_pixels = 0usize;
    let mut distinct = std::collections::HashSet::new();
    for y in 0..height as usize {
        let row = &data[y * padded_bpr as usize..y * padded_bpr as usize + unpadded_bpr as usize];
        for px in row.as_chunks::<4>().0 {
            let dr = (px[0] as i32 - clear_px[0]).abs();
            let dg = (px[1] as i32 - clear_px[1]).abs();
            let db = (px[2] as i32 - clear_px[2]).abs();
            if dr > 4 || dg > 4 || db > 4 {
                non_clear_pixels += 1;
            }
            distinct.insert([px[0], px[1], px[2]]);
        }
    }
    drop(data);
    readback.unmap();

    Ok(OffscreenSmokeMetrics {
        width,
        height,
        scaled_width: scaled_w,
        scaled_height: scaled_h,
        streamed_blocks: visible.len(),
        total_pixels: (width as usize) * (height as usize),
        non_clear_pixels,
        distinct_colors: distinct.len(),
    })
}

/// CPU-side streamed terrain mesh ready for upload to the live viewport renderer.
#[derive(Debug, Clone, Default)]
pub struct TerrainMeshSnapshot {
    pub vertices: Vec<TerrainVertex>,
    pub indices: Vec<u32>,
    pub revision: u64,
    pub streamed_blocks: usize,
    pub hot_ram_bytes: usize,
}

/// CPU-side decoded RGBA8 lightmap texture ready for upload to the live viewport renderer.
#[derive(Debug, Clone)]
pub struct TerrainTextureSnapshot {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub revision: u64,
}

impl Default for TerrainTextureSnapshot {
    fn default() -> Self {
        Self {
            width: 1,
            height: 1,
            rgba: vec![255, 255, 255, 255],
            revision: 0,
        }
    }
}

/// Camera-centered streaming session backed by `TerrainStore` (`cache/terrain-v1`).
/// Supports both real Forsaken World `maps/<zone>/` files and a synthetic preview zone.
pub struct TerrainSession {
    pub zone_name: String,
    pub block_size: f32,
    pub block_grid: u32,
    pub block_rows: u32,
    pub block_cols: u32,
    t2bk_bytes: Vec<u8>,
    t2bk: T2bkFile,
    store: TerrainStore,
    last_center: Option<(i32, i32, i32)>,
    pub mesh: TerrainMeshSnapshot,
    pub lightmap: TerrainTextureSnapshot,
}

impl TerrainSession {
    /// Open a real client zone from `<client_root>/maps/<zone>/` using `cache_root`
    /// (`<root>/cache/terrain-v1`) and the configured hot RAM budget in bytes.
    pub fn open_zone(
        client_root: &std::path::Path,
        zone: &str,
        cache_root: &std::path::Path,
        hot_ram_bytes: usize,
    ) -> Result<Self, String> {
        let zone_dir = client_root.join("maps").join(zone);
        let trn2_path = zone_dir.join(format!("{zone}.trn2"));
        let trn2_bytes = fs::read(&trn2_path)
            .map_err(|e| format!("failed to read {}: {e}", trn2_path.display()))?;
        let trn2 = terrain_block::Trn2Header::parse(&trn2_bytes)?;

        let t2bk_path = zone_dir.join(format!("{zone}_1.t2bk"));
        let t2bk_bytes = fs::read(&t2bk_path)
            .map_err(|e| format!("failed to read {}: {e}", t2bk_path.display()))?;
        let t2bk = T2bkFile::parse_header(&t2bk_bytes)?;

        let store = TerrainStore::open(cache_root, hot_ram_bytes)?;

        let mut lightmap = TerrainTextureSnapshot::default();
        let t2lm_path = zone_dir.join(format!("{zone}_1.t2lm"));
        if let Ok(lm_bytes) = fs::read(&t2lm_path) {
            if let Ok(lm) = terrain_block::T2lmHeader::parse(&lm_bytes) {
                if let Some(&first_off) = lm.offsets.first() {
                    let start = first_off as usize;
                    let end = start + lm.data_size as usize;
                    if let Some(dds_slice) = lm_bytes.get(start..end) {
                        if let Ok(surf) = dxt::parse_dds(dds_slice) {
                            lightmap = TerrainTextureSnapshot {
                                width: surf.width,
                                height: surf.height,
                                rgba: dxt::decode_rgba8(&surf),
                                revision: 1,
                            };
                        }
                    }
                }
            }
        }

        Ok(Self {
            zone_name: zone.to_string(),
            block_size: trn2.block_size,
            block_grid: trn2.block_grid,
            block_rows: trn2.block_rows,
            block_cols: trn2.block_cols,
            t2bk_bytes,
            t2bk,
            store,
            last_center: None,
            mesh: TerrainMeshSnapshot::default(),
            lightmap,
        })
    }

    /// Open a synthetic 4x4 terrain preview zone backed by `TerrainStore` (`cache/terrain-v1`).
    pub fn open_synthetic(
        cache_root: &std::path::Path,
        hot_ram_bytes: usize,
    ) -> Result<Self, String> {
        let grid = 16u32;
        let block_size = 64.0f32;
        let rows = 4u32;
        let cols = 4u32;
        let t2bk_bytes = terrain_block::synthetic::build_t2bk_v6(grid, (rows * cols) as usize);
        let t2bk = T2bkFile::parse_header(&t2bk_bytes)?;
        let store = TerrainStore::open(cache_root, hot_ram_bytes)?;

        let dxt1_block = [0xef, 0xf7, 0x20, 0x84, 0xe4, 0x1b, 0xe4, 0x1b];
        let dds = dxt::synthetic::build_dds(4, 4, b"DXT1", &dxt1_block);
        let surf = dxt::parse_dds(&dds)?;
        let lightmap = TerrainTextureSnapshot {
            width: surf.width,
            height: surf.height,
            rgba: dxt::decode_rgba8(&surf),
            revision: 1,
        };

        Ok(Self {
            zone_name: "synthetic-preview".to_string(),
            block_size,
            block_grid: grid,
            block_rows: rows,
            block_cols: cols,
            t2bk_bytes,
            t2bk,
            store,
            last_center: None,
            mesh: TerrainMeshSnapshot::default(),
            lightmap,
        })
    }

    /// Stream blocks centered on `camera_pos` through `TerrainStore` whenever the camera
    /// moves into a new terrain block cell or the streaming radius changes.
    pub fn update_for_camera(
        &mut self,
        camera_pos: Vec3,
        radius_blocks: i32,
    ) -> Result<bool, String> {
        let cam_col = (camera_pos.x / self.block_size).floor() as i32;
        let cam_row = (camera_pos.z / self.block_size).floor() as i32;
        let key = (cam_row, cam_col, radius_blocks);
        if self.last_center == Some(key) {
            return Ok(false);
        }
        self.last_center = Some(key);

        let visible = streaming_blocks(
            camera_pos,
            self.block_size,
            self.block_rows,
            self.block_cols,
            radius_blocks,
        );

        let mut all_verts = Vec::new();
        let mut all_indices = Vec::new();
        let mut loaded = 0usize;
        for &(row, col) in &visible {
            let block_idx = (row * self.block_cols + col) as usize;
            if block_idx >= self.t2bk.offsets.len() {
                continue;
            }
            let cache_id = format!("{}-b{block_idx}-g{}", self.zone_name, self.block_grid);
            let content_key = TerrainStore::content_key(cache_id.as_bytes());
            let page = self.store.get_or_insert_with(&content_key, || {
                let decoded =
                    self.t2bk
                        .decode_block(&self.t2bk_bytes, block_idx, self.block_grid)?;
                Ok(TerrainPage {
                    heights: decoded.heights,
                    normals: decoded.normals,
                })
            })?;
            let base_vert = all_verts.len() as u32;
            let (verts, idxs) =
                build_block_mesh(&page, None, row, col, self.block_size, self.block_grid);
            all_verts.extend(verts);
            all_indices.extend(idxs.into_iter().map(|i| i + base_vert));
            loaded += 1;
        }

        self.mesh = TerrainMeshSnapshot {
            vertices: all_verts,
            indices: all_indices,
            revision: self.mesh.revision.wrapping_add(1),
            streamed_blocks: loaded,
            hot_ram_bytes: self.store.stats().owned_ram_bytes,
        };
        Ok(true)
    }
}

/// Live `wgpu` viewport renderer supporting 10%–200% resolution scaling with native UI overlay.
pub struct TerrainViewportRenderer {
    terrain_pipeline: wgpu::RenderPipeline,
    blit_pipeline: wgpu::RenderPipeline,
    tex_bgl: wgpu::BindGroupLayout,
    cam_buf: wgpu::Buffer,
    cam_bg: wgpu::BindGroup,
    sampler: wgpu::Sampler,
    lm_bg: wgpu::BindGroup,
    lm_revision: u64,
    scene_color: wgpu::Texture,
    scene_view: wgpu::TextureView,
    scene_depth: wgpu::Texture,
    depth_view: wgpu::TextureView,
    blit_bg: wgpu::BindGroup,
    scaled_size: (u32, u32),
    vbuf: Option<wgpu::Buffer>,
    ibuf: Option<wgpu::Buffer>,
    index_count: u32,
    mesh_revision: u64,
}

impl TerrainViewportRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let initial_uniform = CameraUniform {
            view_proj: Mat4::IDENTITY.to_cols_array_2d(),
            sun_dir: [0.4, 0.85, 0.35, 0.0],
            fog_color: [0.15, 0.25, 0.40, 1.0],
        };
        let cam_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("live-camera-uniform"),
            contents: bytemuck::bytes_of(&initial_uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let cam_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("live-camera-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let cam_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("live-camera-bg"),
            layout: &cam_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: cam_buf.as_entire_binding(),
            }],
        });

        let tex_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("live-tex-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("live-linear-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let terrain_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("live-terrain-shader"),
            source: wgpu::ShaderSource::Wgsl(TERRAIN_WGSL.into()),
        });
        let terrain_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("live-terrain-layout"),
            bind_group_layouts: &[&cam_bgl, &tex_bgl],
            push_constant_ranges: &[],
        });
        let terrain_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("live-terrain-pipeline"),
            layout: Some(&terrain_layout),
            vertex: wgpu::VertexState {
                module: &terrain_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<TerrainVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 12,
                            shader_location: 1,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 24,
                            shader_location: 2,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 40,
                            shader_location: 3,
                        },
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &terrain_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("live-blit-shader"),
            source: wgpu::ShaderSource::Wgsl(BLIT_WGSL.into()),
        });
        let blit_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("live-blit-layout"),
            bind_group_layouts: &[&tex_bgl],
            push_constant_ranges: &[],
        });
        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("live-blit-pipeline"),
            layout: Some(&blit_layout),
            vertex: wgpu::VertexState {
                module: &blit_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &blit_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        let default_lm = TerrainTextureSnapshot::default();
        let lm_bg = Self::upload_texture_bg(device, queue, &tex_bgl, &sampler, &default_lm);
        let (scene_color, scene_view, scene_depth, depth_view, blit_bg) =
            Self::create_scaled_targets(device, &tex_bgl, &sampler, 64, 64);

        Self {
            terrain_pipeline,
            blit_pipeline,
            tex_bgl,
            cam_buf,
            cam_bg,
            sampler,
            lm_bg,
            lm_revision: 0,
            scene_color,
            scene_view,
            scene_depth,
            depth_view,
            blit_bg,
            scaled_size: (64, 64),
            vbuf: None,
            ibuf: None,
            index_count: 0,
            mesh_revision: 0,
        }
    }

    fn upload_texture_bg(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        tex_bgl: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        tex: &TerrainTextureSnapshot,
    ) -> wgpu::BindGroup {
        let w = tex.width.max(1);
        let h = tex.height.max(1);
        let gpu_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("live-lightmap"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &gpu_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &tex.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        let view = gpu_tex.create_view(&wgpu::TextureViewDescriptor::default());
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("live-lm-bg"),
            layout: tex_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }

    fn create_scaled_targets(
        device: &wgpu::Device,
        tex_bgl: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        w: u32,
        h: u32,
    ) -> (
        wgpu::Texture,
        wgpu::TextureView,
        wgpu::Texture,
        wgpu::TextureView,
        wgpu::BindGroup,
    ) {
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("live-scaled-color"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("live-scaled-depth"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
        let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
        let blit_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("live-blit-bg"),
            layout: tex_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        (color, color_view, depth, depth_view, blit_bg)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn prepare_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        viewport_w: u32,
        viewport_h: u32,
        scale_pct: u16,
        uniform: &CameraUniform,
        mesh: &TerrainMeshSnapshot,
        lightmap: &TerrainTextureSnapshot,
    ) -> wgpu::CommandBuffer {
        let desired = scaled_render_target(viewport_w, viewport_h, scale_pct);
        if self.scaled_size != desired {
            let (c, cv, d, dv, bg) = Self::create_scaled_targets(
                device,
                &self.tex_bgl,
                &self.sampler,
                desired.0,
                desired.1,
            );
            self.scene_color = c;
            self.scene_view = cv;
            self.scene_depth = d;
            self.depth_view = dv;
            self.blit_bg = bg;
            self.scaled_size = desired;
        }

        queue.write_buffer(&self.cam_buf, 0, bytemuck::bytes_of(uniform));

        if self.lm_revision != lightmap.revision {
            self.lm_bg =
                Self::upload_texture_bg(device, queue, &self.tex_bgl, &self.sampler, lightmap);
            self.lm_revision = lightmap.revision;
        }

        if self.mesh_revision != mesh.revision {
            if mesh.vertices.is_empty() || mesh.indices.is_empty() {
                self.vbuf = None;
                self.ibuf = None;
                self.index_count = 0;
            } else {
                self.vbuf = Some(
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("live-terrain-vbuf"),
                        contents: bytemuck::cast_slice(&mesh.vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
                );
                self.ibuf = Some(
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("live-terrain-ibuf"),
                        contents: bytemuck::cast_slice(&mesh.indices),
                        usage: wgpu::BufferUsages::INDEX,
                    }),
                );
                self.index_count = mesh.indices.len() as u32;
            }
            self.mesh_revision = mesh.revision;
        }

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("live-terrain-encoder"),
        });
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("live-terrain-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.scene_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.08,
                            g: 0.14,
                            b: 0.24,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            if let (Some(vbuf), Some(ibuf)) = (&self.vbuf, &self.ibuf) {
                if self.index_count > 0 {
                    rpass.set_pipeline(&self.terrain_pipeline);
                    rpass.set_bind_group(0, &self.cam_bg, &[]);
                    rpass.set_bind_group(1, &self.lm_bg, &[]);
                    rpass.set_vertex_buffer(0, vbuf.slice(..));
                    rpass.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint32);
                    rpass.draw_indexed(0..self.index_count, 0, 0..1);
                }
            }
        }
        encoder.finish()
    }

    pub fn paint_blit(&self, render_pass: &mut wgpu::RenderPass<'_>) {
        render_pass.set_pipeline(&self.blit_pipeline);
        render_pass.set_bind_group(0, &self.blit_bg, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

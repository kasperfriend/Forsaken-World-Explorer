//! End-to-end CI test using a tiny real-format client fixture generated in a temp directory.
//!
//! IMPORTANT: Fixtures are NOT game data. This test proves the viewer's REAL loading path
//! (`maps/<zone>/`, `resources/textures.pck`, `resources/litmodels.pck`, `TerrainSession::open_zone`,
//! `zone_dump::dump_zone`, and Angelica world coordinates) executes end-to-end without using
//! the synthetic preview mode.

use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use flate2::{write::ZlibEncoder, Compression};
use forsaken_world_explorer::{
    bmd, dxt, pck, terrain_block, terrain_gpu::TerrainSession, world_objects, zone_dump,
};
use std::io::Write;

fn make_temp_client_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let root =
        std::env::temp_dir().join(format!("fw-real-loader-e2e-{}-{nanos}", std::process::id()));
    fs::create_dir_all(root.join("update")).unwrap();
    fs::create_dir_all(root.join("resources")).unwrap();
    fs::create_dir_all(root.join("maps/s0")).unwrap();
    root
}

fn build_real_format_trn2(tex_paths: &[&str]) -> Vec<u8> {
    let mut out = vec![0u8; 148];
    out[0..4].copy_from_slice(&terrain_block::TRN2_MAGIC.to_le_bytes());
    out[4..8].copy_from_slice(&1u32.to_le_bytes()); // version
    out[8..12].copy_from_slice(&256.0f32.to_le_bytes()); // width = 4 * 64m
    out[12..16].copy_from_slice(&256.0f32.to_le_bytes()); // length = 4 * 64m
    out[16..20].copy_from_slice(&64.0f32.to_le_bytes()); // block_size
    out[20..24].copy_from_slice(&16u32.to_le_bytes()); // block_grid
    out[24..28].copy_from_slice(&16u32.to_le_bytes()); // num_blocks (4x4)
    out[28..32].copy_from_slice(&4u32.to_le_bytes()); // block_rows
    out[32..36].copy_from_slice(&4u32.to_le_bytes()); // block_cols
    out[36..40].copy_from_slice(&(tex_paths.len() as u32).to_le_bytes()); // num_textures
    out[40..44].copy_from_slice(&1u32.to_le_bytes()); // mask_areas (1x1)
    out[48..52].copy_from_slice(&64u32.to_le_bytes()); // mask_size
    out[52..56].copy_from_slice(&1u32.to_le_bytes()); // sub_terrain_count
    out[72..76].copy_from_slice(&148u32.to_le_bytes()); // tex_table_off
    for tp in tex_paths {
        let bytes = tp.as_bytes();
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(bytes);
    }
    out
}

fn build_real_format_t2lm(dds: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&terrain_block::T2LM_MAGIC.to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes()); // version
    out.extend_from_slice(&1u32.to_le_bytes()); // num_areas
    out.extend_from_slice(&4u32.to_le_bytes()); // lm_size
    out.extend_from_slice(&1u32.to_le_bytes()); // flags
    out.extend_from_slice(&(dds.len() as u32).to_le_bytes()); // data_size
    out.extend_from_slice(&28u32.to_le_bytes()); // offset[0]
    out.extend_from_slice(dds);
    out
}

fn build_real_format_t2mk() -> Vec<u8> {
    let mask_raw = vec![180u8; 16 * 16];
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&mask_raw).unwrap();
    let mask_zlib = enc.finish().unwrap();

    let mut out = Vec::new();
    out.extend_from_slice(&terrain_block::T2MK_MAGIC.to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes()); // version
    out.extend_from_slice(&1u32.to_le_bytes()); // num_areas
    out.extend_from_slice(&2u32.to_le_bytes()); // num_layers
    out.extend_from_slice(&1u32.to_le_bytes()); // compressed
    let area0_off = 24u32;
    out.extend_from_slice(&area0_off.to_le_bytes());
    // Area 0: 2 layers (layer 0 base, layer 1 masked)
    out.extend_from_slice(&2u32.to_le_bytes());
    // layer 0: base (len=0)
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    // layer 1: compressed mask right after header
    let data_off = area0_off + 4 + 2 * 12;
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&data_off.to_le_bytes());
    out.extend_from_slice(&(mask_zlib.len() as u32).to_le_bytes());
    out.extend_from_slice(&mask_zlib);
    out
}

fn build_real_format_ecwld_and_ecbsd(items: &[(&str, [f32; 3])]) -> (Vec<u8>, Vec<u8>) {
    let mut bsd = vec![0u8; 72];
    bsd[0..4].copy_from_slice(&world_objects::ECBSD_MAGIC0.to_le_bytes());
    bsd[4..8].copy_from_slice(&world_objects::ECBSD_MAGIC1.to_le_bytes());
    bsd[8..12].copy_from_slice(&15u32.to_le_bytes());
    bsd[12..16].copy_from_slice(&1u32.to_le_bytes());
    let mut bsd_offsets = Vec::new();
    for (path, pos) in items {
        let off = bsd.len() as u32;
        bsd_offsets.push(off);
        for v in pos {
            bsd.extend_from_slice(&v.to_le_bytes());
        }
        for v in [0.0f32, 0.0, 1.0, 0.0, 1.0, 0.0] {
            bsd.extend_from_slice(&v.to_le_bytes());
        }
        bsd.extend_from_slice(&[0u8; 4]);
        let pb = path.as_bytes();
        bsd.extend_from_slice(&(pb.len() as u32).to_le_bytes());
        bsd.extend_from_slice(pb);
    }
    let mut wld = vec![0u8; 116];
    wld[0..4].copy_from_slice(&world_objects::ECWLD_MAGIC.to_le_bytes());
    wld[4..8].copy_from_slice(&14u32.to_le_bytes());
    wld[8..12].copy_from_slice(&256.0f32.to_le_bytes());
    wld[12..16].copy_from_slice(&256.0f32.to_le_bytes());
    wld[16..20].copy_from_slice(&64.0f32.to_le_bytes());
    wld[20..24].copy_from_slice(&16u32.to_le_bytes());
    wld[24..28].copy_from_slice(&1u32.to_le_bytes());
    wld[28..32].copy_from_slice(&1u32.to_le_bytes());
    wld[32..36].copy_from_slice(&1u32.to_le_bytes());
    wld.extend_from_slice(&120u32.to_le_bytes());
    let mut blk = vec![0u8; 44];
    blk[16..20].copy_from_slice(&(items.len() as i32).to_le_bytes());
    wld.extend_from_slice(&blk);
    for (i, &boff) in bsd_offsets.iter().enumerate() {
        wld.extend_from_slice(&((i as u32) + 1).to_le_bytes());
        wld.extend_from_slice(&boff.to_le_bytes());
    }
    (wld, bsd)
}

#[test]
fn real_zone_loader_reads_terrain_textures_and_bmd_buildings_end_to_end() {
    let root = make_temp_client_dir();
    let cache_root = root.join("cache");
    fs::create_dir_all(&cache_root).unwrap();

    // 1. Minimal update/pem.exe marker
    fs::write(root.join("update/pem.exe"), b"MZ\x90\x00").unwrap();

    // 2. Real-format DXT1 DDS textures packed into resources/textures.pck WITHOUT "textures/" prefix
    let dxt1_block = [0xef, 0xf7, 0x20, 0x84, 0xe4, 0x1b, 0xe4, 0x1b];
    let dds = dxt::synthetic::build_dds(4, 4, b"DXT1", &dxt1_block);
    let tex_pck_bytes = pck::synthetic::build_pck_bytes(&[
        ("maps\\arena\\ground01.dds", &dds),
        ("maps\\arena\\rock01.dds", &dds),
        ("building\\wall.dds", &dds),
    ]);
    fs::write(root.join("resources/textures.pck"), &tex_pck_bytes).unwrap();

    // 3. Real-format .bmd building packed into resources/litmodels.pck WITHOUT "litmodels/" prefix
    let bmd_bytes = bmd::synthetic::build_bmd_v5_box("Textures\\building\\wall.dds", 8.0, 20.0);
    let lit_pck_bytes = pck::synthetic::build_pck_bytes(&[("s0\\1\\litmodel_37.bmd", &bmd_bytes)]);
    fs::write(root.join("resources/litmodels.pck"), &lit_pck_bytes).unwrap();

    // 4. Real-format maps/s0/ files
    let trn2 = build_real_format_trn2(&[
        "Textures\\maps\\arena\\ground01.dds",
        "Textures\\maps\\arena\\rock01.dds",
    ]);
    let t2bk = terrain_block::synthetic::build_t2bk_v6(16, 16);
    let t2lm = build_real_format_t2lm(&dds);
    let t2mk = build_real_format_t2mk();
    let (wld, bsd) = build_real_format_ecwld_and_ecbsd(&[
        ("1\\litmodel_37.bmd", [12.0, 105.0, -18.0]),
        ("1\\litmodel_37.bmd", [-24.0, 108.0, 30.0]),
    ]);
    fs::write(root.join("maps/s0/s0.trn2"), &trn2).unwrap();
    fs::write(root.join("maps/s0/s0_1.t2bk"), &t2bk).unwrap();
    fs::write(root.join("maps/s0/s0_1.t2lm"), &t2lm).unwrap();
    fs::write(root.join("maps/s0/s0_1.t2lm1"), &t2lm).unwrap();
    fs::write(root.join("maps/s0/s0_1.t2mk"), &t2mk).unwrap();
    fs::write(root.join("maps/s0/s0.ecwld"), &wld).unwrap();
    fs::write(root.join("maps/s0/s0.ecbsd"), &bsd).unwrap();

    // 5. Run real --dump-zone path
    let summary = zone_dump::dump_zone(&root, "s0");
    assert!(summary.trn2_ok, "report:\n{}", summary.report_text);
    assert_eq!(summary.decoded_blocks, 16);
    assert!(summary.height_at_origin.is_some());
    assert_eq!(summary.trn2_textures_total, 2);
    assert_eq!(summary.trn2_textures_decoded, 2);
    assert_eq!(summary.mask_areas, 1);
    assert_eq!(summary.mask_layers, 2);
    assert_eq!(summary.placements_total, 2);
    assert_eq!(summary.bmd_resolved, 1);
    assert_eq!(summary.bmd_failed, 0);
    assert!(summary.bmd_sample_submeshes >= 1);

    // 6. Run real TerrainSession::open_zone (NOT open_synthetic!)
    let mut session = TerrainSession::open_zone(&root, "s0", &cache_root, 2 * 1024 * 1024).unwrap();
    assert!(!session.is_synthetic);
    let h = session.sample_height(0.0, 0.0).unwrap();
    assert!(h.is_finite() && h > 90.0);
    session
        .update_for_camera(glam::Vec3::new(0.0, h + 30.0, 0.0), 2)
        .unwrap();
    assert!(session.mesh.streamed_blocks > 0);
    assert!(!session.mesh.vertices.is_empty());
    assert!(session.day_lightmap.width >= 128);

    let _ = fs::remove_dir_all(&root);
}

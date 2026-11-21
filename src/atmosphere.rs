//! Phase 5 atmosphere, time-of-day (`EC_SunMoon`), environmental fog, `ECBSDFILEBOXAREA15`
//! and `ECBSDFILEWATER` parsers, water visibility culling + scissor rect math, and
//! back-to-front transparent submesh sorting.

use std::collections::HashSet;

use glam::{Mat4, Vec3, Vec4};

use crate::world_objects::{EcbsdHeader, EcwldHeader};

// Verified client transition constants from `EC_SunMoon.h`
pub const NIGHT_DAY_START_H: f32 = 3.0;
pub const NIGHT_DAY_END_H: f32 = 7.0;
pub const MID_NOON_H: f32 = 12.0;
pub const DAY_NIGHT_START_H: f32 = 18.0;
pub const DAY_NIGHT_END_H: f32 = 21.0;

const ECWLD_HEADER_BYTES: usize = 116;
const ECWLD_BLOCK9_BYTES: usize = 44;
const ECWLD_TREE5_BYTES: usize = 16;
const ECWLD_DATAREF_BYTES: usize = 8;
const ECBSD_BOXAREA_MIN_BYTES: usize = 200;

fn u16le(b: &[u8], off: usize) -> Result<u16, String> {
    let s = b
        .get(off..off + 2)
        .ok_or_else(|| format!("truncated u16 at {off}"))?;
    Ok(u16::from_le_bytes(s.try_into().unwrap()))
}

fn u32le(b: &[u8], off: usize) -> Result<u32, String> {
    let s = b
        .get(off..off + 4)
        .ok_or_else(|| format!("truncated u32 at {off}"))?;
    Ok(u32::from_le_bytes(s.try_into().unwrap()))
}

fn i32le(b: &[u8], off: usize) -> Result<i32, String> {
    let s = b
        .get(off..off + 4)
        .ok_or_else(|| format!("truncated i32 at {off}"))?;
    Ok(i32::from_le_bytes(s.try_into().unwrap()))
}

fn f32le(b: &[u8], off: usize) -> Result<f32, String> {
    let s = b
        .get(off..off + 4)
        .ok_or_else(|| format!("truncated f32 at {off}"))?;
    let v = f32::from_le_bytes(s.try_into().unwrap());
    if !v.is_finite() {
        return Err(format!("non-finite f32 at {off}"));
    }
    Ok(v)
}

pub fn unpack_argb_rgb(argb: u32) -> [f32; 3] {
    [
        ((argb >> 16) & 0xff) as f32 / 255.0,
        ((argb >> 8) & 0xff) as f32 / 255.0,
        (argb & 0xff) as f32 / 255.0,
    ]
}

/// Normalize an hour value into `[0.0, 24.0)` with clean midnight wraparound.
pub fn wrap_hours(hours: f32) -> f32 {
    if !hours.is_finite() {
        return MID_NOON_H;
    }
    hours.rem_euclid(24.0)
}

/// Advance time-of-day by `delta_hours`, wrapping cleanly across midnight (`24.0 -> 0.0`).
pub fn advance_time_of_day(hours: f32, delta_hours: f32) -> f32 {
    wrap_hours(wrap_hours(hours) + delta_hours)
}

/// Compute the client's day/night blend factor `dn_factor` in `[0.0, 1.0]` from `hours`:
/// - `0.0` = full daylight (`07:00..=18:00`)
/// - `1.0` = full night (`21:00..=03:00` across midnight)
/// - linear ramp during dawn (`03:00..07:00`) and dusk (`18:00..21:00`), matching `EC_SunMoon.h`.
pub fn day_night_factor(hours: f32) -> f32 {
    let h = wrap_hours(hours);
    if (NIGHT_DAY_END_H..=DAY_NIGHT_START_H).contains(&h) {
        0.0
    } else if h >= DAY_NIGHT_END_H || h <= NIGHT_DAY_START_H {
        1.0
    } else if h < NIGHT_DAY_END_H {
        // 03:00 -> 1.0, 07:00 -> 0.0
        1.0 - (h - NIGHT_DAY_START_H) / (NIGHT_DAY_END_H - NIGHT_DAY_START_H)
    } else {
        // 18:00 -> 0.0, 21:00 -> 1.0
        (h - DAY_NIGHT_START_H) / (DAY_NIGHT_END_H - DAY_NIGHT_START_H)
    }
}

/// Sun direction vector driven by time-of-day (max pitch 70° at noon per `EC_SunMoon.h`).
pub fn sun_direction_for_time(hours: f32) -> Vec3 {
    let h = wrap_hours(hours);
    let angle = ((h - 6.0) / 12.0) * std::f32::consts::PI;
    let max_pitch = 70.0f32.to_radians();
    let elev = angle.sin() * max_pitch;
    let x = angle.cos() * elev.cos();
    let y = elev.sin().max(-0.4);
    let z = 0.35 * elev.cos();
    Vec3::new(x, y, z).normalize_or_zero()
}

/// Interpolate zenith, horizon, and sun disc RGB colors for the procedural sky dome
/// using `dn_factor` and optional box-area fog colors.
pub fn sky_colors_for_time(
    hours: f32,
    box_area: Option<&BoxAreaAtmosphere>,
) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let dn = day_night_factor(hours);
    let day_zenith = [0.16, 0.42, 0.82];
    let night_zenith = [0.02, 0.04, 0.12];
    let day_horizon = box_area
        .map(|b| unpack_argb_rgb(b.day_fog_color))
        .unwrap_or([0.55, 0.72, 0.90]);
    let night_horizon = box_area
        .map(|b| unpack_argb_rgb(b.night_fog_color))
        .unwrap_or([0.06, 0.09, 0.18]);
    let lerp3 = |a: [f32; 3], b: [f32; 3], t: f32| {
        [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ]
    };
    let zenith = lerp3(day_zenith, night_zenith, dn);
    let horizon = lerp3(day_horizon, night_horizon, dn);
    let sun_col = lerp3([1.0, 0.94, 0.80], [0.65, 0.75, 0.95], dn);
    (zenith, horizon, sun_col)
}

/// Evaluate the linear environmental fog blend factor in `[0.0, 0.92]` at `distance_m`.
pub fn evaluate_fog_factor(
    distance_m: f32,
    fog_start: f32,
    fog_end: f32,
    distance_multiplier: f32,
    fog_enabled: bool,
) -> f32 {
    if !fog_enabled || !distance_m.is_finite() {
        return 0.0;
    }
    let mult = distance_multiplier.clamp(0.2, 5.0);
    let start = (fog_start * mult).max(0.0);
    let end = (fog_end * mult).max(start + 1.0);
    ((distance_m - start) / (end - start)).clamp(0.0, 0.92)
}

/// Parsed `ECBSDFILEBOXAREA14`/`15` atmosphere record from `<zone>.ecbsd`.
#[derive(Debug, Clone, PartialEq)]
pub struct BoxAreaAtmosphere {
    pub export_id: u32,
    pub is_default: bool,
    pub position: [f32; 3],
    pub extents: [f32; 3],
    pub day_fog_start: f32,
    pub day_fog_end: f32,
    pub day_fog_density: f32,
    pub day_fog_color: u32,
    pub night_fog_start: f32,
    pub night_fog_end: f32,
    pub night_fog_density: f32,
    pub night_fog_color: u32,
    pub uw_fog_start: f32,
    pub uw_fog_end: f32,
    pub uw_fog_density: f32,
    pub uw_fog_color: u32,
    pub light_dir: [f32; 3],
    pub day_ambient: u32,
    pub day_dir_light: u32,
    pub night_ambient: u32,
    pub night_dir_light: u32,
    /// Verified from `EC_WorldFile.h` (`ECBSDFILEBOXAREA15` +272, +276) and real `s0.ecbsd`.
    pub cap_tile_speed: f32,
    pub cap_rot_speed: f32,
    /// 3 `A3DSkySphere` day textures (`Cap`, `SurroundF`, `SurroundB`) at +280 in `s0.ecbsd`.
    pub day_sky_textures: [String; 3],
    /// 3 `A3DSkySphere` night textures (`Cap`, `SurroundF`, `SurroundB`) following day textures.
    pub night_sky_textures: [String; 3],
}

impl Default for BoxAreaAtmosphere {
    fn default() -> Self {
        Self {
            export_id: 0,
            is_default: true,
            position: [0.0, 0.0, 0.0],
            extents: [1024.0, 1024.0, 1024.0],
            day_fog_start: 120.0,
            day_fog_end: 720.0,
            day_fog_density: 0.5,
            day_fog_color: 0xff8ab4d8,
            night_fog_start: 80.0,
            night_fog_end: 520.0,
            night_fog_density: 0.6,
            night_fog_color: 0xff162038,
            uw_fog_start: 10.0,
            uw_fog_end: 100.0,
            uw_fog_density: 0.8,
            uw_fog_color: 0xff2492ff,
            light_dir: [0.34, -0.88, -0.34],
            day_ambient: 0xff5b5b5b,
            day_dir_light: 0xfffebe78,
            night_ambient: 0xff222a3e,
            night_dir_light: 0xff5a6f9a,
            cap_tile_speed: 8.0,
            cap_rot_speed: 0.0,
            day_sky_textures: [String::new(), String::new(), String::new()],
            night_sky_textures: [String::new(), String::new(), String::new()],
        }
    }
}

/// Water area reference parsed from `.ecwld` + `.ecbsd` (`ECBSDFILEWATER`).
#[derive(Debug, Clone, PartialEq)]
pub struct WaterAreaRef {
    pub export_id: u32,
    pub sub_terrain: i32,
    pub data_index: i32,
}

/// Rectangular water plane in world space (either decoded from `BSData/*.dat` or synthetic).
#[derive(Debug, Clone, PartialEq)]
pub struct WaterPlane {
    pub id: u32,
    pub center_x: f32,
    pub center_z: f32,
    pub half_x: f32,
    pub half_z: f32,
    pub height_y: f32,
    pub color_rgba: [f32; 4],
}

/// Screen-space scissor rectangle `(x, y, width, height)` in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScissorRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Test whether a water plane is potentially visible to the camera frustum (`view_proj`)
/// and compute its clamped screen-space scissor rectangle in pixels.
/// Returns `None` if the plane lies completely outside the frustum or behind the camera.
pub fn compute_water_scissor(
    plane: &WaterPlane,
    view_proj: Mat4,
    viewport_w: u32,
    viewport_h: u32,
) -> Option<ScissorRect> {
    if viewport_w == 0 || viewport_h == 0 || plane.half_x <= 0.0 || plane.half_z <= 0.0 {
        return None;
    }
    let corners = [
        Vec3::new(
            plane.center_x - plane.half_x,
            plane.height_y,
            plane.center_z - plane.half_z,
        ),
        Vec3::new(
            plane.center_x + plane.half_x,
            plane.height_y,
            plane.center_z - plane.half_z,
        ),
        Vec3::new(
            plane.center_x + plane.half_x,
            plane.height_y,
            plane.center_z + plane.half_z,
        ),
        Vec3::new(
            plane.center_x - plane.half_x,
            plane.height_y,
            plane.center_z + plane.half_z,
        ),
        Vec3::new(plane.center_x, plane.height_y, plane.center_z),
    ];

    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    let mut any_in_front = false;

    for c in corners {
        let clip: Vec4 = view_proj * c.extend(1.0);
        if clip.w > 0.05 {
            any_in_front = true;
            let ndc_x = (clip.x / clip.w).clamp(-1.0, 1.0);
            let ndc_y = (clip.y / clip.w).clamp(-1.0, 1.0);
            let sx = (ndc_x * 0.5 + 0.5) * (viewport_w as f32);
            let sy = (0.5 - ndc_y * 0.5) * (viewport_h as f32);
            min_x = min_x.min(sx);
            max_x = max_x.max(sx);
            min_y = min_y.min(sy);
            max_y = max_y.max(sy);
        }
    }

    if !any_in_front {
        return None;
    }
    let x0 = (min_x.floor().max(0.0) as u32).min(viewport_w);
    let y0 = (min_y.floor().max(0.0) as u32).min(viewport_h);
    let x1 = (max_x.ceil().clamp(0.0, viewport_w as f32) as u32).min(viewport_w);
    let y1 = (max_y.ceil().clamp(0.0, viewport_h as f32) as u32).min(viewport_h);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some(ScissorRect {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    })
}

/// Cull a list of water planes against `view_proj` and return only visible planes paired
/// with their screen-space scissor rectangles.
pub fn visible_water_planes(
    planes: &[WaterPlane],
    view_proj: Mat4,
    viewport_w: u32,
    viewport_h: u32,
) -> Vec<(usize, ScissorRect)> {
    planes
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            compute_water_scissor(p, view_proj, viewport_w, viewport_h).map(|r| (i, r))
        })
        .collect()
}

/// Sort transparent draw keys strictly back-to-front (farthest squared distance from
/// `camera_pos` first, with deterministic tie-breaking by `stable_id`).
pub fn sort_transparent_back_to_front<T>(
    items: &mut [T],
    camera_pos: Vec3,
    mut position_and_id: impl FnMut(&T) -> (Vec3, u64),
) {
    items.sort_by(|a, b| {
        let (pa, ida) = position_and_id(a);
        let (pb, idb) = position_and_id(b);
        let da = pa.distance_squared(camera_pos);
        let db = pb.distance_squared(camera_pos);
        db.total_cmp(&da).then_with(|| ida.cmp(&idb))
    });
}

/// Parse all deduplicated `ECBSDFILEBOXAREA15` sky/fog/lighting areas and `ECBSDFILEWATER`
/// water references from an `.ecwld` (v9+) + `.ecbsd` (v14+) pair.
pub fn parse_ecwld_ecbsd_atmosphere_and_water(
    wld: &[u8],
    bsd: &[u8],
) -> Result<
    (
        EcwldHeader,
        EcbsdHeader,
        Vec<BoxAreaAtmosphere>,
        Vec<WaterAreaRef>,
    ),
    String,
> {
    let wld_hdr = EcwldHeader::parse(wld)?;
    let bsd_hdr = EcbsdHeader::parse(bsd)?;
    if wld.len() < ECWLD_HEADER_BYTES {
        return Err("ecwld header truncated".into());
    }
    let num_blocks = wld_hdr.num_blocks as usize;
    let table_end = ECWLD_HEADER_BYTES
        .checked_add(num_blocks.checked_mul(4).ok_or("block table overflow")?)
        .ok_or("block table overflow")?;
    if table_end > wld.len() {
        return Err("ecwld block offset table truncated".into());
    }

    let mut seen_water = HashSet::new();
    let mut seen_box = HashSet::new();
    let mut waters = Vec::new();
    let mut boxes = Vec::new();

    for bi in 0..num_blocks {
        let boff = u32le(wld, ECWLD_HEADER_BYTES + bi * 4)? as usize;
        if boff + ECWLD_BLOCK9_BYTES > wld.len() {
            return Err(format!("ecwld block {bi} header truncated"));
        }
        let _row = u16le(wld, boff)?;
        let _col = u16le(wld, boff + 2)?;
        let num_tree = i32le(wld, boff + 4)?;
        let num_water = i32le(wld, boff + 8)?;
        let _num_grass = i32le(wld, boff + 12)?;
        let num_ornament = i32le(wld, boff + 16)?;
        let num_box = i32le(wld, boff + 20)?;
        if num_tree < 0 || num_water < 0 || num_ornament < 0 || num_box < 0 {
            return Err(format!("negative count in ecwld block {bi}"));
        }

        let mut p = boff + ECWLD_BLOCK9_BYTES;
        p = p
            .checked_add((num_tree as usize) * ECWLD_TREE5_BYTES)
            .ok_or("tree offset overflow")?;

        // Water refs
        for _ in 0..num_water {
            let eid = u32le(wld, p)?;
            let bsd_off = u32le(wld, p + 4)? as usize;
            p += ECWLD_DATAREF_BYTES;
            if seen_water.insert(eid) && bsd_off + 8 <= bsd.len() {
                waters.push(WaterAreaRef {
                    export_id: eid,
                    sub_terrain: i32le(bsd, bsd_off)?,
                    data_index: i32le(bsd, bsd_off + 4)?,
                });
            }
        }

        // Skip ornament refs
        p = p
            .checked_add((num_ornament as usize) * ECWLD_DATAREF_BYTES)
            .ok_or("ornament offset overflow")?;

        // Box area refs
        for _ in 0..num_box {
            let eid = u32le(wld, p)?;
            let bsd_off = u32le(wld, p + 4)? as usize;
            p += ECWLD_DATAREF_BYTES;
            if seen_box.insert(eid) && bsd_off + ECBSD_BOXAREA_MIN_BYTES <= bsd.len() {
                let is_default = i32le(bsd, bsd_off)? != 0;
                let pos = [
                    f32le(bsd, bsd_off + 8)?,
                    f32le(bsd, bsd_off + 12)?,
                    f32le(bsd, bsd_off + 16)?,
                ];
                let ext = [
                    f32le(bsd, bsd_off + 20)?,
                    f32le(bsd, bsd_off + 24)?,
                    f32le(bsd, bsd_off + 28)?,
                ];
                let day_fog_start = f32le(bsd, bsd_off + 68)?;
                let day_fog_end = f32le(bsd, bsd_off + 72)?;
                let day_fog_density = f32le(bsd, bsd_off + 76)?;
                let day_fog_color = u32le(bsd, bsd_off + 80)?;
                let uw_fog_start = f32le(bsd, bsd_off + 84)?;
                let uw_fog_end = f32le(bsd, bsd_off + 88)?;
                let uw_fog_density = f32le(bsd, bsd_off + 92)?;
                let uw_fog_color = u32le(bsd, bsd_off + 96)?;
                let night_fog_start = f32le(bsd, bsd_off + 100)?;
                let night_fog_end = f32le(bsd, bsd_off + 104)?;
                let night_fog_density = f32le(bsd, bsd_off + 108)?;
                let night_fog_color = u32le(bsd, bsd_off + 112)?;
                let light_dir = [
                    f32le(bsd, bsd_off + 116)?,
                    f32le(bsd, bsd_off + 120)?,
                    f32le(bsd, bsd_off + 124)?,
                ];
                let day_ambient = u32le(bsd, bsd_off + 128)?;
                let day_dir_light = u32le(bsd, bsd_off + 132)?;
                let night_ambient = u32le(bsd, bsd_off + 136)?;
                let night_dir_light = u32le(bsd, bsd_off + 140)?;
                boxes.push(BoxAreaAtmosphere {
                    export_id: eid,
                    is_default,
                    position: pos,
                    extents: ext,
                    day_fog_start,
                    day_fog_end,
                    day_fog_density,
                    day_fog_color,
                    night_fog_start,
                    night_fog_end,
                    night_fog_density,
                    night_fog_color,
                    uw_fog_start,
                    uw_fog_end,
                    uw_fog_density,
                    uw_fog_color,
                    light_dir,
                    day_ambient,
                    day_dir_light,
                    night_ambient,
                    night_dir_light,
                    cap_tile_speed: f32le(bsd, bsd_off + 272).unwrap_or(8.0),
                    cap_rot_speed: f32le(bsd, bsd_off + 276).unwrap_or(0.0),
                    day_sky_textures: {
                        let mut p_str = bsd_off + 280;
                        let mut read_str = || -> String {
                            if let Ok(slen) = u32le(bsd, p_str) {
                                let sl = slen as usize;
                                p_str += 4;
                                if sl <= 512 && p_str + sl <= bsd.len() {
                                    let (dec, _, _) =
                                        encoding_rs::GB18030.decode(&bsd[p_str..p_str + sl]);
                                    p_str += sl;
                                    return dec.into_owned();
                                }
                            }
                            String::new()
                        };
                        [read_str(), read_str(), read_str()]
                    },
                    night_sky_textures: {
                        let mut p_str = bsd_off + 280;
                        for _ in 0..3 {
                            if let Ok(slen) = u32le(bsd, p_str) {
                                let sl = slen as usize;
                                if sl <= 512 && p_str + 4 + sl <= bsd.len() {
                                    p_str += 4 + sl;
                                } else {
                                    break;
                                }
                            }
                        }
                        let mut read_str = || -> String {
                            if let Ok(slen) = u32le(bsd, p_str) {
                                let sl = slen as usize;
                                p_str += 4;
                                if sl <= 512 && p_str + sl <= bsd.len() {
                                    let (dec, _, _) =
                                        encoding_rs::GB18030.decode(&bsd[p_str..p_str + sl]);
                                    p_str += sl;
                                    return dec.into_owned();
                                }
                            }
                            String::new()
                        };
                        [read_str(), read_str(), read_str()]
                    },
                });
            }
        }
    }

    Ok((wld_hdr, bsd_hdr, boxes, waters))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::FreeCamera;

    #[test]
    fn time_of_day_wraparound_and_lightmap_blend_factor() {
        assert!((advance_time_of_day(23.5, 1.0) - 0.5).abs() < 1e-5);
        assert!((advance_time_of_day(0.25, -1.0) - 23.25).abs() < 1e-5);
        assert!((wrap_hours(-2.0) - 22.0).abs() < 1e-5);

        // Noon is full day (0.0), midnight is full night (1.0)
        assert_eq!(day_night_factor(12.0), 0.0);
        assert_eq!(day_night_factor(0.0), 1.0);
        assert_eq!(day_night_factor(24.0), 1.0);
        // Dawn midpoint (05:00) = 0.5; dusk midpoint (19:30) = 0.5
        assert!((day_night_factor(5.0) - 0.5).abs() < 1e-5);
        assert!((day_night_factor(19.5) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn fog_factor_curves_at_fixed_distances() {
        assert_eq!(evaluate_fog_factor(300.0, 50.0, 250.0, 1.0, false), 0.0);
        assert_eq!(evaluate_fog_factor(50.0, 50.0, 250.0, 1.0, true), 0.0);
        assert!((evaluate_fog_factor(150.0, 50.0, 250.0, 1.0, true) - 0.5).abs() < 1e-5);
        // 2x multiplier doubles start/end to 100..500 -> at 300m fog is 0.5
        assert!((evaluate_fog_factor(300.0, 50.0, 250.0, 2.0, true) - 0.5).abs() < 1e-5);
        assert!((evaluate_fog_factor(1000.0, 50.0, 250.0, 1.0, true) - 0.92).abs() < 1e-5);
    }

    #[test]
    fn water_plane_visibility_culling_and_scissor_rect() {
        let cam = FreeCamera {
            position: Vec3::new(0.0, 40.0, 0.0),
            yaw: 0.0, // faces +Z
            pitch: -0.3,
            ..Default::default()
        };
        let vp = cam.view_proj(16.0 / 9.0);
        let in_front = WaterPlane {
            id: 1,
            center_x: 0.0,
            center_z: 100.0,
            half_x: 40.0,
            half_z: 40.0,
            height_y: 10.0,
            color_rgba: [0.1, 0.4, 0.6, 0.85],
        };
        let behind = WaterPlane {
            id: 2,
            center_x: 0.0,
            center_z: -200.0,
            half_x: 30.0,
            half_z: 30.0,
            height_y: 10.0,
            color_rgba: [0.1, 0.4, 0.6, 0.85],
        };
        let vis = visible_water_planes(&[in_front, behind], vp, 1280, 720);
        assert_eq!(vis.len(), 1);
        assert_eq!(vis[0].0, 0);
        let rect = vis[0].1;
        assert!(rect.width > 0 && rect.height > 0);
        assert!(rect.x + rect.width <= 1280 && rect.y + rect.height <= 720);
    }

    #[test]
    fn transparent_sort_order_is_strictly_back_to_front() {
        let mut items = vec![
            ("near", Vec3::new(0.0, 0.0, 10.0), 1u64),
            ("far", Vec3::new(0.0, 0.0, 90.0), 2u64),
            ("mid", Vec3::new(0.0, 0.0, 40.0), 3u64),
        ];
        sort_transparent_back_to_front(&mut items, Vec3::ZERO, |it| (it.1, it.2));
        let names: Vec<&str> = items.into_iter().map(|it| it.0).collect();
        assert_eq!(names, vec!["far", "mid", "near"]);
    }
}

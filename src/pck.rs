//! Reader for Forsaken World v2 `.pck` / `.pkx` archives.
//!
//! Every constant and offset below is verified against real `Forsaken World 0.525 Client`
//! archives (`resources/configs.pck` and `resources/grasses.pck`) and documented in
//! `Findings/archive-format.md`.

use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use encoding_rs::GB18030;
use flate2::{Decompress, FlushDecompress, Status};

pub const SAFE_GUARD0: u32 = 0x4dca_23ef;
pub const SAFE_GUARD1: u32 = 0x56a0_89b7;
pub const FOOTER_GUARD0: u32 = 0xa508_bdc4;
pub const FOOTER_GUARD1: u32 = 0x8535_67c8;
pub const PCK_VERSION_2_0_2: u32 = 0x0002_0002;
pub const PCK_VERSION_2_0_1: u32 = 0x0002_0001;
pub const ENTRY_MASK: u32 = 0x21c3_1a3f;
pub const ENTRY_CHECK: u32 = 0x399f_3a1a;

pub const SAFE_HEADER_LEN: usize = 12;
pub const FOOTER_LEN: usize = 280;
pub const ENTRY_RECORD_LEN: usize = 276;
pub const MAX_ENTRY_COUNT: usize = 1_000_000;
/// Maximum single payload size (256 MiB) to guard against crafted archives.
pub const MAX_PAYLOAD_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub offset: u64,
    pub size: u32,
    pub compressed: u32,
    pub extra: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveInfo {
    pub version: u32,
    pub entry_count: usize,
    pub logical_size: u64,
    pub pck_size: u64,
    pub pkx_size: u64,
    pub description: String,
}

pub struct PckArchive {
    pck: File,
    pck_size: u64,
    pkx: Option<File>,
    pkx_size: u64,
    info: ArchiveInfo,
    entries: Vec<Entry>,
    index_by_norm: HashMap<String, usize>,
}

fn u32le(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let slice = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| format!("truncated u32 at offset {offset}"))?;
    Ok(u32::from_le_bytes(slice.try_into().unwrap()))
}

pub fn normalize_entry_path(path: &str) -> String {
    path.trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_start_matches('/')
        .to_ascii_lowercase()
}

pub fn companion_pkx_path(pck_path: &Path) -> PathBuf {
    pck_path.with_extension("pkx")
}

fn read_exact_at(file: &mut File, offset: u64, buf: &mut [u8]) -> Result<(), String> {
    file.seek(SeekFrom::Start(offset))
        .and_then(|_| file.read_exact(buf))
        .map_err(|e| format!("read error at offset {offset}: {e}"))
}

fn read_logical_range(
    pck: &mut File,
    pck_size: u64,
    pkx: &mut Option<File>,
    pkx_size: u64,
    offset: u64,
    len: usize,
) -> Result<Vec<u8>, String> {
    let total = pck_size
        .checked_add(pkx_size)
        .ok_or_else(|| "archive total size overflow".to_string())?;
    let end = offset
        .checked_add(len as u64)
        .ok_or_else(|| "archive read range overflow".to_string())?;
    if end > total {
        return Err(format!(
            "read range {offset}..{end} exceeds archive size {total}"
        ));
    }
    let mut out = vec![0u8; len];
    if len == 0 {
        return Ok(out);
    }
    if end <= pck_size {
        read_exact_at(pck, offset, &mut out)?;
    } else if offset >= pck_size {
        let pkx_file = pkx
            .as_mut()
            .ok_or_else(|| "archive references split .pkx file which is missing".to_string())?;
        read_exact_at(pkx_file, offset - pck_size, &mut out)?;
    } else {
        let first_len = (pck_size - offset) as usize;
        read_exact_at(pck, offset, &mut out[..first_len])?;
        let pkx_file = pkx
            .as_mut()
            .ok_or_else(|| "archive spans into split .pkx file which is missing".to_string())?;
        read_exact_at(pkx_file, 0, &mut out[first_len..])?;
    }
    Ok(out)
}

pub fn inflate_exact(raw: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    if expected > MAX_PAYLOAD_BYTES {
        return Err(format!(
            "declared decompressed size {expected} exceeds safety limit {MAX_PAYLOAD_BYTES}"
        ));
    }
    let mut decomp = Decompress::new(true);
    let cap = expected
        .checked_add(1)
        .ok_or_else(|| "zlib size overflow".to_string())?;
    let mut out = Vec::with_capacity(cap);
    let status = decomp
        .decompress_vec(raw, &mut out, FlushDecompress::Finish)
        .map_err(|e| format!("zlib decompression failed: {e}"))?;
    if out.len() != expected {
        return Err(format!(
            "decompressed size mismatch: got {}, expected {expected}",
            out.len()
        ));
    }
    if status != Status::StreamEnd {
        return Err("truncated or trailing zlib stream".into());
    }
    Ok(out)
}

impl PckArchive {
    /// Open a Forsaken World `.pck` archive, automatically attaching a companion
    /// `.pkx` file when the archive's logical end exceeds the `.pck` size.
    pub fn open(path: &Path) -> Result<Self, String> {
        let mut pck =
            File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        let pck_size = pck
            .metadata()
            .map_err(|e| format!("cannot stat {}: {e}", path.display()))?
            .len();

        let pkx_path = companion_pkx_path(path);
        let (mut pkx, pkx_size) = if pkx_path.is_file() {
            let file = File::open(&pkx_path)
                .map_err(|e| format!("cannot open companion {}: {e}", pkx_path.display()))?;
            let len = file
                .metadata()
                .map_err(|e| format!("cannot stat {}: {e}", pkx_path.display()))?
                .len();
            (Some(file), len)
        } else {
            (None, 0)
        };

        let total_size = pck_size
            .checked_add(pkx_size)
            .ok_or_else(|| "archive size overflow".to_string())?;
        if total_size < (SAFE_HEADER_LEN + FOOTER_LEN) as u64 {
            return Err("archive too small for PCK header and footer".into());
        }

        let safe = read_logical_range(&mut pck, pck_size, &mut pkx, pkx_size, 0, SAFE_HEADER_LEN)?;
        if u32le(&safe, 0)? != SAFE_GUARD0 || u32le(&safe, 8)? != SAFE_GUARD1 {
            return Err("unsupported or invalid PCK safe header".into());
        }
        let end = u32le(&safe, 4)? as u64;
        if end < (SAFE_HEADER_LEN + FOOTER_LEN) as u64 || end > total_size {
            return Err(format!(
                "invalid PCK logical end {end} (available bytes: {total_size})"
            ));
        }

        let footer_base = end - FOOTER_LEN as u64;
        let footer = read_logical_range(
            &mut pck,
            pck_size,
            &mut pkx,
            pkx_size,
            footer_base,
            FOOTER_LEN,
        )?;
        if u32le(&footer, 0)? != FOOTER_GUARD0 || u32le(&footer, 268)? != FOOTER_GUARD1 {
            return Err("unsupported PCK guard keys (not a recognized Forsaken World PCK)".into());
        }
        let version = u32le(&footer, 4)?;
        let version_tail = u32le(&footer, 276)?;
        if version != version_tail || (version != PCK_VERSION_2_0_2 && version != PCK_VERSION_2_0_1)
        {
            return Err(format!(
                "unsupported PCK version 0x{version:08x} / 0x{version_tail:08x}"
            ));
        }
        let flags = u32le(&footer, 12)?;
        if flags & 0x8000_0000 != 0 {
            return Err("encrypted PCK packages are not supported".into());
        }
        let count = u32le(&footer, 272)? as usize;
        if count > MAX_ENTRY_COUNT {
            return Err(format!(
                "entry count {count} exceeds safety limit {MAX_ENTRY_COUNT}"
            ));
        }

        let desc_raw = &footer[16..268];
        let desc_end = desc_raw
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(desc_raw.len());
        let description = String::from_utf8_lossy(&desc_raw[..desc_end]).into_owned();

        let table_pos = (u32le(&footer, 8)? ^ ENTRY_MASK) as u64;
        if table_pos < SAFE_HEADER_LEN as u64 || table_pos > footer_base {
            return Err(format!(
                "entry table offset {table_pos} outside valid range {SAFE_HEADER_LEN}..={footer_base}"
            ));
        }
        let table_len = (footer_base - table_pos) as usize;
        let table =
            read_logical_range(&mut pck, pck_size, &mut pkx, pkx_size, table_pos, table_len)?;

        let mut cursor = 0usize;
        let mut entries = Vec::with_capacity(count);
        let mut index_by_norm = HashMap::with_capacity(count);

        for i in 0..count {
            if cursor + 8 > table.len() {
                return Err(format!("entry table truncated at record {i}"));
            }
            let w0 = u32le(&table, cursor)?;
            let w1 = u32le(&table, cursor + 4)?;
            let rec_len = (w0 ^ ENTRY_MASK) as usize;
            if (w1 ^ ENTRY_MASK ^ ENTRY_CHECK) != rec_len as u32
                || rec_len == 0
                || rec_len > ENTRY_RECORD_LEN
                || cursor + 8 + rec_len > table.len()
            {
                return Err(format!("invalid entry record header at index {i}"));
            }
            let raw = &table[cursor + 8..cursor + 8 + rec_len];
            let decoded;
            let rec: &[u8] = if rec_len == ENTRY_RECORD_LEN {
                raw
            } else {
                decoded = inflate_exact(raw, ENTRY_RECORD_LEN)?;
                &decoded
            };

            let name_bytes = &rec[..260];
            let nul = name_bytes
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(name_bytes.len());
            let (name_cow, _, _) = GB18030.decode(&name_bytes[..nul]);
            let name = name_cow.into_owned();
            let offset = u32le(rec, 260)? as u64;
            let size = u32le(rec, 264)?;
            let compressed = u32le(rec, 268)?;
            let extra = u32le(rec, 272)?;

            let payload_end = offset
                .checked_add(compressed as u64)
                .ok_or_else(|| format!("entry {i} payload offset overflow"))?;
            if payload_end > table_pos {
                return Err(format!(
                    "entry {i} ('{name}') payload {offset}..{payload_end} overlaps entry table at {table_pos}"
                ));
            }

            let norm = normalize_entry_path(&name);
            index_by_norm.entry(norm).or_insert(i);
            entries.push(Entry {
                name,
                offset,
                size,
                compressed,
                extra,
            });
            cursor += 8 + rec_len;
        }

        let info = ArchiveInfo {
            version,
            entry_count: entries.len(),
            logical_size: end,
            pck_size,
            pkx_size,
            description,
        };

        Ok(Self {
            pck,
            pck_size,
            pkx,
            pkx_size,
            info,
            entries,
            index_by_norm,
        })
    }

    pub fn info(&self) -> &ArchiveInfo {
        &self.info
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn find_index(&self, name: &str) -> Option<usize> {
        let key = normalize_entry_path(name);
        self.index_by_norm.get(&key).copied()
    }

    pub fn read_entry_by_index(&mut self, index: usize) -> Result<Vec<u8>, String> {
        let entry = self
            .entries
            .get(index)
            .ok_or_else(|| format!("entry index {index} out of bounds"))?
            .clone();
        let raw = read_logical_range(
            &mut self.pck,
            self.pck_size,
            &mut self.pkx,
            self.pkx_size,
            entry.offset,
            entry.compressed as usize,
        )?;
        if entry.compressed == entry.size {
            Ok(raw)
        } else {
            inflate_exact(&raw, entry.size as usize)
        }
    }

    pub fn read_entry(&mut self, name: &str) -> Result<Vec<u8>, String> {
        let index = self
            .find_index(name)
            .ok_or_else(|| format!("entry not found: {name}"))?;
        self.read_entry_by_index(index)
    }
}

/// Synthetic `.pck` / `.pkx` builder for deterministic unit tests, fuzzing, and `--self-test`.
pub mod synthetic {
    use super::*;
    use flate2::{write::ZlibEncoder, Compression};
    use std::{fs, io::Write};

    fn compress_zlib(data: &[u8]) -> Vec<u8> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data).expect("zlib write");
        encoder.finish().expect("zlib finish")
    }

    /// Build a valid Forsaken World v2 PCK byte stream in memory.
    pub fn build_pck_bytes(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        // Reserve 12-byte safe header.
        out.extend_from_slice(&[0u8; SAFE_HEADER_LEN]);

        let mut records = Vec::with_capacity(files.len());
        for &(name, payload) in files {
            let offset = out.len() as u32;
            let compressed = compress_zlib(payload);
            let (stored, stored_len) = if compressed.len() < payload.len() {
                let len = compressed.len() as u32;
                (compressed, len)
            } else {
                (payload.to_vec(), payload.len() as u32)
            };
            out.extend_from_slice(&stored);

            let mut rec = [0u8; ENTRY_RECORD_LEN];
            let (encoded_name, _, _) = GB18030.encode(name);
            let copy_len = encoded_name.len().min(259);
            rec[..copy_len].copy_from_slice(&encoded_name[..copy_len]);
            rec[260..264].copy_from_slice(&offset.to_le_bytes());
            rec[264..268].copy_from_slice(&(payload.len() as u32).to_le_bytes());
            rec[268..272].copy_from_slice(&stored_len.to_le_bytes());
            rec[272..276].copy_from_slice(&0u32.to_le_bytes());
            records.push(rec);
        }

        let table_pos = out.len() as u32;
        for rec in &records {
            let comp = compress_zlib(rec);
            let stored = if comp.len() < ENTRY_RECORD_LEN {
                comp
            } else {
                rec.to_vec()
            };
            let len = stored.len() as u32;
            let w0 = len ^ ENTRY_MASK;
            let w1 = len ^ ENTRY_MASK ^ ENTRY_CHECK;
            out.extend_from_slice(&w0.to_le_bytes());
            out.extend_from_slice(&w1.to_le_bytes());
            out.extend_from_slice(&stored);
        }

        let mut footer = [0u8; FOOTER_LEN];
        footer[0..4].copy_from_slice(&FOOTER_GUARD0.to_le_bytes());
        footer[4..8].copy_from_slice(&PCK_VERSION_2_0_2.to_le_bytes());
        footer[8..12].copy_from_slice(&(table_pos ^ ENTRY_MASK).to_le_bytes());
        footer[12..16].copy_from_slice(&0u32.to_le_bytes());
        let desc = b"Synthetic Forsaken World PCK fixture";
        footer[16..16 + desc.len()].copy_from_slice(desc);
        footer[268..272].copy_from_slice(&FOOTER_GUARD1.to_le_bytes());
        footer[272..276].copy_from_slice(&(files.len() as u32).to_le_bytes());
        footer[276..280].copy_from_slice(&PCK_VERSION_2_0_2.to_le_bytes());
        out.extend_from_slice(&footer);

        let end = out.len() as u32;
        out[0..4].copy_from_slice(&SAFE_GUARD0.to_le_bytes());
        out[4..8].copy_from_slice(&end.to_le_bytes());
        out[8..12].copy_from_slice(&SAFE_GUARD1.to_le_bytes());
        out
    }

    /// Write a single-file `.pck` fixture to `path`.
    pub fn write_pck(path: &Path, files: &[(&str, &[u8])]) -> Result<(), String> {
        let bytes = build_pck_bytes(files);
        fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Write a split `.pck` + `.pkx` fixture split at `split_at` bytes.
    pub fn write_split_pck(
        pck_path: &Path,
        files: &[(&str, &[u8])],
        split_at: usize,
    ) -> Result<(), String> {
        let bytes = build_pck_bytes(files);
        if split_at == 0 || split_at >= bytes.len() {
            return Err(format!(
                "split_at {split_at} must lie inside 1..{}",
                bytes.len()
            ));
        }
        let pkx_path = companion_pkx_path(pck_path);
        fs::write(pck_path, &bytes[..split_at])
            .map_err(|e| format!("{}: {e}", pck_path.display()))?;
        fs::write(&pkx_path, &bytes[split_at..])
            .map_err(|e| format!("{}: {e}", pkx_path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        env, fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = env::temp_dir().join(format!(
            "fw-explorer-pck-{tag}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn round_trip_single_pck_and_case_insensitive_lookup() {
        let dir = temp_dir("single");
        let path = dir.join("synthetic.pck");
        let compressible = b"Forsaken World Explorer synthetic terrain config!\n".repeat(40);
        let tiny = b"raw";
        synthetic::write_pck(
            &path,
            &[
                ("Configs\\World\\Sample.cfg", &compressible),
                ("tiny.bin", tiny),
                ("empty.txt", b""),
            ],
        )
        .expect("write fixture");

        let mut archive = PckArchive::open(&path).expect("open pck");
        assert_eq!(archive.info().version, PCK_VERSION_2_0_2);
        assert_eq!(archive.entries().len(), 3);
        assert_eq!(
            archive.read_entry("configs/world/sample.cfg").unwrap(),
            compressible
        );
        assert_eq!(archive.read_entry("TINY.BIN").unwrap(), tiny);
        assert_eq!(archive.read_entry("empty.txt").unwrap(), b"");
        assert!(archive.read_entry("missing.txt").is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn round_trip_split_pck_and_pkx_across_boundary() {
        let dir = temp_dir("split");
        let pck_path = dir.join("models.pck");
        let blob_a = vec![0x41u8; 256];
        let blob_b: Vec<u8> = (0u8..=255).cycle().take(1024).collect();
        // Split right inside the payload area so both payload and footer span .pck/.pkx.
        synthetic::write_split_pck(
            &pck_path,
            &[("models\\a.bin", &blob_a), ("models\\b.bin", &blob_b)],
            120,
        )
        .expect("write split pck");

        let mut archive = PckArchive::open(&pck_path).expect("open split pck+pkx");
        assert!(archive.info().pkx_size > 0);
        assert_eq!(archive.read_entry("models/a.bin").unwrap(), blob_a);
        assert_eq!(archive.read_entry("models/b.bin").unwrap(), blob_b);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_and_fuzzed_inputs_return_error_without_panicking() {
        let dir = temp_dir("fuzz");
        let path = dir.join("bad.pck");

        // Empty and truncated files
        for len in [0usize, 4, 12, 291, 292] {
            fs::write(&path, vec![0u8; len]).unwrap();
            assert!(PckArchive::open(&path).is_err());
        }

        // Deterministic pseudo-random byte fuzzing (no external RNG dependency)
        let mut seed: u64 = 0x5eed_cafe_1234_5678;
        for case in 0..32 {
            let len = 300 + (case * 37);
            let mut bytes = Vec::with_capacity(len);
            for _ in 0..len {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                bytes.push((seed >> 33) as u8);
            }
            fs::write(&path, &bytes).unwrap();
            assert!(PckArchive::open(&path).is_err());
        }

        // Corrupt a valid archive's guard keys and entry table
        let mut valid = synthetic::build_pck_bytes(&[("test.txt", b"hello world")]);
        let end = valid.len();
        valid[end - 280] ^= 0xff;
        fs::write(&path, &valid).unwrap();
        assert!(PckArchive::open(&path).is_err());

        let _ = fs::remove_dir_all(&dir);
    }
}

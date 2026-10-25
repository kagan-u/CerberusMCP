//! Compiled registry cache (`conf.cerberus` → rkyv binary snapshot).
//!
//! The cache is a pure derived artifact: every byte can be recomputed from
//! the source document, so it may be deleted at any time and is rebuilt on
//! the next load. A cached read is only taken after three independent
//! guards succeed: the container header (magic, schema, cache revision,
//! build identity), a stat of the source (mtime + length), and FNV-1a
//! hashes of both the source content and the payload. rkyv's bytecheck
//! validation then walks the archived graph in place before anything is
//! materialized. Any failure at any step is a silent miss — parsing the
//! source document is always a correct fallback.
//!
//! Container layout (all little-endian; header is [`HEADER_LEN`] bytes):
//!
//! ```text
//! 0..4    magic b"CBIN"
//! 4..6    container version (u16)
//! 6..8    cache revision (u16)   — bump CACHE_REV when schema types or
//!                                   validation rules change within the same
//!                                   CARGO_PKG_VERSION (dev builds)
//! 8..12   schema version (u32)
//! 12..20  source mtime, seconds since epoch (i64)
//! 20..24  source mtime, nanoseconds (u32)
//! 24..32  source length in bytes (u64)
//! 32..40  source content hash, FNV-1a 64 (u64)
//! 40..48  payload length in bytes (u64)
//! 48..56  payload hash, FNV-1a 64 (u64)
//! 56..64  build identity: FNV-1a 64 of CARGO_PKG_VERSION (u64)
//! 64..    rkyv payload (the archived Registry)
//! ```
//!
//! `CERBERUS_NO_CACHE=1` disables both reads and writes.
//! `cerberus config compile` always writes, so a warm cache can be built or
//! inspected explicitly regardless of the environment switch.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use rkyv::rancor::Error as RkyvError;

use crate::error::{CerberusError, Result};
use crate::paths::CerberusPaths;

use super::schema::{Registry, SCHEMA_VERSION};
use super::{atomic_write, hash_bytes, hash_text, validate};

/// Magic bytes at the start of every cache file.
const MAGIC: [u8; 4] = *b"CBIN";
/// Header size; the payload starts at this offset (a multiple of any
/// alignment the archived types can require, so a heap-allocated file
/// buffer yields an aligned payload slice).
const HEADER_LEN: usize = 64;
/// Version of the container header itself (independent of the schema).
const CONTAINER_VERSION: u16 = 1;
/// Bump when schema types or validation rules change in a way that
/// invalidates caches within the same `CARGO_PKG_VERSION`.
pub const CACHE_REV: u16 = 1;

/// Details about a written cache file (for `config compile`).
#[derive(Debug, Clone)]
pub struct CacheInfo {
    pub path: PathBuf,
    pub container_bytes: u64,
    pub payload_bytes: u64,
    pub source_fnv: u64,
    pub payload_fnv: u64,
}

/// Cache reads and writes are off when `CERBERUS_NO_CACHE` is set to any
/// value other than `0`, `false`, or the empty string.
fn cache_disabled() -> bool {
    std::env::var_os("CERBERUS_NO_CACHE")
        .is_some_and(|v| !matches!(v.to_str(), Some("0" | "false" | "")))
}

/// Build identity stored in the header: caches do not survive a release
/// upgrade even if the schema version is unchanged.
fn build_identity() -> u64 {
    hash_text(env!("CARGO_PKG_VERSION"))
}

/// Cache file for `source` under this configuration's cache directory.
/// The name keys off the source path so distinct `--config` files never
/// collide.
pub fn cache_file_for(paths: &CerberusPaths, source: &Path) -> PathBuf {
    let key = hash_text(&source.to_string_lossy());
    paths.cache_dir().join(format!("registry-{key:016x}.cbin"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Header {
    container_version: u16,
    cache_rev: u16,
    schema_version: u32,
    source_mtime_sec: i64,
    source_mtime_nsec: u32,
    source_len: u64,
    source_fnv: u64,
    payload_len: u64,
    payload_fnv: u64,
    build_id: u64,
}

/// `(mtime_secs, mtime_nanos, len)` of `path`, or `None` when unreadable
/// or modified before the Unix epoch.
fn stat_source(path: &Path) -> Option<(i64, u32, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some((mtime.as_secs() as i64, mtime.subsec_nanos(), meta.len()))
}

fn put<const N: usize>(buf: &mut [u8], at: usize, bytes: [u8; N]) {
    buf[at..at + N].copy_from_slice(&bytes);
}

fn get<const N: usize>(buf: &[u8], at: usize) -> [u8; N] {
    buf[at..at + N]
        .try_into()
        .expect("header slice bounds checked")
}

fn encode_header(h: &Header) -> [u8; HEADER_LEN] {
    let mut buf = [0u8; HEADER_LEN];
    buf[0..4].copy_from_slice(&MAGIC);
    put(&mut buf, 4, h.container_version.to_le_bytes());
    put(&mut buf, 6, h.cache_rev.to_le_bytes());
    put(&mut buf, 8, h.schema_version.to_le_bytes());
    put(&mut buf, 12, h.source_mtime_sec.to_le_bytes());
    put(&mut buf, 20, h.source_mtime_nsec.to_le_bytes());
    put(&mut buf, 24, h.source_len.to_le_bytes());
    put(&mut buf, 32, h.source_fnv.to_le_bytes());
    put(&mut buf, 40, h.payload_len.to_le_bytes());
    put(&mut buf, 48, h.payload_fnv.to_le_bytes());
    put(&mut buf, 56, h.build_id.to_le_bytes());
    buf
}

/// Decode the fixed header; `None` only when the buffer is too short or
/// the magic does not match. Semantic checks (versions, stat, hashes) are
/// the caller's job.
fn decode_header(bytes: &[u8]) -> Option<Header> {
    if bytes.len() < HEADER_LEN || bytes[0..4] != MAGIC {
        return None;
    }
    Some(Header {
        container_version: u16::from_le_bytes(get(bytes, 4)),
        cache_rev: u16::from_le_bytes(get(bytes, 6)),
        schema_version: u32::from_le_bytes(get(bytes, 8)),
        source_mtime_sec: i64::from_le_bytes(get(bytes, 12)),
        source_mtime_nsec: u32::from_le_bytes(get(bytes, 20)),
        source_len: u64::from_le_bytes(get(bytes, 24)),
        source_fnv: u64::from_le_bytes(get(bytes, 32)),
        payload_len: u64::from_le_bytes(get(bytes, 40)),
        payload_fnv: u64::from_le_bytes(get(bytes, 48)),
        build_id: u64::from_le_bytes(get(bytes, 56)),
    })
}

/// Try to load `source`'s compiled snapshot, returning the registry plus
/// the source content hash the cache validated against.
///
/// Every failure mode is a cache miss: disabled by env, missing or
/// truncated file, stale stat, changed content, corrupt payload, or an
/// incompatible schema/cache-revision/build. Parsing the source document
/// is always a correct fallback, so this never fails loudly.
pub fn try_load_cache(paths: &CerberusPaths, source: &Path) -> Option<(Registry, u64)> {
    if cache_disabled() {
        return None;
    }
    let source_stat = stat_source(source)?;
    let cache_path = cache_file_for(paths, source);
    let bytes = std::fs::read(&cache_path).ok()?;
    let header = decode_header(&bytes)?;
    if header.container_version != CONTAINER_VERSION
        || header.cache_rev != CACHE_REV
        || header.schema_version != SCHEMA_VERSION
        || header.build_id != build_identity()
    {
        tracing::debug!(cache = %cache_path.display(), "registry cache header mismatch");
        return None;
    }
    if (
        header.source_mtime_sec,
        header.source_mtime_nsec,
        header.source_len,
    ) != source_stat
    {
        tracing::debug!(cache = %cache_path.display(), "registry cache is stale");
        return None;
    }
    let text = std::fs::read_to_string(source).ok()?;
    let source_fnv = hash_text(&text);
    if source_fnv != header.source_fnv {
        tracing::debug!(cache = %cache_path.display(), "registry cache content hash mismatch");
        return None;
    }
    let payload = bytes.get(HEADER_LEN..)?;
    if payload.len() as u64 != header.payload_len || hash_bytes(payload) != header.payload_fnv {
        tracing::debug!(cache = %cache_path.display(), "registry cache payload is corrupt");
        return None;
    }
    let registry = rkyv::from_bytes::<Registry, RkyvError>(payload).ok()?;
    tracing::debug!(cache = %cache_path.display(), "registry loaded from compiled cache");
    Some((registry, source_fnv))
}

/// Best-effort compile: never fails loudly, never returns an error.
/// Returns `None` when caching is disabled, the registry fails
/// validation, the source cannot be stat'ed, or writing fails.
pub fn write_cache(
    paths: &CerberusPaths,
    source: &Path,
    registry: &Registry,
    source_fnv: u64,
) -> Option<CacheInfo> {
    if cache_disabled() {
        return None;
    }
    match compile_cache(paths, source, registry, source_fnv) {
        Ok(info) => Some(info),
        Err(e) => {
            tracing::debug!(error = %e, "registry cache not written");
            None
        }
    }
}

/// Compile and write the cache explicitly. Used by `cerberus config
/// compile` and by the best-effort [`write_cache`]. Validation runs first:
/// an invalid registry is never cached, so cache hits cannot mask a
/// configuration error.
pub fn compile_cache(
    paths: &CerberusPaths,
    source: &Path,
    registry: &Registry,
    source_fnv: u64,
) -> Result<CacheInfo> {
    // The header records the *build's* schema version, so a registry from
    // another schema must never be compiled into it (parsing rejects these
    // too; this is belt and braces for direct constructions).
    if registry.schema_version != SCHEMA_VERSION {
        return Err(CerberusError::UnsupportedSchemaVersion {
            found: registry.schema_version,
            supported: SCHEMA_VERSION,
        });
    }
    validate::validate(registry)?;
    let payload = rkyv::to_bytes::<RkyvError>(registry)
        .map_err(|e| CerberusError::Other(format!("failed to compile registry cache: {e}")))?;
    let (mtime_sec, mtime_nsec, source_len) = stat_source(source).ok_or_else(|| {
        CerberusError::Other(format!(
            "cannot stat registry file {} for the cache",
            source.display()
        ))
    })?;
    let header = Header {
        container_version: CONTAINER_VERSION,
        cache_rev: CACHE_REV,
        schema_version: SCHEMA_VERSION,
        source_mtime_sec: mtime_sec,
        source_mtime_nsec: mtime_nsec,
        source_len,
        source_fnv,
        payload_len: payload.len() as u64,
        payload_fnv: hash_bytes(&payload),
        build_id: build_identity(),
    };
    let mut buf = Vec::with_capacity(HEADER_LEN + payload.len());
    buf.extend_from_slice(&encode_header(&header));
    buf.extend_from_slice(&payload);
    let path = cache_file_for(paths, source);
    atomic_write(&path, &buf)?;
    Ok(CacheInfo {
        path,
        container_bytes: buf.len() as u64,
        payload_bytes: payload.len() as u64,
        source_fnv: header.source_fnv,
        payload_fnv: header.payload_fnv,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use crate::registry::schema::{CommandDef, ServerDef, Transport};

    struct Sandbox {
        _dir: tempfile::TempDir,
        paths: CerberusPaths,
        source: PathBuf,
    }

    fn sandbox() -> Sandbox {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = CerberusPaths::with_roots(dir.path().join("config"), dir.path().join("data"));
        std::fs::create_dir_all(&paths.config_dir).expect("config dir");
        let source = paths.registry_file.clone();
        std::fs::write(&source, "schema_version = 2\n").expect("write source");
        Sandbox {
            _dir: dir,
            paths,
            source,
        }
    }

    fn fingerprint(source: &Path) -> u64 {
        hash_text(&std::fs::read_to_string(source).expect("read source"))
    }

    #[test]
    fn roundtrips_default_registry() {
        let sb = sandbox();
        let registry = Registry::default();
        let fnv = fingerprint(&sb.source);
        let info = write_cache(&sb.paths, &sb.source, &registry, fnv).expect("cache written");
        assert!(info.path.is_file());
        assert!(info.payload_bytes > 0);
        assert_eq!(
            try_load_cache(&sb.paths, &sb.source),
            Some((registry, fnv)),
            "cached registry must equal the original"
        );
    }

    #[test]
    fn roundtrips_servers_commands_and_string_cwd() {
        let sb = sandbox();
        let mut registry = Registry::default();
        registry.commands.insert(
            "bootstrap".into(),
            CommandDef {
                run: vec!["sh".into(), "-c".into(), "echo hi".into()],
                cwd: Some("~/work".into()),
                env: BTreeMap::new(),
                timeout_ms: Some(5000),
                description: Some("warm".into()),
            },
        );
        registry.servers.insert(
            "fs".into(),
            ServerDef {
                enabled: true,
                transport: Transport::Stdio {
                    command: "npx".into(),
                    args: vec!["-y".into(), "pkg".into()],
                    env: BTreeMap::new(),
                    cwd: Some("/tmp".into()),
                },
                mode: None,
                tags: vec!["local".into()],
                timeout_ms: None,
                startup_timeout_ms: None,
                policy: Default::default(),
                description: Some("files".into()),
                source: None,
                pre_launch: vec![],
            },
        );
        let fnv = fingerprint(&sb.source);
        write_cache(&sb.paths, &sb.source, &registry, fnv).expect("cache written");
        let (loaded, _) = try_load_cache(&sb.paths, &sb.source).expect("cache hit");
        assert_eq!(loaded, registry);
        assert_eq!(
            loaded.servers["fs"].transport,
            registry.servers["fs"].transport
        );
    }

    #[test]
    fn misses_when_source_changes() {
        let sb = sandbox();
        let registry = Registry::default();
        let fnv = fingerprint(&sb.source);
        write_cache(&sb.paths, &sb.source, &registry, fnv).expect("cache written");
        std::fs::write(&sb.source, "schema_version = 2\nactive_profile = \"x\"\n").unwrap();
        assert_eq!(try_load_cache(&sb.paths, &sb.source), None);
    }

    #[test]
    fn misses_when_only_mtime_changes() {
        let sb = sandbox();
        let registry = Registry::default();
        let fnv = fingerprint(&sb.source);
        write_cache(&sb.paths, &sb.source, &registry, fnv).expect("cache written");
        // Same bytes, different stat: `touch` must invalidate (the header
        // records the mtime the cache was compiled from).
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&sb.source)
            .unwrap();
        let past = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        file.set_modified(past).unwrap();
        assert_eq!(try_load_cache(&sb.paths, &sb.source), None);
    }

    #[test]
    fn misses_on_corrupt_payload_and_header() {
        let sb = sandbox();
        let registry = Registry::default();
        let fnv = fingerprint(&sb.source);
        let info = write_cache(&sb.paths, &sb.source, &registry, fnv).expect("cache written");

        // Flip a payload byte: the payload hash guard rejects it.
        let mut bytes = std::fs::read(&info.path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        std::fs::write(&info.path, &bytes).unwrap();
        assert_eq!(try_load_cache(&sb.paths, &sb.source), None);

        // Restore a valid cache, then break the magic.
        write_cache(&sb.paths, &sb.source, &registry, fnv).expect("cache rewritten");
        let mut bytes = std::fs::read(&info.path).unwrap();
        bytes[0] = b'X';
        std::fs::write(&info.path, &bytes).unwrap();
        assert_eq!(try_load_cache(&sb.paths, &sb.source), None);
    }

    #[test]
    fn misses_on_schema_version_and_cache_rev_mismatch() {
        let sb = sandbox();
        let registry = Registry::default();
        let fnv = fingerprint(&sb.source);
        let info = write_cache(&sb.paths, &sb.source, &registry, fnv).expect("cache written");

        // Rewrite the header with a different schema version (bytes 8..12).
        let mut bytes = std::fs::read(&info.path).unwrap();
        bytes[8..12].copy_from_slice(&99u32.to_le_bytes());
        std::fs::write(&info.path, &bytes).unwrap();
        assert_eq!(try_load_cache(&sb.paths, &sb.source), None);

        // Different cache revision (bytes 6..8).
        write_cache(&sb.paths, &sb.source, &registry, fnv).expect("cache rewritten");
        let mut bytes = std::fs::read(&info.path).unwrap();
        bytes[6..8].copy_from_slice(&99u16.to_le_bytes());
        std::fs::write(&info.path, &bytes).unwrap();
        assert_eq!(try_load_cache(&sb.paths, &sb.source), None);
    }

    #[test]
    fn cache_files_are_keyed_by_source_path() {
        let sb = sandbox();
        let other = sb.paths.config_dir.join("other.cerberus");
        assert_ne!(
            cache_file_for(&sb.paths, &sb.source),
            cache_file_for(&sb.paths, &other),
            "distinct source files must not share a cache file"
        );
    }

    #[test]
    fn invalid_registry_is_never_cached() {
        let sb = sandbox();
        let registry = Registry {
            schema_version: 999, // unknown version must fail validation
            ..Registry::default()
        };
        let fnv = fingerprint(&sb.source);
        let err = compile_cache(&sb.paths, &sb.source, &registry, fnv).unwrap_err();
        assert!(matches!(
            err,
            CerberusError::UnsupportedSchemaVersion { .. }
        ));
        assert!(!cache_file_for(&sb.paths, &sb.source).exists());
    }
}

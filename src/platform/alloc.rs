//! Filesystem-aware allocated size resolution.
//!
//! Provides the [`AllocatedSizeResolver`] trait and concrete implementations
//! that select the correct `allocated_size` computation based on the detected
//! filesystem type. See the design spec for background on why `st_blocks * 512`
//! is not universally accurate.

use std::{fs::Metadata, path::Path};

use crate::types::SizeAccuracy;

/// Resolve the on-disk allocated size for a single file.
///
/// Implementations are selected once per scan based on the filesystem type
/// at the scan root. The resolver is stored in [`crate::types::ScanConfig`]
/// as `Arc<dyn AllocatedSizeResolver>` and called per-file in the walk loop.
pub(crate) trait AllocatedSizeResolver: Send + Sync + std::fmt::Debug {
    /// Compute the allocated (on-disk) size for the file at `path`.
    fn resolve(&self, path: &Path, metadata: &Metadata) -> u64;

    /// The accuracy level of values returned by [`Self::resolve`].
    fn accuracy(&self) -> SizeAccuracy;
}

/// Select the appropriate resolver for the given filesystem type.
///
/// Returns [`LogicalOnlyResolver`] for filesystems where `st_blocks` reports
/// uncompressed logical blocks, and [`PosixResolver`] for everything else.
pub(crate) fn select_resolver(fs_type: &str) -> Box<dyn AllocatedSizeResolver> {
    match fs_type {
        "btrfs" | "bcachefs" | "f2fs" => Box::new(LogicalOnlyResolver),
        _ => Box::new(PosixResolver),
    }
}

/// Resolver for filesystems where `st_blocks * 512` is ground truth.
///
/// Used for: ext4, XFS, ZFS, NTFS3, tmpfs, vfat, NFS, and unknown types.
#[derive(Debug)]
pub(crate) struct PosixResolver;

impl AllocatedSizeResolver for PosixResolver {
    fn resolve(&self, _path: &Path, metadata: &Metadata) -> u64 {
        use std::os::unix::fs::MetadataExt as _;
        metadata.blocks().saturating_mul(512)
    }

    fn accuracy(&self) -> SizeAccuracy {
        SizeAccuracy::Exact
    }
}

/// Resolver for filesystems where `st_blocks` reports logical (uncompressed) blocks.
///
/// The computation is identical to [`PosixResolver`] — what differs is the
/// [`SizeAccuracy`] tag, which tells downstream consumers (UI, export) that the
/// value may significantly overstate actual on-disk usage.
///
/// Used for: btrfs, bcachefs, f2fs.
#[derive(Debug)]
pub(crate) struct LogicalOnlyResolver;

impl AllocatedSizeResolver for LogicalOnlyResolver {
    fn resolve(&self, _path: &Path, metadata: &Metadata) -> u64 {
        use std::os::unix::fs::MetadataExt as _;
        metadata.blocks().saturating_mul(512)
    }

    fn accuracy(&self) -> SizeAccuracy {
        SizeAccuracy::Logical
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::MetadataExt as _;

    use super::*;

    #[test]
    fn select_resolver_btrfs_returns_logical() {
        assert_eq!(select_resolver("btrfs").accuracy(), SizeAccuracy::Logical);
    }

    #[test]
    fn select_resolver_bcachefs_returns_logical() {
        assert_eq!(
            select_resolver("bcachefs").accuracy(),
            SizeAccuracy::Logical
        );
    }

    #[test]
    fn select_resolver_f2fs_returns_logical() {
        assert_eq!(select_resolver("f2fs").accuracy(), SizeAccuracy::Logical);
    }

    #[test]
    fn select_resolver_ext4_returns_exact() {
        assert_eq!(select_resolver("ext4").accuracy(), SizeAccuracy::Exact);
    }

    #[test]
    fn select_resolver_unknown_returns_exact() {
        assert_eq!(select_resolver("unknown").accuracy(), SizeAccuracy::Exact);
    }

    #[test]
    fn posix_resolver_returns_blocks_times_512() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        std::fs::write(&path, vec![0u8; 4096]).unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        let expected = meta.blocks().saturating_mul(512);
        assert_eq!(PosixResolver.resolve(&path, &meta), expected);
    }

    #[test]
    fn logical_only_resolver_returns_blocks_times_512() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        std::fs::write(&path, vec![0u8; 4096]).unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        let expected = meta.blocks().saturating_mul(512);
        assert_eq!(LogicalOnlyResolver.resolve(&path, &meta), expected);
    }
}

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
/// On Linux with a btrfs scan root, attempts to create a
/// [`BtrfsTreeSearchResolver`] that reads accurate compressed sizes via
/// `TREE_SEARCH`. Falls back to [`LogicalOnlyResolver`] if the ioctl is
/// unavailable (e.g. not running as root).
pub(crate) fn select_resolver(fs_type: &str, root: &Path) -> Box<dyn AllocatedSizeResolver> {
    match fs_type {
        "btrfs" => select_btrfs_resolver(root),
        "f2fs" => Box::new(LogicalOnlyResolver),
        _ => Box::new(PosixResolver),
    }
}

#[cfg(target_os = "linux")]
fn select_btrfs_resolver(root: &Path) -> Box<dyn AllocatedSizeResolver> {
    Box::new(BtrfsTreeSearchResolver::new(root))
}

#[cfg(not(target_os = "linux"))]
fn select_btrfs_resolver(_root: &Path) -> Box<dyn AllocatedSizeResolver> {
    Box::new(LogicalOnlyResolver)
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
/// Used for: btrfs, f2fs.
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

/// Resolver that reads accurate compressed on-disk sizes from btrfs via
/// `BTRFS_IOC_TREE_SEARCH`.
///
/// Requires `CAP_SYS_ADMIN`. On construction, probes whether the ioctl is
/// available on the scan root. If unavailable, falls back to `st_blocks * 512`
/// with [`SizeAccuracy::Logical`].
#[cfg(target_os = "linux")]
#[derive(Debug)]
pub(crate) struct BtrfsTreeSearchResolver {
    available: bool,
}

#[cfg(target_os = "linux")]
impl BtrfsTreeSearchResolver {
    pub(crate) fn new(root: &Path) -> Self {
        let available = std::fs::File::open(root).is_ok_and(|f| {
            use std::os::fd::AsFd as _;
            super::btrfs_ioctl::probe_tree_search(f.as_fd())
        });
        Self { available }
    }
}

#[cfg(target_os = "linux")]
impl AllocatedSizeResolver for BtrfsTreeSearchResolver {
    fn resolve(&self, path: &Path, metadata: &Metadata) -> u64 {
        use std::os::unix::fs::MetadataExt as _;

        if !self.available {
            return metadata.blocks().saturating_mul(512);
        }
        std::fs::File::open(path)
            .and_then(|f| {
                use std::os::fd::AsFd as _;
                super::btrfs_ioctl::file_disk_bytes(f.as_fd(), metadata.ino())
            })
            .unwrap_or_else(|_| metadata.blocks().saturating_mul(512))
    }

    fn accuracy(&self) -> SizeAccuracy {
        if self.available {
            SizeAccuracy::Exact
        } else {
            SizeAccuracy::Logical
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::MetadataExt as _;

    use super::*;

    fn test_root() -> std::path::PathBuf {
        std::env::temp_dir()
    }

    #[test]
    fn select_resolver_btrfs_on_non_btrfs_returns_logical() {
        let root = test_root();
        assert_eq!(
            select_resolver("btrfs", &root).accuracy(),
            SizeAccuracy::Logical
        );
    }

    #[test]
    fn select_resolver_f2fs_returns_logical() {
        let root = test_root();
        assert_eq!(
            select_resolver("f2fs", &root).accuracy(),
            SizeAccuracy::Logical
        );
    }

    #[test]
    fn select_resolver_ext4_returns_exact() {
        let root = test_root();
        assert_eq!(
            select_resolver("ext4", &root).accuracy(),
            SizeAccuracy::Exact
        );
    }

    #[test]
    fn select_resolver_unknown_returns_exact() {
        let root = test_root();
        assert_eq!(
            select_resolver("unknown", &root).accuracy(),
            SizeAccuracy::Exact
        );
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

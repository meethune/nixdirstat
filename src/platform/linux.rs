//! Linux-specific filesystem type detection via `statfs(2)`.

use std::path::Path;

use nix::sys::statfs::{FsType, statfs};

// Filesystem magic numbers from linux/magic.h and OpenZFS source.
// Defined locally to avoid dependency on libc platform-conditional constants.
const EXT4: FsType = FsType(0xEF53);
const XFS: FsType = FsType(0x5846_5342);
const BTRFS: FsType = FsType(0x9123_683E);
/// `OpenZFS` on Linux (`ZFS_SUPER_MAGIC` from the `OpenZFS` source tree).
const ZFS: FsType = FsType(0x2FC1_2FC1);
const TMPFS: FsType = FsType(0x0102_1994);
const NFS: FsType = FsType(0x6969);
/// MS-DOS / VFAT (`MSDOS_SUPER_MAGIC`).
const VFAT: FsType = FsType(0x4D44);
const F2FS: FsType = FsType(0xF2F5_2010);
const OVERLAYFS: FsType = FsType(0x794C_7630);

/// Detect the filesystem type by matching `statfs.f_type` against known magic numbers.
///
/// Returns a lowercase name (e.g. `"ext4"`, `"btrfs"`) for recognised types, or
/// `"0x{hex}"` for unrecognised ones.
pub(super) fn detect_filesystem_type(path: &Path) -> Result<String, std::io::Error> {
    let stat = statfs(path).map_err(std::io::Error::from)?;
    let fs_type = stat.filesystem_type();
    let name = match fs_type {
        EXT4 => "ext4",
        XFS => "xfs",
        BTRFS => "btrfs",
        ZFS => "zfs",
        TMPFS => "tmpfs",
        NFS => "nfs",
        VFAT => "vfat",
        F2FS => "f2fs",
        OVERLAYFS => "overlay",
        other => return Ok(format!("0x{:x}", other.0)),
    };
    Ok(name.to_owned())
}

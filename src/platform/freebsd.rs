//! FreeBSD-specific filesystem type detection via `statfs(2)`.

use std::path::Path;

use nix::sys::statfs::statfs;

/// Detect the filesystem type by reading the `f_fstypename` field of the `statfs` struct.
///
/// Returns a lowercase string such as `"zfs"`, `"ufs"`, `"nfs"`, or `"msdosfs"`.
pub(super) fn detect_filesystem_type(path: &Path) -> Result<String, std::io::Error> {
    let stat = statfs(path).map_err(std::io::Error::from)?;
    Ok(stat.filesystem_type_name().to_owned())
}

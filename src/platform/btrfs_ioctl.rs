//! Minimal safe wrapper around `BTRFS_IOC_TREE_SEARCH` for reading file extent sizes.
//!
//! This is the sole module in the project that contains `unsafe` code. The
//! `unsafe` is confined to a single `libc::ioctl` call whose safety relies on:
//! - Struct layouts matching the kernel ABI (verified by compile-time size assertions).
//! - The file descriptor being valid (guaranteed by `BorrowedFd`).
//! - The kernel validating all other arguments and returning an error on invalid input.
//!
//! The public API (`file_disk_bytes`) is safe. Callers never interact with raw
//! pointers or kernel structs.

// The crate-level lint is `deny(unsafe_code)`. The `allow` override is on
// the `mod` declaration in `platform/mod.rs`, making this module the sole
// sanctioned exception. See the module doc comment above for the safety case.

use std::{
    io,
    os::fd::{AsRawFd as _, BorrowedFd},
};

/// `BTRFS_IOC_TREE_SEARCH` ioctl request code.
///
/// Computed as `_IOWR(0x94, 17, struct btrfs_ioctl_search_args)` which on
/// Linux/x86-64 is `(3 << 30) | (0x94 << 8) | 17 | (4096 << 16)`.
const BTRFS_IOC_TREE_SEARCH: libc::c_ulong = {
    let dir: libc::c_ulong = 3; // _IOC_READ | _IOC_WRITE
    let magic: libc::c_ulong = 0x94;
    let nr: libc::c_ulong = 17;
    let size: libc::c_ulong = SEARCH_ARGS_SIZE as libc::c_ulong;
    (dir << 30) | (magic << 8) | nr | (size << 16)
};

/// `BTRFS_EXTENT_DATA_KEY` — identifies file extent items in the B-tree.
const BTRFS_EXTENT_DATA_KEY: u32 = 108;

/// Size of the fixed search args buffer (same as kernel `BTRFS_SEARCH_ARGS_BUFSIZE`).
const SEARCH_BUF_SIZE: usize = 4096 - std::mem::size_of::<SearchKey>();
const SEARCH_ARGS_SIZE: usize = std::mem::size_of::<SearchKey>() + SEARCH_BUF_SIZE;

/// Kernel struct `btrfs_ioctl_search_key`.
#[repr(C)]
#[derive(Clone)]
struct SearchKey {
    tree_id: u64,
    min_objectid: u64,
    max_objectid: u64,
    min_offset: u64,
    max_offset: u64,
    min_transid: u64,
    max_transid: u64,
    min_type: u32,
    max_type: u32,
    nr_items: u32,
    _unused: [u32; 9],
}

/// Kernel struct `btrfs_ioctl_search_args` (key + fixed-size buffer).
#[repr(C)]
struct SearchArgs {
    key: SearchKey,
    buf: [u8; SEARCH_BUF_SIZE],
}

/// Kernel struct `btrfs_ioctl_search_header` — precedes each result item.
#[repr(C)]
struct SearchHeader {
    transid: u64,
    objectid: u64,
    offset: u64,
    r#type: u32,
    len: u32,
}

const _: () = assert!(std::mem::size_of::<SearchKey>() == 104);
const _: () = assert!(std::mem::size_of::<SearchArgs>() == 4096);
const _: () = assert!(std::mem::size_of::<SearchHeader>() == 32);

/// Offset of the `type` field within `btrfs_file_extent_item`.
const EXTENT_TYPE_OFFSET: usize = 20;
/// Offset of `disk_num_bytes` within `btrfs_file_extent_item` (for REG/PREALLOC).
const DISK_NUM_BYTES_OFFSET: usize = 29;
/// Minimum item length for a regular extent (up to and including `disk_num_bytes`).
const MIN_REG_EXTENT_LEN: usize = DISK_NUM_BYTES_OFFSET + 8;
/// `btrfs_file_extent_item.type` value for inline data.
const EXTENT_INLINE: u8 = 0;
/// `btrfs_file_extent_item.type` value for regular extents.
const EXTENT_REG: u8 = 1;
/// `btrfs_file_extent_item.type` value for preallocated extents.
const EXTENT_PREALLOC: u8 = 2;
/// Fixed-size portion of `btrfs_file_extent_item` before inline data.
const INLINE_HEADER_SIZE: usize = 21;

/// Compute the total on-disk bytes allocated for a file's extents.
///
/// Opens a tree search for all `EXTENT_DATA_KEY` items belonging to `inode`
/// on the btrfs volume accessed through `fd`. Returns the sum of:
/// - `disk_num_bytes` for regular (type 1) and preallocated (type 2) extents.
/// - `item_len - 21` for inline (type 0) extents (data embedded in the B-tree node).
///
/// # Errors
///
/// Returns `io::Error` if the ioctl fails (e.g. `EPERM` when not root, or
/// the filesystem is not btrfs).
pub(crate) fn file_disk_bytes(fd: BorrowedFd<'_>, inode: u64) -> io::Result<u64> {
    let mut total: u64 = 0;
    let mut min_offset: u64 = 0;

    loop {
        let mut args = SearchArgs {
            key: SearchKey {
                tree_id: 0,
                min_objectid: inode,
                max_objectid: inode,
                min_offset,
                max_offset: u64::MAX,
                min_transid: 0,
                max_transid: u64::MAX,
                min_type: BTRFS_EXTENT_DATA_KEY,
                max_type: BTRFS_EXTENT_DATA_KEY,
                nr_items: 256,
                _unused: [0; 9],
            },
            buf: [0u8; SEARCH_BUF_SIZE],
        };

        // SAFETY: `args` is a `#[repr(C)]` struct whose layout matches the
        // kernel's `btrfs_ioctl_search_args` (verified by compile-time size
        // assertions above). `fd` is a valid borrowed file descriptor. The
        // kernel validates all search parameters and writes only within the
        // `buf` field.
        let ret = unsafe {
            libc::ioctl(
                fd.as_raw_fd(),
                BTRFS_IOC_TREE_SEARCH as libc::c_ulong,
                &mut args,
            )
        };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }

        let nr = args.key.nr_items;
        if nr == 0 {
            break;
        }

        let mut pos: usize = 0;
        for _ in 0..nr {
            let hdr_size = std::mem::size_of::<SearchHeader>();
            if pos + hdr_size > SEARCH_BUF_SIZE {
                break;
            }
            let hdr = parse_search_header(&args.buf[pos..pos + hdr_size]);
            pos += hdr_size;

            let item_end = pos + hdr.len as usize;
            if item_end > SEARCH_BUF_SIZE {
                break;
            }

            if hdr.r#type == BTRFS_EXTENT_DATA_KEY {
                let item = &args.buf[pos..item_end];
                total += parse_extent_disk_bytes(item, hdr.len);
            }

            min_offset = hdr.offset.wrapping_add(1);
            pos = item_end;
        }

        if min_offset == 0 {
            break;
        }
    }

    Ok(total)
}

/// Parse a `SearchHeader` from a byte slice without alignment requirements.
fn parse_search_header(bytes: &[u8]) -> SearchHeader {
    SearchHeader {
        transid: u64::from_le_bytes(bytes[0..8].try_into().unwrap_or([0; 8])),
        objectid: u64::from_le_bytes(bytes[8..16].try_into().unwrap_or([0; 8])),
        offset: u64::from_le_bytes(bytes[16..24].try_into().unwrap_or([0; 8])),
        r#type: u32::from_le_bytes(bytes[24..28].try_into().unwrap_or([0; 4])),
        len: u32::from_le_bytes(bytes[28..32].try_into().unwrap_or([0; 4])),
    }
}

/// Parse a `btrfs_file_extent_item` and return its on-disk byte count.
fn parse_extent_disk_bytes(item: &[u8], item_len: u32) -> u64 {
    if item.len() < INLINE_HEADER_SIZE {
        return 0;
    }
    let extent_type = item[EXTENT_TYPE_OFFSET];
    match extent_type {
        EXTENT_INLINE => u64::from(item_len).saturating_sub(INLINE_HEADER_SIZE as u64),
        EXTENT_REG | EXTENT_PREALLOC => {
            if item.len() < MIN_REG_EXTENT_LEN {
                return 0;
            }
            u64::from_le_bytes(
                item[DISK_NUM_BYTES_OFFSET..DISK_NUM_BYTES_OFFSET + 8]
                    .try_into()
                    .unwrap_or([0; 8]),
            )
        },
        _ => 0,
    }
}

/// Probe whether the tree search ioctl is available on the given fd.
///
/// Attempts a minimal search (inode 2, root directory). Returns `true`
/// if the ioctl succeeds, `false` if it fails (typically `EPERM`).
pub(crate) fn probe_tree_search(fd: BorrowedFd<'_>) -> bool {
    file_disk_bytes(fd, 2).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_key_size_matches_kernel() {
        assert_eq!(std::mem::size_of::<SearchKey>(), 104);
    }

    #[test]
    fn search_args_size_matches_kernel() {
        assert_eq!(std::mem::size_of::<SearchArgs>(), 4096);
    }

    #[test]
    fn search_header_size_matches_kernel() {
        assert_eq!(std::mem::size_of::<SearchHeader>(), 32);
    }

    #[test]
    fn parse_inline_extent() {
        let mut item = vec![0u8; 50];
        item[EXTENT_TYPE_OFFSET] = EXTENT_INLINE;
        assert_eq!(parse_extent_disk_bytes(&item, 50), 50 - 21);
    }

    #[test]
    fn parse_regular_extent() {
        let mut item = vec![0u8; 53];
        item[EXTENT_TYPE_OFFSET] = EXTENT_REG;
        let disk_bytes: u64 = 8192;
        item[DISK_NUM_BYTES_OFFSET..DISK_NUM_BYTES_OFFSET + 8]
            .copy_from_slice(&disk_bytes.to_le_bytes());
        assert_eq!(parse_extent_disk_bytes(&item, 53), 8192);
    }

    #[test]
    fn parse_truncated_item_returns_zero() {
        let item = vec![0u8; 10];
        assert_eq!(parse_extent_disk_bytes(&item, 10), 0);
    }

    #[test]
    fn parse_unknown_type_returns_zero() {
        let mut item = vec![0u8; 53];
        item[EXTENT_TYPE_OFFSET] = 99;
        assert_eq!(parse_extent_disk_bytes(&item, 53), 0);
    }

    #[test]
    fn probe_fails_on_non_btrfs() {
        let tmpdir = tempfile::tempdir().unwrap();
        let file = std::fs::File::open(tmpdir.path()).unwrap();
        let fd = std::os::fd::AsFd::as_fd(&file);
        assert!(!probe_tree_search(fd));
    }
}

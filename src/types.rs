//! Core domain types for NixDirStat.
//!
//! This module defines the data structures that flow through all subsystems:
//! - [`FileType`] / [`FileCategory`]: file classification
//! - [`FileEntry`]: per-file metadata collected during a scan
//! - [`ScanConfig`] / [`ScanConfigBuilder`]: validated scan parameters
//! - [`EntryBatch`]: non-empty batch of entries for the storage pipeline
//! - [`EntryQuery`]: query parameters for the explore UI
//! - [`ScanMetadata`] / [`ScanProgress`] / [`ScanWarning`]: scan lifecycle types
//! - [`TypeStat`] / [`DirectoryStats`] / [`SpaceInfo`]: aggregation types
//! - [`JournalMode`]: `SQLite` WAL strategy

use std::{
    ffi::OsStr,
    fmt,
    path::{Path, PathBuf},
    time::SystemTime,
};

use serde::Serialize;

use crate::error::ScanError;

// ---------------------------------------------------------------------------
// FileType
// ---------------------------------------------------------------------------

/// Structural type of a filesystem entry, derived from mode bits.
///
/// Determined by masking `st_mode` with `S_IFMT` and matching the result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[non_exhaustive]
pub enum FileType {
    /// A regular file.
    Regular,
    /// A directory.
    Directory,
    /// A symbolic link.
    Symlink,
    /// A block or character device file.
    Device,
    /// A Unix domain socket.
    Socket,
    /// A named pipe (FIFO).
    Pipe,
    /// An unrecognised entry type.
    Unknown,
}

impl FileType {
    /// Return the stable integer discriminant used for database storage.
    ///
    /// The mapping is: `Regular`=0, `Directory`=1, `Symlink`=2, `Device`=3,
    /// `Socket`=4, `Pipe`=5, `Unknown`=6. This mapping is stable across
    /// versions and must not change once written to a database.
    pub const fn as_discriminant(self) -> u32 {
        match self {
            Self::Regular => 0,
            Self::Directory => 1,
            Self::Symlink => 2,
            Self::Device => 3,
            Self::Socket => 4,
            Self::Pipe => 5,
            Self::Unknown => 6,
        }
    }

    /// Reconstruct a `FileType` from its database discriminant.
    ///
    /// Returns `None` for unrecognised discriminant values.
    pub const fn from_discriminant(discriminant: u32) -> Option<Self> {
        match discriminant {
            0 => Some(Self::Regular),
            1 => Some(Self::Directory),
            2 => Some(Self::Symlink),
            3 => Some(Self::Device),
            4 => Some(Self::Socket),
            5 => Some(Self::Pipe),
            6 => Some(Self::Unknown),
            _ => None,
        }
    }

    /// Derive the file type from a raw `st_mode` value.
    ///
    /// The permission bits are masked out; only the file-type bits (`S_IFMT`)
    /// are used. The cast through [`libc::mode_t`] handles the platform
    /// difference between `u32` (Linux) and `u16` (FreeBSD / macOS).
    pub const fn from_mode(mode: u32) -> Self {
        // Cast through libc::mode_t to handle the u16/u32 platform difference.
        // cast_possible_truncation: on macOS/FreeBSD mode_t is u16 so u32→u16 may truncate,
        //   but S_IF* constants always fit in u16.
        // unnecessary_cast: on Linux mode_t is u32, so u32→u32 appears redundant,
        //   but the cast is required for cross-platform portability.
        #[allow(clippy::cast_possible_truncation, clippy::unnecessary_cast)]
        let mode_t = mode as libc::mode_t;
        let file_type_bits = mode_t & libc::S_IFMT;

        if file_type_bits == libc::S_IFREG {
            Self::Regular
        } else if file_type_bits == libc::S_IFDIR {
            Self::Directory
        } else if file_type_bits == libc::S_IFLNK {
            Self::Symlink
        } else if file_type_bits == libc::S_IFBLK || file_type_bits == libc::S_IFCHR {
            Self::Device
        } else if file_type_bits == libc::S_IFSOCK {
            Self::Socket
        } else if file_type_bits == libc::S_IFIFO {
            Self::Pipe
        } else {
            Self::Unknown
        }
    }
}

impl fmt::Display for FileType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Regular => write!(f, "File"),
            Self::Directory => write!(f, "Dir"),
            Self::Symlink => write!(f, "Symlink"),
            Self::Device => write!(f, "Device"),
            Self::Socket => write!(f, "Socket"),
            Self::Pipe => write!(f, "Pipe"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

// ---------------------------------------------------------------------------
// FileCategory
// ---------------------------------------------------------------------------

/// Content category of a file, derived from its extension.
///
/// Uses the last extension component (e.g. `file.tar.gz` → `"gz"`), matching
/// the recommendation from the specification. Multi-dot extensions and
/// non-UTF-8 extensions both fall through to [`FileCategory::Other`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[non_exhaustive]
pub enum FileCategory {
    /// Source code files (`.rs`, `.py`, `.js`, `.ts`, `.c`, `.cpp`, `.h`, etc.).
    Code,
    /// Raster and vector images (`.jpg`, `.png`, `.gif`, `.svg`, `.webp`, etc.).
    Image,
    /// Documents (`.pdf`, `.doc`, `.docx`, `.odt`, `.txt`, `.md`, etc.).
    Document,
    /// Archive and compressed files (`.zip`, `.tar`, `.gz`, `.xz`, `.zst`, etc.).
    Archive,
    /// Audio files (`.mp3`, `.flac`, `.wav`, `.ogg`, `.m4a`, etc.).
    Audio,
    /// Video files (`.mp4`, `.mkv`, `.avi`, `.mov`, `.webm`, etc.).
    Video,
    /// Compiled binaries, libraries, and object files (`.so`, `.dylib`, `.o`, `.a`).
    Binary,
    /// Files without any extension.
    NoExtension,
    /// Files with an unrecognised extension.
    Other,
}

impl fmt::Display for FileCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Code => write!(f, "Code"),
            Self::Image => write!(f, "Image"),
            Self::Document => write!(f, "Document"),
            Self::Archive => write!(f, "Archive"),
            Self::Audio => write!(f, "Audio"),
            Self::Video => write!(f, "Video"),
            Self::Binary => write!(f, "Binary"),
            Self::NoExtension => write!(f, "NoExt"),
            Self::Other => write!(f, "Other"),
        }
    }
}

impl FileCategory {
    /// Classify a file by its extension and mode bits.
    ///
    /// Uses a three-tier approach:
    /// 1. **Known extensions** — a curated list for high-confidence matches.
    /// 2. **Executable bit** — files with `mode & 0o111 != 0` and no known
    ///    extension are classified as [`FileCategory::Binary`].
    /// 3. **Extension heuristic** — unrecognised extensions are classified by
    ///    character pattern: all-alpha extensions suggest source/config
    ///    ([`FileCategory::Code`]); extensions containing digits or longer
    ///    than 8 characters suggest generated artifacts
    ///    ([`FileCategory::Binary`]).
    ///
    /// Takes `Option<&OsStr>` to match [`std::path::Path::extension`].
    pub fn classify(ext: Option<&OsStr>, mode: u32) -> Self {
        let Some(ext) = ext else {
            return if mode & 0o111 != 0 {
                Self::Binary
            } else {
                Self::NoExtension
            };
        };
        let Some(s) = ext.to_str() else {
            return Self::Other;
        };
        let lower = s.to_ascii_lowercase();
        if let Some(cat) = Self::from_known_extension(&lower) {
            return cat;
        }
        Self::heuristic_classify(&lower, mode)
    }

    /// Backward-compatible classification from extension alone.
    ///
    /// Equivalent to [`FileCategory::classify`] with `mode = 0` (no
    /// executable-bit inference).
    pub fn from_extension(ext: Option<&OsStr>) -> Self {
        Self::classify(ext, 0)
    }

    /// Match against the curated list of known extensions.
    fn from_known_extension(ext: &str) -> Option<Self> {
        let cat = match ext {
            // Code / source / config
            "rs" | "py" | "js" | "ts" | "jsx" | "tsx" | "c" | "cc" | "cpp" | "cxx" | "h" | "hh"
            | "hpp" | "hxx" | "go" | "java" | "kt" | "kts" | "cs" | "rb" | "php" | "swift"
            | "m" | "mm" | "sh" | "bash" | "zsh" | "fish" | "ps1" | "lua" | "pl" | "pm" | "r"
            | "jl" | "ex" | "exs" | "erl" | "hrl" | "hs" | "lhs" | "clj" | "cljs" | "cljc"
            | "scala" | "groovy" | "gradle" | "dart" | "nim" | "zig" | "v" | "vhd" | "vhdl"
            | "sv" | "f" | "f90" | "f95" | "for" | "html" | "htm" | "css" | "scss" | "sass"
            | "less" | "xml" | "xsl" | "xslt" | "toml" | "yaml" | "yml" | "json" | "json5"
            | "jsonc" | "ini" | "cfg" | "conf" | "env" | "sql" | "proto" | "graphql" | "gql"
            | "tf" | "tfvars" | "cmake" | "make" | "mk" | "dockerfile" | "cob" | "cbl" | "pas"
            | "asm" | "s" | "pyi" | "pxd" | "pxi" | "razor" | "mdx" | "vue" | "svelte"
            | "astro" | "wgsl" | "glsl" | "hlsl" | "metal" => Self::Code,

            // Image
            "jpg" | "jpeg" | "png" | "gif" | "bmp" | "tiff" | "tif" | "webp" | "avif" | "heic"
            | "heif" | "svg" | "ico" | "psd" | "xcf" | "raw" | "cr2" | "nef" | "orf" | "arw"
            | "dng" | "eps" | "ai" | "indd" => Self::Image,

            // Document
            "pdf" | "doc" | "docx" | "odt" | "rtf" | "txt" | "text" | "md" | "markdown" | "rst"
            | "adoc" | "asciidoc" | "tex" | "latex" | "xls" | "xlsx" | "ods" | "csv" | "tsv"
            | "ppt" | "pptx" | "odp" | "epub" | "mobi" | "djvu" | "man" => Self::Document,

            // Archive
            "zip" | "tar" | "gz" | "tgz" | "bz2" | "tbz2" | "xz" | "txz" | "zst" | "tzst"
            | "lz4" | "lzma" | "lzo" | "7z" | "rar" | "ar" | "cpio" | "deb" | "rpm" | "apk"
            | "jar" | "war" | "ear" | "whl" | "egg" | "nupkg" | "snap" | "flatpak" => Self::Archive,

            // Audio
            "mp3" | "flac" | "wav" | "aiff" | "aif" | "ogg" | "oga" | "opus" | "m4a" | "aac"
            | "wma" | "mid" | "midi" | "mka" | "amr" | "ra" | "au" => Self::Audio,

            // Video
            "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" | "webm" | "m4v" | "mpg" | "mpeg"
            | "3gp" | "3g2" | "ogv" | "m2ts" | "mts" | "vob" | "rm" | "rmvb" | "asf" | "divx"
            | "xvid" => Self::Video,

            // Binary / compiled
            "so" | "dylib" | "dll" | "o" | "a" | "lib" | "ko" | "out" | "exe" | "bin" | "elf"
            | "pyc" | "pyd" | "class" | "wasm" | "pdb" | "rlib" | "rmeta" | "d" | "db"
            | "sqlite" | "sqlite3" | "dat" | "idx" | "pak" | "bundle" | "node" | "woff"
            | "woff2" | "ttf" | "otf" | "eot" => Self::Binary,

            _ => return None,
        };
        Some(cat)
    }

    /// Heuristic classification for extensions not in the known list.
    ///
    /// - Executable bit set → Binary
    /// - Extension > 8 chars or contains digits → Binary (likely generated)
    /// - All-alpha extension ≤ 8 chars → Code (likely source/config)
    /// - Everything else → Other
    fn heuristic_classify(ext: &str, mode: u32) -> Self {
        if mode & 0o111 != 0 {
            return Self::Binary;
        }
        if ext.len() > 8 || ext.bytes().any(|b| b.is_ascii_digit()) {
            return Self::Binary;
        }
        if ext.bytes().all(|b| b.is_ascii_alphabetic()) {
            return Self::Code;
        }
        Self::Other
    }
}

// ---------------------------------------------------------------------------
// FileEntry
// ---------------------------------------------------------------------------

/// Metadata collected for a single filesystem entry during a scan.
///
/// All fields are public so that later pipeline stages (e.g. hardlink dedup)
/// can mutate `allocated_size` after construction.
#[derive(Debug, Clone, Serialize)]
pub struct FileEntry {
    /// Absolute path of the entry.
    pub path: PathBuf,
    /// Logical file size in bytes (`st_size`).
    pub size: u64,
    /// Physical (allocated) size in bytes (`st_blocks * 512`).
    pub allocated_size: u64,
    /// Structural file type derived from mode bits.
    pub file_type: FileType,
    /// Content category derived from the file extension.
    pub category: FileCategory,
    /// Inode number (`st_ino`).
    pub inode: u64,
    /// Device ID on which the entry resides (`st_dev`).
    pub device: u64,
    /// Hard-link count (`st_nlink`).
    pub nlink: u64,
    /// Owner user ID (`st_uid`).
    pub uid: u32,
    /// Owner group ID (`st_gid`).
    pub gid: u32,
    /// Last modification time.
    pub mtime: SystemTime,
    /// Raw mode bits (`st_mode`).
    pub mode: u32,
}

impl FileEntry {
    /// Construct a `FileEntry` from a path and its metadata.
    ///
    /// Uses [`std::os::unix::fs::MetadataExt`] for portable access to the
    /// nine standard stat fields. The extension for `category` is extracted
    /// application-side via [`Path::extension`], as the specification requires.
    pub fn from_metadata(path: PathBuf, metadata: &std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt as _;

        let mode = metadata.mode();
        let file_type = FileType::from_mode(mode);
        let category = FileCategory::classify(path.extension(), mode);
        // Physical size: st_blocks * 512 (POSIX convention, accurate on ext4/xfs/ZFS).
        let allocated_size = metadata.blocks().saturating_mul(512);

        Self {
            path,
            size: metadata.size(),
            allocated_size,
            file_type,
            category,
            inode: metadata.ino(),
            device: metadata.dev(),
            nlink: metadata.nlink(),
            uid: metadata.uid(),
            gid: metadata.gid(),
            mtime: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            mode,
        }
    }
}

// ---------------------------------------------------------------------------
// ScanConfig / ScanConfigBuilder
// ---------------------------------------------------------------------------

/// Validated configuration for a filesystem scan.
///
/// Created via [`ScanConfig::builder`] and the [`ScanConfigBuilder`] type.
/// All fields are private; access them via the getter methods.
#[derive(Debug)]
pub struct ScanConfig {
    root: PathBuf,
    cross_device: bool,
    batch_size: usize,
}

impl ScanConfig {
    /// Return a new [`ScanConfigBuilder`] with defaults.
    pub fn builder() -> ScanConfigBuilder {
        ScanConfigBuilder::default()
    }

    /// The root directory to scan.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether the scanner should cross device boundaries.
    pub const fn cross_device(&self) -> bool {
        self.cross_device
    }

    /// Number of [`FileEntry`] items per batch sent to the storage writer.
    pub const fn batch_size(&self) -> usize {
        self.batch_size
    }
}

/// Builder for [`ScanConfig`].
///
/// Validates inputs in [`ScanConfigBuilder::build`]:
/// - `root` must exist and be a directory.
/// - `batch_size` must be > 0.
#[derive(Debug)]
pub struct ScanConfigBuilder {
    root: Option<PathBuf>,
    cross_device: bool,
    batch_size: usize,
}

impl Default for ScanConfigBuilder {
    fn default() -> Self {
        Self {
            root: None,
            cross_device: false,
            batch_size: 10_000,
        }
    }
}

impl ScanConfigBuilder {
    /// Set the root directory to scan.
    #[must_use]
    pub fn root(mut self, path: impl Into<PathBuf>) -> Self {
        self.root = Some(path.into());
        self
    }

    /// Set whether cross-device mount points should be followed.
    #[must_use]
    pub const fn cross_device(mut self, value: bool) -> Self {
        self.cross_device = value;
        self
    }

    /// Set the number of entries per storage batch.
    #[must_use]
    pub const fn batch_size(mut self, size: usize) -> Self {
        self.batch_size = size;
        self
    }

    /// Validate the configuration and produce a [`ScanConfig`].
    ///
    /// # Errors
    ///
    /// - [`ScanError::RootNotFound`] if `root` does not exist.
    /// - [`ScanError::RootNotDirectory`] if `root` is not a directory.
    /// - [`ScanError::InvalidBatchSize`] if `batch_size` is zero.
    pub fn build(self) -> Result<ScanConfig, ScanError> {
        let root = self.root.unwrap_or_default();

        if !root.exists() {
            return Err(ScanError::RootNotFound(root));
        }
        if !root.is_dir() {
            return Err(ScanError::RootNotDirectory(root));
        }
        if self.batch_size == 0 {
            return Err(ScanError::InvalidBatchSize);
        }

        Ok(ScanConfig {
            root,
            cross_device: self.cross_device,
            batch_size: self.batch_size,
        })
    }
}

// ---------------------------------------------------------------------------
// EntryBatch
// ---------------------------------------------------------------------------

/// A non-empty, ordered batch of [`FileEntry`] items.
///
/// The newtype invariant (non-empty) is enforced by [`EntryBatch::new`],
/// which returns `None` for an empty vector.
#[derive(Debug)]
pub struct EntryBatch(Vec<FileEntry>);

impl EntryBatch {
    /// Wrap `entries` in a batch.
    ///
    /// Returns `None` if `entries` is empty, preserving the non-empty invariant.
    pub fn new(entries: Vec<FileEntry>) -> Option<Self> {
        if entries.is_empty() {
            None
        } else {
            Some(Self(entries))
        }
    }

    /// Return a slice of the entries in this batch.
    pub fn entries(&self) -> &[FileEntry] {
        &self.0
    }

    /// Number of entries in the batch.
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    /// Always returns `false` — the non-empty invariant is guaranteed.
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Sort types
// ---------------------------------------------------------------------------

/// Field to sort file entries by in query results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SortField {
    /// Sort by physical (allocated) size.
    Size,
    /// Sort by file name.
    Name,
    /// Sort by last modification time.
    Modified,
    /// Sort by file type.
    Type,
}

/// Direction for sorting query results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SortDirection {
    /// Largest / latest / Z→A first.
    Descending,
    /// Smallest / earliest / A→Z first.
    Ascending,
}

// ---------------------------------------------------------------------------
// EntryQuery
// ---------------------------------------------------------------------------

/// Query parameters used by the explore UI to fetch file entries.
#[derive(Debug, Clone)]
pub struct EntryQuery {
    /// Field to sort results by.
    pub sort_by: SortField,
    /// Sort direction.
    pub sort_direction: SortDirection,
    /// Optional result limit.
    pub limit: Option<usize>,
    /// Restrict results to entries under this path prefix, if set.
    pub path_prefix: Option<PathBuf>,
    /// Minimum size filter (logical bytes).
    pub min_size: Option<u64>,
    /// Maximum size filter (logical bytes).
    pub max_size: Option<u64>,
    /// Restrict results to this file type, if set.
    pub file_type: Option<FileType>,
    /// Restrict results to this file category, if set.
    pub category: Option<FileCategory>,
}

impl Default for EntryQuery {
    fn default() -> Self {
        Self {
            sort_by: SortField::Size,
            sort_direction: SortDirection::Descending,
            limit: None,
            path_prefix: None,
            min_size: None,
            max_size: None,
            file_type: None,
            category: None,
        }
    }
}

// ---------------------------------------------------------------------------
// ScanMetadata
// ---------------------------------------------------------------------------

/// Summary metadata recorded at the end of a completed scan.
#[derive(Debug, Clone)]
pub struct ScanMetadata {
    /// Root directory that was scanned.
    pub root: PathBuf,
    /// Wall-clock time when the scan started.
    pub started_at: SystemTime,
    /// Wall-clock time when the scan completed.
    pub completed_at: SystemTime,
    /// Total number of entries discovered.
    pub entry_count: u64,
    /// Sum of logical sizes for all discovered entries.
    pub total_size: u64,
    /// Detected filesystem type(s) (e.g. `"ext4"`, `"apfs"`).
    pub filesystem_types: Vec<String>,
    /// Non-fatal warnings collected during the scan (e.g. permission errors).
    pub warnings: Vec<ScanWarning>,
}

// ---------------------------------------------------------------------------
// ScanProgress
// ---------------------------------------------------------------------------

/// A snapshot of scanner progress, sent periodically to the TUI.
#[derive(Debug, Clone)]
pub struct ScanProgress {
    /// Number of entries discovered so far.
    pub entries_scanned: u64,
    /// Approximate scanning rate in entries per second.
    pub entries_per_second: f64,
    /// Path of the entry most recently processed.
    pub current_path: PathBuf,
    /// Elapsed time since the scan started (in seconds).
    pub elapsed_secs: f64,
}

// ---------------------------------------------------------------------------
// ScanWarning
// ---------------------------------------------------------------------------

/// A non-fatal warning emitted during a scan.
///
/// Warnings are collected and displayed to the user after the scan completes.
#[derive(Debug, Clone)]
pub struct ScanWarning {
    /// Path associated with the warning.
    pub path: PathBuf,
    /// Human-readable description of what went wrong.
    pub message: String,
}

// ---------------------------------------------------------------------------
// TypeStat
// ---------------------------------------------------------------------------

/// Aggregated statistics for a single [`FileCategory`] within a scan.
#[derive(Debug, Clone)]
pub struct TypeStat {
    /// The file category this stat applies to.
    pub category: FileCategory,
    /// Number of entries in this category.
    pub count: u64,
    /// Total logical size of all entries in this category.
    pub total_size: u64,
    /// Total physical (allocated) size of all entries in this category.
    pub total_allocated: u64,
}

// ---------------------------------------------------------------------------
// DirectoryStats
// ---------------------------------------------------------------------------

/// Aggregated size statistics for a directory node.
#[derive(Debug, Clone)]
pub struct DirectoryStats {
    /// Path of the directory.
    pub path: PathBuf,
    /// Recursive total of logical sizes for all descendants.
    pub total_size: u64,
    /// Recursive total of physical (allocated) sizes for all descendants.
    pub total_allocated: u64,
    /// Direct child count (files + subdirectories, non-recursive).
    pub child_count: u64,
}

// ---------------------------------------------------------------------------
// SpaceInfo
// ---------------------------------------------------------------------------

/// Free and unknown disk space at the scan root.
///
/// Derived from `statvfs` at the completion of a scan.
#[derive(Debug, Clone)]
pub struct SpaceInfo {
    /// Total filesystem capacity in bytes.
    pub total_bytes: u64,
    /// Free bytes available to non-root processes (`f_bavail * f_bsize`).
    pub free_bytes: u64,
    /// Space not accounted for by scanned entries or free space.
    pub unknown_bytes: u64,
}

// ---------------------------------------------------------------------------
// JournalMode
// ---------------------------------------------------------------------------

/// `SQLite` journal mode selection strategy for the storage writer.
///
/// The appropriate mode is chosen at runtime based on the underlying filesystem
/// (WAL has 2.15× overhead on ZFS due to double-journaling; see specification).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum JournalMode {
    /// Write-Ahead Logging — enables concurrent readers during writes (TUI mode).
    Wal,
    /// Classic rollback journal — lower overhead on CoW filesystems like ZFS.
    Delete,
}

// ---------------------------------------------------------------------------
// format_size
// ---------------------------------------------------------------------------

/// Format a byte count as a human-readable string using binary units.
///
/// Uses one decimal place for non-exact values, e.g. `"1.5 KiB"`, `"3.0 GiB"`.
/// Exact multiples are shown with one decimal place to maintain consistent width.
/// Values smaller than 1 `KiB` are shown as `"N B"` with no decimal.
///
/// # Examples
///
/// ```
/// use nixdirstat::types::format_size;
/// assert_eq!(format_size(0), "0 B");
/// assert_eq!(format_size(500), "500 B");
/// assert_eq!(format_size(1024), "1.0 KiB");
/// assert_eq!(format_size(1536), "1.5 KiB");
/// assert_eq!(format_size(1_073_741_824), "1.0 GiB");
/// ```
// cast_precision_loss: u64→f64 conversions here are intentional and safe.
// At the TiB tier the maximum representable value is u64::MAX / TIB ≈ 2^24,
// which is well within f64's 53-bit mantissa. One-decimal-place display
// accuracy is unaffected by the precision loss.
#[allow(clippy::cast_precision_loss)]
pub fn format_size(bytes: u64) -> String {
    const KIB: u64 = 1_024;
    const MIB: u64 = 1_024 * KIB;
    const GIB: u64 = 1_024 * MIB;
    const TIB: u64 = 1_024 * GIB;

    if bytes < KIB {
        format!("{bytes} B")
    } else if bytes < MIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else if bytes < GIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes < TIB {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    } else {
        format!("{:.1} TiB", bytes as f64 / TIB as f64)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Convert a `libc::mode_t` constant to `u32` for `from_mode` test calls.
    ///
    /// On Linux `mode_t` is already `u32` (the cast is a no-op); on
    /// macOS/FreeBSD it is `u16` (widening). Either platform fires a clippy
    /// lint for the "wrong" direction, so we centralise the suppression here
    /// with an explanatory comment rather than scattering `#[allow]` attributes
    /// across every test.
    // unnecessary_cast: on Linux mode_t == u32, making `m as u32` a no-op;
    // the cast is still needed for cross-platform portability on macOS/FreeBSD.
    #[allow(clippy::unnecessary_cast)]
    fn as_mode_u32(m: libc::mode_t) -> u32 {
        m as u32
    }

    // --- FileType::from_mode ---

    #[test]
    fn file_type_from_regular_mode() {
        assert_eq!(
            FileType::from_mode(as_mode_u32(libc::S_IFREG)),
            FileType::Regular
        );
    }

    #[test]
    fn file_type_from_directory_mode() {
        assert_eq!(
            FileType::from_mode(as_mode_u32(libc::S_IFDIR)),
            FileType::Directory
        );
    }

    #[test]
    fn file_type_from_symlink_mode() {
        assert_eq!(
            FileType::from_mode(as_mode_u32(libc::S_IFLNK)),
            FileType::Symlink
        );
    }

    #[test]
    fn file_type_from_mode_with_permission_bits() {
        // Permission bits (0o755) must not affect the type result.
        let mode = as_mode_u32(libc::S_IFREG | 0o755);
        assert_eq!(FileType::from_mode(mode), FileType::Regular);
    }

    // --- FileCategory::from_extension ---

    #[test]
    fn file_category_code() {
        for ext in &["rs", "py", "js", "c", "h"] {
            assert_eq!(
                FileCategory::from_extension(Some(OsStr::new(ext))),
                FileCategory::Code,
                "expected Code for extension {ext:?}"
            );
        }
    }

    #[test]
    fn file_category_image() {
        for ext in &["jpg", "png", "gif", "svg"] {
            assert_eq!(
                FileCategory::from_extension(Some(OsStr::new(ext))),
                FileCategory::Image,
                "expected Image for extension {ext:?}"
            );
        }
    }

    #[test]
    fn file_category_archive() {
        for ext in &["gz", "zip", "tar", "xz"] {
            assert_eq!(
                FileCategory::from_extension(Some(OsStr::new(ext))),
                FileCategory::Archive,
                "expected Archive for extension {ext:?}"
            );
        }
    }

    #[test]
    fn file_category_no_extension() {
        assert_eq!(
            FileCategory::from_extension(None),
            FileCategory::NoExtension
        );
    }

    #[test]
    fn file_category_digits_in_extension_is_binary() {
        assert_eq!(
            FileCategory::from_extension(Some(OsStr::new("xyz123"))),
            FileCategory::Binary,
            "extensions with digits are heuristically classified as binary (generated artifacts)"
        );
    }

    #[test]
    fn file_category_unknown_alpha_extension_is_code() {
        assert_eq!(
            FileCategory::from_extension(Some(OsStr::new("razor"))),
            FileCategory::Code,
            "all-alpha unrecognised extensions are heuristically classified as code"
        );
    }

    #[test]
    fn file_category_executable_no_extension_is_binary() {
        assert_eq!(
            FileCategory::classify(None, 0o755),
            FileCategory::Binary,
            "extensionless files with executable bit should be binary"
        );
    }

    #[test]
    fn file_category_nonexecutable_no_extension() {
        assert_eq!(
            FileCategory::classify(None, 0o644),
            FileCategory::NoExtension,
            "extensionless files without executable bit stay NoExtension"
        );
    }

    // --- ScanConfig builder ---

    #[test]
    fn scan_config_rejects_nonexistent_root() {
        let result = ScanConfig::builder()
            .root("/no/such/path/__nixdirstat")
            .build();
        assert!(
            matches!(result, Err(ScanError::RootNotFound(_))),
            "expected RootNotFound, got {result:?}"
        );
    }

    #[test]
    fn scan_config_rejects_file_as_root() {
        let file = tempfile::NamedTempFile::new().expect("tempfile");
        let result = ScanConfig::builder().root(file.path()).build();
        assert!(
            matches!(result, Err(ScanError::RootNotDirectory(_))),
            "expected RootNotDirectory, got {result:?}"
        );
    }

    #[test]
    fn scan_config_accepts_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let result = ScanConfig::builder().root(dir.path()).build();
        assert!(result.is_ok(), "expected Ok, got {result:?}");
    }

    #[test]
    fn scan_config_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = ScanConfig::builder()
            .root(dir.path())
            .build()
            .expect("build");
        assert!(
            !config.cross_device(),
            "cross_device should default to false"
        );
        assert_eq!(
            config.batch_size(),
            10_000,
            "batch_size should default to 10_000"
        );
    }

    #[test]
    fn scan_config_rejects_zero_batch_size() {
        let dir = tempfile::tempdir().expect("tempdir");
        let result = ScanConfig::builder().root(dir.path()).batch_size(0).build();
        assert!(
            matches!(result, Err(ScanError::InvalidBatchSize)),
            "expected InvalidBatchSize, got {result:?}"
        );
    }

    // --- EntryBatch ---

    #[test]
    fn entry_batch_rejects_empty() {
        assert!(EntryBatch::new(vec![]).is_none());
    }

    #[test]
    fn entry_batch_accepts_nonempty() {
        use std::fs;

        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("test.txt");
        fs::write(&file_path, b"hello").expect("write");
        let meta = fs::metadata(&file_path).expect("metadata");
        let entry = FileEntry::from_metadata(file_path, &meta);

        let batch = EntryBatch::new(vec![entry]);
        assert!(batch.is_some());
        let batch = batch.expect("Some");
        assert_eq!(batch.len(), 1);
        assert!(!batch.is_empty());
    }

    // --- format_size ---

    #[test]
    fn format_size_bytes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(500), "500 B");
        assert_eq!(format_size(1_023), "1023 B");
    }

    #[test]
    fn format_size_kib() {
        assert_eq!(format_size(1_024), "1.0 KiB");
        assert_eq!(format_size(1_536), "1.5 KiB");
    }

    #[test]
    fn format_size_gib() {
        assert_eq!(format_size(1_073_741_824), "1.0 GiB");
    }

    #[test]
    fn format_size_tib() {
        assert_eq!(format_size(1_099_511_627_776), "1.0 TiB");
    }

    #[test]
    fn format_size_mib() {
        assert_eq!(format_size(1_048_576), "1.0 MiB");
        assert_eq!(format_size(1_572_864), "1.5 MiB");
    }
}

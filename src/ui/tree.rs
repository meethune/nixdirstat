//! In-memory directory tree built from flat [`FileEntry`] records.
//!
//! The [`DirNode`] tree is the single source of truth for both the treemap
//! and the directory tree panel. It is constructed once at explorer
//! initialization from the flat entries table and never queries the database
//! again during navigation.

use std::{collections::HashMap, path::Path, time::SystemTime};

use crate::types::{FileEntry, FileType};

/// A node in the in-memory directory tree.
///
/// Files are leaf nodes (`is_dir == false`, `children` empty).
/// Directories contain children sorted by size descending.
#[derive(Debug, Clone)]
pub struct DirNode {
    /// Filename component (not the full path).
    pub name: String,
    /// Total logical size in bytes (for directories: aggregated from the database).
    pub size: u64,
    /// Total allocated (physical) size in bytes.
    pub allocated: u64,
    /// Recursive file count (files only, not subdirectories).
    pub file_count: u64,
    /// Child nodes (files and subdirectories).
    pub children: Vec<Self>,
    /// Whether this node represents a directory.
    pub is_dir: bool,
    /// Lowercase file extension, if any. `None` for directories and extensionless files.
    pub extension: Option<String>,
    /// Last modification time.
    pub mtime: SystemTime,
}

/// Aggregated statistics for a single file extension across the tree.
#[derive(Debug, Clone)]
pub struct ExtensionStat {
    /// The extension (lowercase), or `None` for files without an extension.
    pub extension: Option<String>,
    /// Number of files with this extension.
    pub count: u64,
    /// Total logical size of all files with this extension.
    pub total_size: u64,
}

/// Build a [`DirNode`] tree from a flat list of [`FileEntry`] records.
///
/// Entries should be pre-sorted by path (lexicographic ascending). Each
/// entry's path is stripped of `root_path` to produce relative components,
/// which are inserted into the tree. Directory sizes come from the entries
/// themselves (already aggregated by the analyzer).
pub fn build_tree(entries: &[FileEntry], root_path: &Path) -> DirNode {
    let mut root = DirNode {
        name: root_path.file_name().map_or_else(
            || root_path.to_string_lossy().into_owned(),
            |n| n.to_string_lossy().into_owned(),
        ),
        size: 0,
        allocated: 0,
        file_count: 0,
        children: Vec::new(),
        is_dir: true,
        extension: None,
        mtime: SystemTime::UNIX_EPOCH,
    };

    for entry in entries {
        let Ok(rel) = entry.path().strip_prefix(root_path) else {
            continue;
        };

        let components: Vec<String> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();

        if components.is_empty() {
            root.size = entry.size();
            root.allocated = entry.allocated_size();
            root.mtime = entry.mtime();
            continue;
        }

        let is_dir = entry.file_type() == FileType::Directory;
        let extension = if is_dir {
            None
        } else {
            entry
                .path()
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
        };

        let leaf = DirNode {
            name: components.last().cloned().unwrap_or_default(),
            size: entry.size(),
            allocated: entry.allocated_size(),
            file_count: u64::from(!is_dir),
            children: Vec::new(),
            is_dir,
            extension,
            mtime: entry.mtime(),
        };

        insert_node(&mut root, &components[..components.len() - 1], leaf);
    }

    root
}

/// Insert `leaf` into the tree at the position described by `parent_components`.
///
/// Iteratively walks or creates intermediate directory nodes as needed.
fn insert_node(root: &mut DirNode, parent_components: &[String], leaf: DirNode) {
    let mut current = root;

    for component in parent_components {
        // Find or create the intermediate directory.
        let idx = current.children.iter().position(|c| c.name == *component);
        if let Some(i) = idx {
            current = &mut current.children[i];
        } else {
            current.children.push(DirNode {
                name: component.clone(),
                size: 0,
                allocated: 0,
                file_count: 0,
                children: Vec::new(),
                is_dir: true,
                extension: None,
                mtime: SystemTime::UNIX_EPOCH,
            });
            let last = current.children.len() - 1;
            current = &mut current.children[last];
        }
    }

    current.children.push(leaf);
}

/// Collect per-extension statistics by walking the tree.
///
/// Only leaf nodes (files) are counted. Returns a list sorted by
/// `total_size` descending.
pub fn collect_extension_stats(node: &DirNode) -> Vec<ExtensionStat> {
    let mut map: HashMap<Option<String>, (u64, u64)> = HashMap::new();
    collect_stats_recursive(node, &mut map);

    let mut stats: Vec<ExtensionStat> = map
        .into_iter()
        .map(|(ext, (count, total_size))| ExtensionStat {
            extension: ext,
            count,
            total_size,
        })
        .collect();

    stats.sort_by_key(|s| std::cmp::Reverse(s.total_size));
    stats
}

/// Recursive helper for [`collect_extension_stats`].
fn collect_stats_recursive(node: &DirNode, map: &mut HashMap<Option<String>, (u64, u64)>) {
    if !node.is_dir {
        let entry = map.entry(node.extension.clone()).or_insert((0, 0));
        entry.0 += 1;
        entry.1 = entry.1.saturating_add(node.size);
        return;
    }
    for child in &node.children {
        collect_stats_recursive(child, map);
    }
}

/// Navigate to a subtree by following path components from `root`.
///
/// Returns `None` if any component is not found.
pub fn find_node<'a>(root: &'a DirNode, path: &[String]) -> Option<&'a DirNode> {
    let mut current = root;
    for component in path {
        current = current.children.iter().find(|c| &c.name == component)?;
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FileCategory, FileType};

    use crate::types::FileEntryBuilder;

    fn make_entry(path: &str, size: u64, file_type: FileType) -> FileEntry {
        FileEntryBuilder::new()
            .path(path)
            .size(size)
            .file_type(file_type)
            .category(FileCategory::Other)
            .uid(1000)
            .gid(1000)
            .mode(if file_type == FileType::Directory {
                0o755
            } else {
                0o644
            })
            .build()
    }

    #[test]
    fn build_tree_flat_directory() {
        let entries = vec![
            make_entry("/root", 0, FileType::Directory),
            make_entry("/root/a.txt", 10, FileType::Regular),
            make_entry("/root/b.rs", 20, FileType::Regular),
            make_entry("/root/c.py", 30, FileType::Regular),
        ];
        let tree = build_tree(&entries, Path::new("/root"));
        assert!(tree.is_dir);
        assert_eq!(tree.children.len(), 3);
        assert!(tree.children.iter().all(|c| !c.is_dir));
    }

    #[test]
    fn build_tree_nested() {
        let entries = vec![
            make_entry("/root", 0, FileType::Directory),
            make_entry("/root/sub", 0, FileType::Directory),
            make_entry("/root/sub/file.txt", 42, FileType::Regular),
        ];
        let tree = build_tree(&entries, Path::new("/root"));
        assert_eq!(tree.children.len(), 1);
        let sub = &tree.children[0];
        assert!(sub.is_dir);
        assert_eq!(sub.name, "sub");
        assert_eq!(sub.children.len(), 1);
        assert_eq!(sub.children[0].name, "file.txt");
    }

    #[test]
    fn build_tree_sizes_from_entries() {
        let entries = vec![
            make_entry("/root", 0, FileType::Directory),
            make_entry("/root/big.bin", 100, FileType::Regular),
        ];
        let tree = build_tree(&entries, Path::new("/root"));
        assert_eq!(tree.children[0].size, 100);
    }

    #[test]
    fn build_tree_directory_sizes_aggregated() {
        let entries = vec![
            make_entry("/root", 80, FileType::Directory),
            make_entry("/root/a.txt", 50, FileType::Regular),
            make_entry("/root/b.txt", 30, FileType::Regular),
        ];
        let tree = build_tree(&entries, Path::new("/root"));
        assert_eq!(tree.size, 80);
    }

    #[test]
    fn build_tree_empty_directory() {
        let entries = vec![make_entry("/root", 0, FileType::Directory)];
        let tree = build_tree(&entries, Path::new("/root"));
        assert!(tree.children.is_empty());
        assert_eq!(tree.size, 0);
    }

    #[test]
    fn build_tree_non_utf8_filename() {
        let entry = make_entry("/root/test\u{FFFD}file.txt", 10, FileType::Regular);
        let entries = vec![make_entry("/root", 0, FileType::Directory), entry];
        let tree = build_tree(&entries, Path::new("/root"));
        assert_eq!(tree.children.len(), 1);
        assert!(tree.children[0].name.contains('\u{FFFD}'));
    }

    #[test]
    fn build_tree_extensions() {
        let entries = vec![
            make_entry("/root", 0, FileType::Directory),
            make_entry("/root/file.rs", 10, FileType::Regular),
            make_entry("/root/Makefile", 5, FileType::Regular),
        ];
        let tree = build_tree(&entries, Path::new("/root"));
        let rs_node = tree
            .children
            .iter()
            .find(|c| c.name == "file.rs")
            .expect("file.rs");
        let makefile = tree
            .children
            .iter()
            .find(|c| c.name == "Makefile")
            .expect("Makefile");
        assert_eq!(rs_node.extension, Some("rs".into()));
        assert_eq!(makefile.extension, None);
    }

    #[test]
    fn collect_stats_groups_by_extension() {
        let entries = vec![
            make_entry("/root", 0, FileType::Directory),
            make_entry("/root/a.rs", 10, FileType::Regular),
            make_entry("/root/b.rs", 20, FileType::Regular),
            make_entry("/root/c.py", 50, FileType::Regular),
        ];
        let tree = build_tree(&entries, Path::new("/root"));
        let stats = collect_extension_stats(&tree);
        let rs = stats
            .iter()
            .find(|s| s.extension.as_deref() == Some("rs"))
            .expect("rs stat");
        let py = stats
            .iter()
            .find(|s| s.extension.as_deref() == Some("py"))
            .expect("py stat");
        assert_eq!(rs.total_size, 30);
        assert_eq!(rs.count, 2);
        assert_eq!(py.total_size, 50);
        assert_eq!(py.count, 1);
    }

    #[test]
    fn collect_stats_sorted_by_size() {
        let entries = vec![
            make_entry("/root", 0, FileType::Directory),
            make_entry("/root/small.rs", 10, FileType::Regular),
            make_entry("/root/big.py", 100, FileType::Regular),
        ];
        let tree = build_tree(&entries, Path::new("/root"));
        let stats = collect_extension_stats(&tree);
        assert_eq!(stats[0].extension.as_deref(), Some("py"));
        assert_eq!(stats[1].extension.as_deref(), Some("rs"));
    }

    #[test]
    fn find_node_root() {
        let entries = vec![make_entry("/root", 0, FileType::Directory)];
        let tree = build_tree(&entries, Path::new("/root"));
        let found = find_node(&tree, &[]);
        assert!(found.is_some());
        assert_eq!(found.expect("root").name, tree.name);
    }

    #[test]
    fn find_node_nested() {
        let entries = vec![
            make_entry("/root", 0, FileType::Directory),
            make_entry("/root/sub", 0, FileType::Directory),
            make_entry("/root/sub/deep", 0, FileType::Directory),
        ];
        let tree = build_tree(&entries, Path::new("/root"));
        let found = find_node(&tree, &["sub".into(), "deep".into()]);
        assert!(found.is_some());
        assert_eq!(found.expect("deep").name, "deep");
    }

    #[test]
    fn find_node_missing() {
        let entries = vec![make_entry("/root", 0, FileType::Directory)];
        let tree = build_tree(&entries, Path::new("/root"));
        let found = find_node(&tree, &["nonexistent".into()]);
        assert!(found.is_none());
    }
}

#[cfg(test)]
#[allow(missing_docs)] // test-only fixtures; clippy --all-targets enables cfg(test)
pub mod test_fixtures {
    use super::DirNode;
    use std::time::SystemTime;

    pub fn make_file(name: &str, size: u64) -> DirNode {
        DirNode {
            name: name.to_owned(),
            size,
            allocated: size,
            file_count: 1,
            children: vec![],
            is_dir: false,
            extension: name.rsplit('.').next().map(str::to_lowercase),
            mtime: SystemTime::UNIX_EPOCH,
        }
    }

    pub fn make_dir(name: &str, children: Vec<DirNode>) -> DirNode {
        let size: u64 = children.iter().map(|c| c.size).sum();
        DirNode {
            name: name.to_owned(),
            size,
            allocated: size,
            file_count: children.iter().map(|c| c.file_count).sum(),
            children,
            is_dir: true,
            extension: None,
            mtime: SystemTime::UNIX_EPOCH,
        }
    }
}

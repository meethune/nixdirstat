//! TUI widget implementations for the explorer view.
//!
//! Each sub-module provides a single self-contained widget:
//! - [`treemap`] — recursive squarified treemap of files by size
//! - [`dir_tree`] — directory tree with expandable nodes
//! - [`extension_legend`] — scrollable extension list with color swatches

pub mod dir_tree;
pub mod extension_legend;
pub mod treemap;

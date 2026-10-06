//! TUI widget implementations for the explorer view.
//!
//! Each sub-module provides a single self-contained widget:
//! - [`treemap`] — squarified treemap of files by size
//! - [`file_table`] — sortable table of directory children (legacy, pending removal)
//! - [`type_chart`] — bar chart of size by file category (legacy, pending removal)
//! - [`extension_legend`] — scrollable extension list with color swatches
//! - [`dir_tree`] — directory tree widget

pub mod extension_legend;
pub mod file_table;
pub mod treemap;
pub mod type_chart;

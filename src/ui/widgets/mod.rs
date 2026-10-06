//! TUI widget implementations for the explorer view.
//!
//! Each sub-module provides a single self-contained widget:
//! - [`treemap`] — squarified treemap of files by size
//! - [`file_table`] — sortable table of directory children
//! - [`type_chart`] — bar chart of size by file category

pub mod file_table;
pub mod treemap;
pub mod type_chart;

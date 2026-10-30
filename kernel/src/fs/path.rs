//! Path Normalization and Segment Parsing
//! Handles canonical path resolution, normalizing '.', '..', and duplicate slashes.

use alloc::vec::Vec;
use super::vfs::FsError;

pub struct Path;

impl Path {
    /// Normalize a hierarchical path into clean component segments.
    /// Examples:
    pub fn parse_segments(path: &str) -> Result<Vec<&str>, FsError> {
        let mut segments = Vec::new();

        for part in path.split('/') {
            if part.is_empty() || part == "." {
                continue;
            }
            if part == ".." {
                // Cannot escape root, but safely pop last segment if present
                segments.pop();
            } else {
                segments.push(part);
            }
        }

        Ok(segments)
    }

    /// Check if path is absolute (starts with '/').
    pub fn is_absolute(path: &str) -> bool {
        path.starts_with('/')
    }
}

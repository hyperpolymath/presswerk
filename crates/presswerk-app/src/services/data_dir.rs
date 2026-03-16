// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Platform-aware data directory resolution.

use std::path::PathBuf;

/// Return the application data directory, creating it if needed.
///
/// On desktop this uses a conventional location. On mobile the platform
/// bridge should provide the documents directory instead.
pub fn data_dir() -> PathBuf {
    let base = dirs_fallback();
    let dir = base.join("presswerk");
    std::fs::create_dir_all(&dir).ok();
    dir
}

/// Return a subdirectory inside the data dir (e.g. "documents", "temp").
pub fn data_subdir(name: &str) -> PathBuf {
    let dir = data_dir().join(name);
    std::fs::create_dir_all(&dir).ok();
    dir
}

fn dirs_fallback() -> PathBuf {
    // Try XDG data dir, then fallback to home
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(xdg);
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".local").join("share");
    }
    // Last resort
    PathBuf::from("/tmp")
}

//! Conservative cleanup for generated Roblox files.
//!
//! Roblox's content store is indexed by its own database, so this module never
//! enters `rbx-storage` or removes content/database files. It only rotates
//! known log/profile patterns and prunes empty directories outside that store.

use std::fs;
use std::path::Path;
use std::time::SystemTime;

const RETAINED_LOGS: usize = 10;
const RETAINED_MEMORY_PROFILES: usize = 10;

#[derive(Default)]
struct Report {
    logs_removed: usize,
    profiles_removed: usize,
    directories_removed: usize,
    errors: usize,
}

/// Run before Roblox starts so none of the files being considered are active.
pub(super) fn run(data_dir: &Path, cache_dir: &Path) {
    let mut report = Report::default();

    for directory in [
        data_dir.join("files/appData/logs"),
        data_dir.join("run/appData/logs"),
    ] {
        retain_newest_matching(
            &directory,
            RETAINED_LOGS,
            |name| name.ends_with(".log"),
            &mut report.logs_removed,
            &mut report.errors,
        );
    }

    for directory in [
        data_dir.join("files/appData/LocalStorage"),
        data_dir.join("run/appData/LocalStorage"),
    ] {
        retain_newest_matching(
            &directory,
            RETAINED_MEMORY_PROFILES,
            |name| name.starts_with("memProfStorage") && name.ends_with(".json"),
            &mut report.profiles_removed,
            &mut report.errors,
        );
    }

    // Walk the runtime-owned roots, but never enter an rbx-storage tree. The
    // cache's hash blobs and the data directory's database/index are Roblox's
    // responsibility and must remain a consistent pair.
    for root in [data_dir, cache_dir] {
        prune_empty_directories(
            root,
            true,
            &mut report.directories_removed,
            &mut report.errors,
        );
    }

    if report.logs_removed > 0
        || report.profiles_removed > 0
        || report.directories_removed > 0
        || report.errors > 0
    {
        eprintln!(
            "[runtime] storage cleanup: removed {} old logs, {} memory profiles, pruned {} empty directories{}",
            report.logs_removed,
            report.profiles_removed,
            report.directories_removed,
            if report.errors == 0 {
                String::new()
            } else {
                format!(", {} cleanup errors", report.errors)
            }
        );
    }
}

fn retain_newest_matching(
    directory: &Path,
    keep: usize,
    matches: impl Fn(&str) -> bool,
    removed: &mut usize,
    errors: &mut usize,
) {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(_) => {
            *errors += 1;
            return;
        }
    };
    let mut files = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else {
            *errors += 1;
            continue;
        };
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !matches(name) {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            *errors += 1;
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        let modified = match entry.metadata().and_then(|metadata| metadata.modified()) {
            Ok(modified) => modified,
            Err(_) => {
                *errors += 1;
                SystemTime::UNIX_EPOCH
            }
        };
        files.push((modified, entry.path()));
    }

    // Newest first. Ties are stable enough for retention; every candidate is
    // generated data and the policy only removes files beyond the count limit.
    files.sort_by(|left, right| right.0.cmp(&left.0));
    for (_, path) in files.into_iter().skip(keep) {
        match fs::remove_file(path) {
            Ok(()) => *removed += 1,
            Err(_) => *errors += 1,
        }
    }
}

fn prune_empty_directories(
    directory: &Path,
    is_root: bool,
    removed: &mut usize,
    errors: &mut usize,
) -> bool {
    if !is_root && is_rbx_storage(directory) {
        return false;
    }
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
        Err(_) => {
            *errors += 1;
            return false;
        }
    };

    let mut has_entries = false;
    for entry in entries {
        let Ok(entry) = entry else {
            *errors += 1;
            has_entries = true;
            continue;
        };
        let Ok(file_type) = entry.file_type() else {
            *errors += 1;
            has_entries = true;
            continue;
        };
        // Do not follow symlinks; their targets may be outside the managed tree.
        if file_type.is_dir() {
            let child_empty = prune_empty_directories(&entry.path(), false, removed, errors);
            if !child_empty {
                has_entries = true;
            }
        } else {
            has_entries = true;
        }
    }

    if has_entries {
        return false;
    }
    if is_root {
        return true;
    }
    match fs::remove_dir(directory) {
        Ok(()) => {
            *removed += 1;
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(_) => {
            *errors += 1;
            false
        }
    }
}

fn is_rbx_storage(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().starts_with("rbx-storage"))
}

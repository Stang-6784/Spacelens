//! Chooses the fastest scanning strategy for a path and runs it.

use crate::tree::Tree;
use crate::{drives, mft, walk};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

pub const CANCELLED: &str = "cancelled";

#[derive(Default)]
pub struct Progress {
    phase: Mutex<&'static str>,
    pub items: AtomicU64,
    /// Total work units if known (MFT records), else 0.
    pub total: AtomicU64,
}

impl Progress {
    pub fn set_phase(&self, p: &'static str) {
        *self.phase.lock().unwrap() = p;
    }
    pub fn phase(&self) -> &'static str {
        *self.phase.lock().unwrap()
    }
}

pub struct ScanOutput {
    pub tree: Tree,
    pub used_mft: bool,
    pub note: Option<String>,
}

/// "c:" -> "C:\", "C:\Users\" -> "C:\Users"
pub fn normalize(path: &str) -> String {
    let p = path.trim().trim_matches('"').replace('/', "\\");
    let b = p.as_bytes();
    if b.len() >= 2 && b[1] == b':' {
        let rest = p[2..].trim_end_matches('\\');
        let letter = (b[0] as char).to_ascii_uppercase();
        if rest.is_empty() {
            return format!("{letter}:\\");
        }
        return format!("{letter}:{rest}");
    }
    p.trim_end_matches('\\').to_string()
}

pub fn run(path: &str, is_admin: bool, progress: &Progress, cancel: &AtomicBool) -> Result<ScanOutput, String> {
    let path = normalize(path);
    let b = path.as_bytes();
    let mut note = None;

    if b.len() >= 3 && b[1] == b':' {
        let letter = b[0] as char;
        let volume_root = format!("{letter}:\\");
        if drives::fs_name(&volume_root).eq_ignore_ascii_case("NTFS") {
            if is_admin {
                match mft::scan(letter, progress, cancel) {
                    Ok(mut tree) => {
                        let target = if path == volume_root { Some(tree.root) } else { tree.find_path(&path) };
                        if let Some(t) = target {
                            if t != tree.root {
                                tree.reroot(t, &path);
                            }
                            progress.set_phase("Calculating sizes");
                            tree.finalize();
                            return Ok(ScanOutput { tree, used_mft: true, note: None });
                        }
                        note = Some("Folder not found in the MFT, so a folder scan was used".into());
                    }
                    Err(e) if e == CANCELLED => return Err(e),
                    Err(e) => note = Some(format!("MFT read failed ({e}); used a folder scan instead")),
                }
            } else {
                note = Some("Turn on Turbo mode (admin) to read the MFT directly — usually 10× faster".into());
            }
        }
    }

    progress.items.store(0, Ordering::Relaxed);
    progress.total.store(0, Ordering::Relaxed);
    let mut tree = walk::scan(&path, progress, cancel)?;
    progress.set_phase("Calculating sizes");
    tree.finalize();
    Ok(ScanOutput { tree, used_mft: false, note })
}

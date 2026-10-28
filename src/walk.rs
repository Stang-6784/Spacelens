//! Fallback path: multi-threaded directory walk using `FindFirstFileExW` with large fetches.
//! Works on any file system and without admin rights.

use crate::scan::{Progress, CANCELLED};
use crate::tree::{Node, Tree, NONE};
use crate::win::wide;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::Storage::FileSystem::{
    FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW, FindNextFileW,
    GetDiskFreeSpaceW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    FIND_FIRST_EX_LARGE_FETCH, WIN32_FIND_DATAW,
};

struct Entry {
    name: Box<str>,
    wide_name: Option<Vec<u16>>, // kept for directories we will descend into
    is_dir: bool,
    /// Online-only cloud file (e.g. OneDrive "files on demand"): has a size but no local data.
    cloud_only: bool,
    size: u64,
    modified: u64,
}

/// FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS
const CLOUD_ONLY: u32 = 0x1000 | 0x0004_0000 | 0x0040_0000;

fn list_dir(dir: &[u16], out: &mut Vec<Entry>) {
    let mut pattern = Vec::with_capacity(dir.len() + 3);
    pattern.extend_from_slice(dir);
    pattern.extend_from_slice(&[b'\\' as u16, b'*' as u16, 0]);
    let mut fd: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };
    let h = unsafe {
        FindFirstFileExW(
            pattern.as_ptr(),
            FindExInfoBasic,
            (&mut fd as *mut WIN32_FIND_DATAW).cast(),
            FindExSearchNameMatch,
            std::ptr::null(),
            FIND_FIRST_EX_LARGE_FETCH,
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return; // access denied, vanished, etc.
    }
    loop {
        let len = fd.cFileName.iter().position(|&c| c == 0).unwrap_or(fd.cFileName.len());
        let nm = &fd.cFileName[..len];
        if nm != [46] && nm != [46, 46] {
            let attrs = fd.dwFileAttributes;
            let is_dir = attrs & FILE_ATTRIBUTE_DIRECTORY != 0;
            // Don't follow junctions/symlinks (name-surrogate reparse tags): avoids loops and
            // double counting. Other reparse points, e.g. OneDrive folders, are real folders.
            let is_link = attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 && fd.dwReserved0 & 0x2000_0000 != 0;
            let descend = is_dir && !is_link;
            out.push(Entry {
                name: String::from_utf16_lossy(nm).into(),
                wide_name: descend.then(|| nm.to_vec()),
                is_dir,
                cloud_only: attrs & CLOUD_ONLY != 0,
                size: ((fd.nFileSizeHigh as u64) << 32) | fd.nFileSizeLow as u64,
                modified: ((fd.ftLastWriteTime.dwHighDateTime as u64) << 32)
                    | fd.ftLastWriteTime.dwLowDateTime as u64,
            });
        }
        if unsafe { FindNextFileW(h, &mut fd) } == 0 {
            break;
        }
    }
    unsafe { FindClose(h) };
}

fn cluster_size(path: &str) -> u64 {
    let root = if path.as_bytes().get(1) == Some(&b':') { format!("{}\\", &path[..2]) } else { return 4096 };
    let (mut spc, mut bps, mut free, mut total) = (0u32, 0u32, 0u32, 0u32);
    let ok = unsafe { GetDiskFreeSpaceW(wide(&root).as_ptr(), &mut spc, &mut bps, &mut free, &mut total) };
    if ok != 0 && spc > 0 && bps > 0 { spc as u64 * bps as u64 } else { 4096 }
}

/// Extended-length form of a path, without a trailing separator or terminator.
fn extended(path: &str) -> Vec<u16> {
    let p = path.trim_end_matches('\\');
    let s = if let Some(unc) = p.strip_prefix("\\\\") {
        format!("\\\\?\\UNC\\{unc}")
    } else {
        format!("\\\\?\\{p}")
    };
    s.encode_utf16().collect()
}

pub fn scan(path: &str, progress: &Progress, cancel: &AtomicBool) -> Result<Tree, String> {
    let cluster = cluster_size(path);
    let nodes = Mutex::new(vec![Node::new(path.into(), NONE, true)]);
    let queue: Mutex<Vec<(u32, Vec<u16>)>> = Mutex::new(vec![(0, extended(path))]);
    let wake = Condvar::new();
    let pending = AtomicUsize::new(1);
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 16);

    progress.set_phase("Scanning folders");
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                let mut entries = Vec::new();
                loop {
                    let job = {
                        let mut q = queue.lock().unwrap();
                        loop {
                            if cancel.load(Ordering::Relaxed) {
                                break None;
                            }
                            if let Some(j) = q.pop() {
                                break Some(j);
                            }
                            if pending.load(Ordering::SeqCst) == 0 {
                                break None;
                            }
                            q = wake.wait_timeout(q, Duration::from_millis(50)).unwrap().0;
                        }
                    };
                    let Some((dir, dir_path)) = job else {
                        wake.notify_all();
                        return;
                    };

                    entries.clear();
                    list_dir(&dir_path, &mut entries);
                    let found = entries.len() as u64;
                    let mut jobs = Vec::new();
                    {
                        let mut ns = nodes.lock().unwrap();
                        let mut kids = Vec::with_capacity(entries.len());
                        for e in entries.drain(..) {
                            let idx = ns.len() as u32;
                            let mut n = Node::new(e.name, dir, e.is_dir);
                            n.modified = e.modified;
                            if !e.is_dir {
                                n.size = e.size;
                                n.alloc = if e.cloud_only { 0 } else { e.size.div_ceil(cluster) * cluster };
                            }
                            ns.push(n);
                            kids.push(idx);
                            if let Some(w) = e.wide_name {
                                let mut child = Vec::with_capacity(dir_path.len() + 1 + w.len());
                                child.extend_from_slice(&dir_path);
                                child.push(b'\\' as u16);
                                child.extend_from_slice(&w);
                                jobs.push((idx, child));
                            }
                        }
                        ns[dir as usize].children = kids;
                    }
                    progress.items.fetch_add(found, Ordering::Relaxed);

                    let mut q = queue.lock().unwrap();
                    pending.fetch_add(jobs.len(), Ordering::SeqCst);
                    q.extend(jobs);
                    pending.fetch_sub(1, Ordering::SeqCst);
                    drop(q);
                    wake.notify_all();
                }
            });
        }
    });

    if cancel.load(Ordering::Relaxed) {
        return Err(CANCELLED.into());
    }
    Ok(Tree::new(nodes.into_inner().unwrap(), 0))
}

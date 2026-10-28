//! Fast path: read the NTFS Master File Table straight from the raw volume.
//!
//! Every file and folder on an NTFS volume has a fixed-size record in the MFT. Reading the
//! whole table sequentially in large chunks and parsing it in memory is far faster than asking
//! the OS to enumerate millions of directory entries. Requires administrator rights.

use crate::scan::{Progress, CANCELLED};
use crate::tree::{Node, Tree, NONE};
use crate::win::{last_error, wide};
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, SetFilePointerEx, FILE_BEGIN, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Ioctl::FSCTL_GET_NTFS_VOLUME_DATA;
use windows_sys::Win32::System::IO::DeviceIoControl;

const ROOT_RECORD: usize = 5;
const REF_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;

struct Volume(HANDLE);

impl Drop for Volume {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

impl Volume {
    fn open(letter: char) -> Result<Self, String> {
        let path = wide(&format!("\\\\.\\{letter}:"));
        let h = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        if h == INVALID_HANDLE_VALUE {
            Err(last_error("open volume"))
        } else {
            Ok(Volume(h))
        }
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), String> {
        unsafe {
            if SetFilePointerEx(self.0, offset as i64, std::ptr::null_mut(), FILE_BEGIN) == 0 {
                return Err(last_error("seek"));
            }
            let mut read = 0u32;
            if ReadFile(self.0, buf.as_mut_ptr(), buf.len() as u32, &mut read, std::ptr::null_mut())
                == 0
            {
                return Err(last_error("read"));
            }
            if read as usize != buf.len() {
                return Err("short read from volume".into());
            }
        }
        Ok(())
    }
}

#[inline]
fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
#[inline]
fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
#[inline]
fn u64le(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

/// Applies the update-sequence fixups that protect each sector of a record.
fn fixup(rec: &mut [u8]) -> bool {
    let rs = rec.len();
    if rs < 64 || &rec[0..4] != b"FILE" {
        return false;
    }
    let usa = u16le(rec, 4) as usize;
    let count = u16le(rec, 6) as usize;
    if count < 2 || usa + count * 2 > rs {
        return false;
    }
    let stride = rs / (count - 1);
    if stride < 2 {
        return false;
    }
    let (u0, u1) = (rec[usa], rec[usa + 1]);
    for i in 1..count {
        let p = i * stride - 2;
        if p + 2 > rs || rec[p] != u0 || rec[p + 1] != u1 {
            return false;
        }
        rec[p] = rec[usa + 2 * i];
        rec[p + 1] = rec[usa + 2 * i + 1];
    }
    true
}

/// Decodes a non-resident attribute's run list into (LCN, cluster count) pairs.
/// `None` LCN marks a sparse run.
fn decode_runs(b: &[u8], mut p: usize, end: usize) -> Vec<(Option<u64>, u64)> {
    let mut out = Vec::new();
    let mut lcn: i64 = 0;
    while p < end {
        let h = b[p];
        if h == 0 {
            break;
        }
        let len_bytes = (h & 0x0F) as usize;
        let off_bytes = (h >> 4) as usize;
        p += 1;
        if len_bytes == 0 || len_bytes > 8 || off_bytes > 8 || p + len_bytes + off_bytes > end {
            break;
        }
        let mut count = 0u64;
        for i in 0..len_bytes {
            count |= (b[p + i] as u64) << (8 * i);
        }
        p += len_bytes;
        if off_bytes == 0 {
            out.push((None, count));
        } else {
            let mut delta: i64 = 0;
            for i in 0..off_bytes {
                delta |= (b[p + i] as i64) << (8 * i);
            }
            if off_bytes < 8 && b[p + off_bytes - 1] & 0x80 != 0 {
                delta |= -1i64 << (8 * off_bytes);
            }
            lcn += delta;
            out.push((Some(lcn as u64), count));
        }
        p += off_bytes;
    }
    out
}

/// Finds the run list of the MFT's own unnamed $DATA attribute in record 0.
fn mft_runs(rec: &[u8]) -> Result<Vec<(Option<u64>, u64)>, String> {
    let used = (u32le(rec, 0x18) as usize).min(rec.len());
    let mut a = u16le(rec, 0x14) as usize;
    while a + 16 <= used {
        let ty = u32le(rec, a);
        if ty == 0xFFFF_FFFF {
            break;
        }
        let len = u32le(rec, a + 4) as usize;
        if len < 16 || a + len > used {
            break;
        }
        if ty == 0x80 && rec[a + 8] != 0 && rec[a + 9] == 0 && len >= 0x40 {
            let runs = decode_runs(rec, a + u16le(rec, a + 0x20) as usize, a + len);
            if !runs.is_empty() {
                return Ok(runs);
            }
        }
        a += len;
    }
    Err("could not locate the $MFT data runs".into())
}

/// Per-record scratch data collected while streaming the MFT.
struct Records {
    parent: Vec<u32>,
    name: Vec<Option<Box<str>>>,
    name_rank: Vec<u8>,
    size: Vec<u64>,
    alloc: Vec<u64>,
    modified: Vec<u64>,
    flags: Vec<u8>, // bit0 = in use, bit1 = directory
}

impl Records {
    fn new(n: usize) -> Self {
        Records {
            parent: vec![NONE; n],
            name: (0..n).map(|_| None).collect(),
            name_rank: vec![0; n],
            size: vec![0; n],
            alloc: vec![0; n],
            modified: vec![0; n],
            flags: vec![0; n],
        }
    }

    fn parse(&mut self, rec: &mut [u8], idx: usize) {
        if !fixup(rec) {
            return;
        }
        let rflags = u16le(rec, 0x16);
        if rflags & 1 == 0 {
            return; // record not in use
        }
        let base = (u64le(rec, 0x20) & REF_MASK) as usize;
        let n = self.flags.len();
        // Extension records describe extra attributes of their base record.
        let target = if base == 0 { idx } else { base };
        if target >= n {
            return;
        }
        if base == 0 {
            self.flags[idx] |= 1 | if rflags & 2 != 0 { 2 } else { 0 };
        }

        let used = (u32le(rec, 0x18) as usize).min(rec.len());
        let mut a = u16le(rec, 0x14) as usize;
        while a + 16 <= used {
            let ty = u32le(rec, a);
            if ty == 0xFFFF_FFFF {
                break;
            }
            let len = u32le(rec, a + 4) as usize;
            if len < 16 || a + len > used {
                break;
            }
            let end = a + len;
            let non_resident = rec[a + 8] != 0;
            let name_len = rec[a + 9];
            match ty {
                // $STANDARD_INFORMATION: last-write time
                0x10 if !non_resident && len >= 0x18 => {
                    let vo = a + u16le(rec, a + 0x14) as usize;
                    if vo + 16 <= end {
                        self.modified[target] = u64le(rec, vo + 8);
                    }
                }
                // $FILE_NAME: name + parent directory
                0x30 if !non_resident && len >= 0x18 => {
                    let vo = a + u16le(rec, a + 0x14) as usize;
                    if vo + 0x42 > end {
                        a += len;
                        continue;
                    }
                    let parent_ref = u64le(rec, vo) & REF_MASK;
                    let nl = rec[vo + 0x40] as usize;
                    // Prefer Win32 names over POSIX, and anything over 8.3 DOS aliases.
                    let rank = match rec[vo + 0x41] {
                        2 => 1,
                        0 => 2,
                        _ => 3,
                    };
                    if rank > self.name_rank[target] && vo + 0x42 + nl * 2 <= end {
                        let units: Vec<u16> = rec[vo + 0x42..vo + 0x42 + nl * 2]
                            .chunks_exact(2)
                            .map(|c| u16::from_le_bytes([c[0], c[1]]))
                            .collect();
                        self.name[target] = Some(String::from_utf16_lossy(&units).into());
                        self.name_rank[target] = rank;
                        self.parent[target] =
                            if parent_ref < NONE as u64 { parent_ref as u32 } else { NONE };
                    }
                }
                // Unnamed $DATA: file contents
                0x80 if name_len == 0 => {
                    if !non_resident {
                        if len >= 0x18 {
                            self.size[target] += u32le(rec, a + 0x10) as u64;
                        }
                    } else if len >= 0x40 && u64le(rec, a + 0x10) == 0 {
                        // Only the first extent (starting VCN 0) carries the sizes.
                        let data_size = u64le(rec, a + 0x30);
                        let mut alloc = u64le(rec, a + 0x28);
                        let aflags = u16le(rec, a + 0x0C);
                        if aflags & 0x8001 != 0 && len >= 0x48 {
                            alloc = u64le(rec, a + 0x40); // compressed / sparse: real on-disk size
                        }
                        self.size[target] += data_size;
                        self.alloc[target] += alloc;
                    }
                }
                _ => {}
            }
            a += len;
        }
    }
}

/// Reads the whole MFT of `letter:` and returns an unfinalized tree rooted at the volume.
pub fn scan(letter: char, progress: &Progress, cancel: &AtomicBool) -> Result<Tree, String> {
    let vol = Volume::open(letter)?;

    let mut vd = [0u8; 128];
    let mut returned = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            vol.0,
            FSCTL_GET_NTFS_VOLUME_DATA,
            std::ptr::null(),
            0,
            vd.as_mut_ptr().cast(),
            vd.len() as u32,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(last_error("query NTFS volume data"));
    }
    let bytes_per_sector = u32le(&vd, 40) as usize;
    let bytes_per_cluster = u32le(&vd, 44) as u64;
    let rs = u32le(&vd, 48) as usize;
    let mft_valid = u64le(&vd, 56);
    let mft_start_lcn = u64le(&vd, 64);
    if rs < 256 || bytes_per_cluster == 0 {
        return Err("unexpected NTFS geometry".into());
    }

    let mut first = vec![0u8; rs.max(bytes_per_sector)];
    vol.read_at(mft_start_lcn * bytes_per_cluster, &mut first)?;
    let mut rec0 = first[..rs].to_vec();
    if !fixup(&mut rec0) {
        return Err("$MFT record 0 is damaged".into());
    }
    let runs = mft_runs(&rec0)?;

    let n = (mft_valid / rs as u64) as usize;
    if n <= ROOT_RECORD || n >= NONE as usize {
        return Err("unexpected MFT size".into());
    }
    let mut recs = Records::new(n);
    progress.set_phase("Reading Master File Table");
    progress.total.store(n as u64, Ordering::Relaxed);

    let chunk = (((8u64 << 20) / bytes_per_cluster).max(1) * bytes_per_cluster) as usize;
    let mut buf = vec![0u8; chunk + rs];
    let mut carry = 0usize;
    let mut idx = 0usize;

    for (lcn, clusters) in runs {
        let run_bytes = clusters * bytes_per_cluster;
        let Some(lcn) = lcn else {
            idx += (run_bytes / rs as u64) as usize;
            continue;
        };
        let mut pos = 0u64;
        while pos < run_bytes && idx < n {
            if cancel.load(Ordering::Relaxed) {
                return Err(CANCELLED.into());
            }
            let to_read = (run_bytes - pos).min(chunk as u64) as usize;
            vol.read_at(lcn * bytes_per_cluster + pos, &mut buf[carry..carry + to_read])?;
            pos += to_read as u64;
            let avail = carry + to_read;
            let mut off = 0;
            while avail - off >= rs && idx < n {
                recs.parse(&mut buf[off..off + rs], idx);
                off += rs;
                idx += 1;
            }
            // A record can straddle two runs; keep the partial bytes for the next read.
            carry = avail - off;
            buf.copy_within(off..avail, 0);
            progress.items.store(idx as u64, Ordering::Relaxed);
        }
    }
    drop(buf);

    progress.set_phase("Building folder tree");
    build_tree(recs, letter)
}

fn build_tree(mut recs: Records, letter: char) -> Result<Tree, String> {
    let n = recs.flags.len();
    let mut map = vec![NONE; n];
    let mut nodes: Vec<Node> = Vec::with_capacity(n);

    let mut root = Node::new(format!("{letter}:\\").into(), NONE, true);
    root.modified = recs.modified[ROOT_RECORD];
    nodes.push(root);
    map[ROOT_RECORD] = 0;

    for i in 0..n {
        if i == ROOT_RECORD || recs.flags[i] & 1 == 0 {
            continue;
        }
        let Some(name) = recs.name[i].take() else { continue };
        map[i] = nodes.len() as u32;
        let mut node = Node::new(name, NONE, recs.flags[i] & 2 != 0);
        node.size = recs.size[i];
        node.alloc = recs.alloc[i];
        node.modified = recs.modified[i];
        nodes.push(node);
    }

    for i in 0..n {
        let ni = map[i];
        if ni == NONE || ni == 0 {
            continue;
        }
        let p = recs.parent[i] as usize;
        let mut pi = if p < n { map[p] } else { NONE };
        if pi == NONE || pi == ni || !nodes[pi as usize].is_dir {
            pi = 0; // orphaned record: attach to the volume root
        }
        nodes[ni as usize].parent = pi;
        nodes[pi as usize].children.push(ni);
    }
    Ok(Tree::new(nodes, 0))
}

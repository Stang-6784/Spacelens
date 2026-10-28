//! Arena-allocated file tree. Nodes are addressed by `u32` index for compactness and speed.

use crate::categories::{self, Category};
use std::collections::HashMap;

pub const NONE: u32 = u32::MAX;

pub struct Node {
    pub name: Box<str>,
    pub parent: u32,
    pub children: Vec<u32>,
    pub size: u64,
    pub alloc: u64,
    /// Last write time as a Windows FILETIME (0 = unknown).
    pub modified: u64,
    pub files: u32,
    pub folders: u32,
    pub is_dir: bool,
    pub cat: Category,
}

impl Node {
    pub fn new(name: Box<str>, parent: u32, is_dir: bool) -> Self {
        Node {
            name,
            parent,
            children: Vec::new(),
            size: 0,
            alloc: 0,
            modified: 0,
            files: 0,
            folders: 0,
            is_dir,
            cat: Category::Other,
        }
    }
}

pub struct ExtStat {
    pub ext: String,
    pub size: u64,
    pub count: u32,
    pub cat: Category,
}

#[derive(Default, Clone, Copy)]
pub struct CatStat {
    pub size: u64,
    pub count: u32,
}

pub struct Tree {
    pub nodes: Vec<Node>,
    pub root: u32,
    /// Extensions sorted by total size, largest first.
    pub exts: Vec<ExtStat>,
    pub cats: [CatStat; categories::COUNT],
}

impl Tree {
    pub fn new(nodes: Vec<Node>, root: u32) -> Self {
        Tree { nodes, root, exts: Vec::new(), cats: [CatStat::default(); categories::COUNT] }
    }

    #[inline]
    pub fn node(&self, i: u32) -> &Node {
        &self.nodes[i as usize]
    }

    pub fn path(&self, i: u32) -> String {
        if i == self.root {
            return self.node(i).name.to_string();
        }
        let mut parts = Vec::new();
        let mut n = i;
        while n != self.root && n != NONE {
            parts.push(&*self.node(n).name);
            n = self.node(n).parent;
        }
        let mut s = self.node(self.root).name.trim_end_matches('\\').to_string();
        for p in parts.iter().rev() {
            s.push('\\');
            s.push_str(p);
        }
        s
    }

    pub fn is_under(&self, i: u32, ancestor: u32) -> bool {
        let mut n = i;
        while n != NONE {
            if n == ancestor {
                return true;
            }
            n = self.node(n).parent;
        }
        false
    }

    /// Finds a node by absolute path below the root (case-insensitive).
    pub fn find_path(&self, path: &str) -> Option<u32> {
        let root = self.node(self.root).name.trim_end_matches('\\');
        let rest = path.get(root.len()..)?;
        if !path[..root.len()].eq_ignore_ascii_case(root) {
            return None;
        }
        let mut cur = self.root;
        for part in rest.split('\\').filter(|p| !p.is_empty()) {
            let lower = part.to_lowercase();
            cur = *self
                .node(cur)
                .children
                .iter()
                .find(|&&c| self.node(c).name.to_lowercase() == lower)?;
        }
        Some(cur)
    }

    /// Makes `i` the new root (used when a folder was located inside a full-volume MFT read).
    pub fn reroot(&mut self, i: u32, name: &str) {
        let n = &mut self.nodes[i as usize];
        n.parent = NONE;
        n.name = name.into();
        self.root = i;
    }

    /// Aggregates sizes/counts bottom-up, classifies files, sorts children by size and
    /// computes file-type statistics. Only nodes reachable from the root are visited.
    pub fn finalize(&mut self) {
        let mut stack = vec![(self.root, false)];
        while let Some((i, post)) = stack.pop() {
            if !post {
                stack.push((i, true));
                for &c in &self.nodes[i as usize].children {
                    if self.nodes[c as usize].is_dir {
                        stack.push((c, false));
                    }
                }
                continue;
            }
            let mut kids = std::mem::take(&mut self.nodes[i as usize].children);
            let (mut size, mut alloc, mut files, mut folders) = (0u64, 0u64, 0u32, 0u32);
            let mut modified = self.nodes[i as usize].modified;
            for &c in &kids {
                let n = &mut self.nodes[c as usize];
                if n.is_dir {
                    files += n.files;
                    folders += n.folders + 1;
                } else {
                    n.cat = categories::classify(&n.name);
                    files += 1;
                }
                size += n.size;
                alloc += n.alloc;
                modified = modified.max(n.modified);
            }
            let nodes = &self.nodes;
            kids.sort_unstable_by(|&a, &b| nodes[b as usize].size.cmp(&nodes[a as usize].size));
            kids.shrink_to_fit();
            let n = &mut self.nodes[i as usize];
            n.children = kids;
            n.size = size;
            n.alloc = alloc;
            n.files = files;
            n.folders = folders;
            n.modified = modified;
        }
        self.recompute_stats();
    }

    pub fn recompute_stats(&mut self) {
        let mut map: HashMap<String, ExtStat> = HashMap::new();
        let mut cats = [CatStat::default(); categories::COUNT];
        let mut stack = vec![self.root];
        while let Some(i) = stack.pop() {
            let n = &self.nodes[i as usize];
            if n.is_dir {
                stack.extend_from_slice(&n.children);
                continue;
            }
            let c = &mut cats[n.cat as usize];
            c.size += n.size;
            c.count += 1;
            let ext = categories::ext_of(&n.name).to_ascii_lowercase();
            let e = map.entry(ext).or_insert_with_key(|k| ExtStat {
                ext: k.clone(),
                size: 0,
                count: 0,
                cat: n.cat,
            });
            e.size += n.size;
            e.count += 1;
        }
        let mut exts: Vec<ExtStat> = map.into_values().collect();
        exts.sort_unstable_by(|a, b| b.size.cmp(&a.size));
        self.exts = exts;
        self.cats = cats;
    }

    /// Detaches a node that was deleted on disk and fixes up every total above it.
    pub fn remove(&mut self, i: u32) {
        let parent = self.node(i).parent;
        if parent == NONE {
            return;
        }
        let (size, alloc) = (self.node(i).size, self.node(i).alloc);
        let (files, folders) = if self.node(i).is_dir {
            (self.node(i).files, self.node(i).folders + 1)
        } else {
            (1, 0)
        };
        self.nodes[parent as usize].children.retain(|&c| c != i);
        let mut a = parent;
        while a != NONE {
            let n = &mut self.nodes[a as usize];
            n.size -= size;
            n.alloc -= alloc;
            n.files -= files;
            n.folders -= folders;
            let mut kids = std::mem::take(&mut n.children);
            let nodes = &self.nodes;
            kids.sort_unstable_by(|&x, &y| nodes[y as usize].size.cmp(&nodes[x as usize].size));
            let n = &mut self.nodes[a as usize];
            n.children = kids;
            a = n.parent;
        }
        self.nodes[i as usize].parent = NONE;
        self.recompute_stats();
    }
}

//! Parallel directory scanner. Walks the tree with rayon (work-stealing across
//! all cores), then flattens the result into a compact arena for the UI.

use rayon::prelude::*;
use std::cmp::Reverse;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Progress {
    pub files: AtomicU64,
    pub dirs: AtomicU64,
    pub bytes: AtomicU64,
    pub errors: AtomicU64,
    pub name_bytes: AtomicU64,
    pub cancel: AtomicBool,
}

/// Temporary nested result of the parallel walk.
struct TDir {
    name: Box<str>,
    size: u64,
    files: u64,
    /// All file names of this directory, concatenated.
    names: String,
    /// (size, offset into `names`, length)
    file_list: Vec<(u64, u32, u32)>,
    dirs: Vec<TDir>,
}

fn scan_dir(path: &Path, name: Box<str>, p: &Progress) -> TDir {
    let mut d = TDir { name, size: 0, files: 0, names: String::new(), file_list: Vec::new(), dirs: Vec::new() };
    if p.cancel.load(Relaxed) {
        return d;
    }
    let rd = match fs::read_dir(path) {
        Ok(r) => r,
        Err(_) => {
            p.errors.fetch_add(1, Relaxed);
            return d;
        }
    };
    let mut subdirs: Vec<(PathBuf, Box<str>)> = Vec::new();
    let mut bytes = 0u64;
    let mut name_bytes = 0u64;
    for e in rd {
        let Ok(e) = e else {
            p.errors.fetch_add(1, Relaxed);
            continue;
        };
        let Ok(ft) = e.file_type() else { continue };
        // Skip symlinks and junctions: avoids loops and double counting.
        if ft.is_symlink() {
            continue;
        }
        let os_name = e.file_name();
        let name = os_name.to_string_lossy();
        name_bytes += name.len() as u64;
        if ft.is_dir() {
            subdirs.push((e.path(), name.into()));
        } else {
            // On Windows this comes from the directory listing itself (no extra syscall).
            let len = e.metadata().map(|m| m.len()).unwrap_or(0);
            bytes += len;
            let off = d.names.len() as u32;
            d.names.push_str(&name);
            d.file_list.push((len, off, name.len() as u32));
        }
    }
    d.names.shrink_to_fit();
    d.file_list.shrink_to_fit();
    let nfiles = d.file_list.len() as u64;
    p.files.fetch_add(nfiles, Relaxed);
    p.bytes.fetch_add(bytes, Relaxed);
    p.dirs.fetch_add(1, Relaxed);
    p.name_bytes.fetch_add(name_bytes, Relaxed);

    d.dirs = subdirs.into_par_iter().map(|(pp, n)| scan_dir(&pp, n, p)).collect();
    d.size = bytes + d.dirs.iter().map(|x| x.size).sum::<u64>();
    d.files = nfiles + d.dirs.iter().map(|x| x.files).sum::<u64>();
    d
}

pub const NONE: u32 = u32::MAX;
pub const TOP_N: usize = 1000;

/// 32 bytes per entry; names live in the shared `Tree::names` pool.
pub struct Node {
    pub size: u64,
    /// Number of files contained (1 for a file).
    pub files: u32,
    pub parent: u32,
    pub child_start: u32,
    pub child_len: u32,
    name_off: u32,
    name_len: u16,
    pub is_dir: bool,
    pub removed: bool,
}

pub struct Tree {
    pub root_path: PathBuf,
    pub nodes: Vec<Node>,
    names: String,
    pub top_files: Vec<u32>,
    pub top_dirs: Vec<u32>,
    pub errors: u64,
    pub elapsed: Duration,
    pub cancelled: bool,
}

pub fn run(path: PathBuf, progress: &Progress) -> Tree {
    let t0 = Instant::now();
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    // Directory listing is I/O bound: oversubscribe a bit to keep the disk queue full.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads((threads * 2).clamp(4, 48))
        .stack_size(16 << 20)
        .build()
        .expect("thread pool");
    let root_name: Box<str> = path.display().to_string().into();
    let root = pool.install(|| scan_dir(&path, root_name, progress));
    let entries = (progress.files.load(Relaxed) + progress.dirs.load(Relaxed)) as usize;
    let name_bytes = progress.name_bytes.load(Relaxed) as usize + root.name.len();
    let mut tree = Tree::build(path, root, entries, name_bytes);
    tree.errors = progress.errors.load(Relaxed);
    tree.cancelled = progress.cancel.load(Relaxed);
    tree.elapsed = t0.elapsed();
    tree
}

enum Kid {
    D(TDir),
    F(u64, u32, u32),
}

impl Kid {
    fn size(&self) -> u64 {
        match self {
            Kid::D(d) => d.size,
            Kid::F(s, _, _) => *s,
        }
    }
}

fn intern(pool: &mut String, s: &str) -> (u32, u16) {
    let off = pool.len() as u32;
    pool.push_str(s);
    (off, s.len().min(u16::MAX as usize) as u16)
}

impl Tree {
    fn build(root_path: PathBuf, root: TDir, entries: usize, name_bytes: usize) -> Tree {
        let mut nodes: Vec<Node> = Vec::with_capacity(entries + 1);
        let mut names = String::with_capacity(name_bytes);
        let (name_off, name_len) = intern(&mut names, &root.name);
        nodes.push(Node {
            size: root.size,
            files: root.files as u32,
            parent: NONE,
            child_start: 0,
            child_len: 0,
            name_off,
            name_len,
            is_dir: true,
            removed: false,
        });
        let mut queue = VecDeque::new();
        queue.push_back((0u32, root));
        while let Some((idx, td)) = queue.pop_front() {
            let TDir { names: fnames, file_list, dirs, .. } = td;
            let mut kids: Vec<Kid> = Vec::with_capacity(dirs.len() + file_list.len());
            kids.extend(dirs.into_iter().map(Kid::D));
            kids.extend(file_list.into_iter().map(|(s, o, l)| Kid::F(s, o, l)));
            kids.sort_unstable_by_key(|k| Reverse(k.size()));
            let start = nodes.len() as u32;
            nodes[idx as usize].child_start = start;
            nodes[idx as usize].child_len = kids.len() as u32;
            for k in kids {
                let ni = nodes.len() as u32;
                match k {
                    Kid::D(mut d) => {
                        let (name_off, name_len) = intern(&mut names, &d.name);
                        d.name = Box::default();
                        nodes.push(Node {
                            size: d.size,
                            files: d.files as u32,
                            parent: idx,
                            child_start: 0,
                            child_len: 0,
                            name_off,
                            name_len,
                            is_dir: true,
                            removed: false,
                        });
                        queue.push_back((ni, d));
                    }
                    Kid::F(size, o, l) => {
                        let (name_off, name_len) = intern(&mut names, &fnames[o as usize..(o + l) as usize]);
                        nodes.push(Node {
                            size,
                            files: 1,
                            parent: idx,
                            child_start: 0,
                            child_len: 0,
                            name_off,
                            name_len,
                            is_dir: false,
                            removed: false,
                        });
                    }
                }
            }
        }
        names.shrink_to_fit();
        let top_files = top(&nodes, |i, n| i > 0 && !n.is_dir);
        let top_dirs = top(&nodes, |i, n| i > 0 && n.is_dir);
        Tree {
            root_path,
            nodes,
            names,
            top_files,
            top_dirs,
            errors: 0,
            elapsed: Duration::ZERO,
            cancelled: false,
        }
    }

    pub fn node(&self, i: u32) -> &Node {
        &self.nodes[i as usize]
    }

    pub fn name(&self, i: u32) -> &str {
        let n = self.node(i);
        &self.names[n.name_off as usize..n.name_off as usize + n.name_len as usize]
    }

    pub fn children(&self, i: u32) -> impl Iterator<Item = u32> + '_ {
        let n = self.node(i);
        (n.child_start..n.child_start + n.child_len).filter(|&c| !self.nodes[c as usize].removed)
    }

    pub fn path_of(&self, mut i: u32) -> PathBuf {
        let mut parts = Vec::new();
        while i != 0 && i != NONE {
            parts.push(self.name(i));
            i = self.nodes[i as usize].parent;
        }
        let mut p = self.root_path.clone();
        for s in parts.iter().rev() {
            p.push(s);
        }
        p
    }

    /// True if neither the node nor any ancestor has been deleted.
    pub fn alive(&self, mut i: u32) -> bool {
        while i != NONE {
            let n = &self.nodes[i as usize];
            if n.removed {
                return false;
            }
            i = n.parent;
        }
        true
    }

    pub fn ancestors(&self, i: u32) -> impl Iterator<Item = u32> + '_ {
        let mut cur = self.node(i).parent;
        std::iter::from_fn(move || {
            if cur == NONE {
                None
            } else {
                let r = cur;
                cur = self.nodes[cur as usize].parent;
                Some(r)
            }
        })
    }

    /// Mark a node deleted and subtract its size from every ancestor.
    pub fn remove(&mut self, i: u32) {
        if i == 0 || self.nodes[i as usize].removed {
            return;
        }
        let (size, files) = (self.nodes[i as usize].size, self.nodes[i as usize].files);
        self.nodes[i as usize].removed = true;
        let mut p = self.nodes[i as usize].parent;
        while p != NONE {
            let n = &mut self.nodes[p as usize];
            n.size = n.size.saturating_sub(size);
            n.files = n.files.saturating_sub(files);
            p = n.parent;
        }
    }
}

fn top(nodes: &[Node], pred: impl Fn(usize, &Node) -> bool) -> Vec<u32> {
    let mut v: Vec<u32> = nodes
        .iter()
        .enumerate()
        .filter(|(i, n)| pred(*i, n))
        .map(|(i, _)| i as u32)
        .collect();
    let by_size = |a: &u32, b: &u32| nodes[*b as usize].size.cmp(&nodes[*a as usize].size);
    if v.len() > TOP_N {
        v.select_nth_unstable_by(TOP_N, by_size);
        v.truncate(TOP_N);
    }
    v.sort_unstable_by(by_size);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_sizes_and_remove() {
        let base = std::env::temp_dir().join(format!("spacescout_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("a/b")).unwrap();
        fs::create_dir_all(base.join("c")).unwrap();
        fs::write(base.join("a/b/big.bin"), vec![0u8; 5000]).unwrap();
        fs::write(base.join("a/small.txt"), vec![0u8; 100]).unwrap();
        fs::write(base.join("c/mid.bin"), vec![0u8; 2000]).unwrap();
        fs::write(base.join("root.txt"), vec![0u8; 10]).unwrap();

        let p = Progress::default();
        let mut t = run(base.clone(), &p);
        assert_eq!(t.nodes[0].size, 7110);
        assert_eq!(t.nodes[0].files, 4);
        // Children are sorted by size: a (5100), c (2000), root.txt (10)
        let kids: Vec<_> = t.children(0).map(|i| t.name(i).to_string()).collect();
        assert_eq!(kids, ["a", "c", "root.txt"]);
        assert_eq!(t.path_of(t.top_files[0]), base.join("a").join("b").join("big.bin"));
        assert_eq!(t.top_dirs.len(), 3);

        let a = t.children(0).next().unwrap();
        t.remove(a);
        assert_eq!(t.nodes[0].size, 2010);
        assert_eq!(t.nodes[0].files, 2);
        assert!(!t.alive(t.top_files[0]));
        fs::remove_dir_all(&base).unwrap();
    }
}

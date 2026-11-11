//! The file library: a folder of files that linked stations may browse and
//! download (Settings → File library; off by default).
//!
//! Requests are paths relative to the library folder. Anything that could
//! reach outside it is refused: absolute paths, `..`, hidden files and
//! folders (names starting with '.'), and links that resolve outside the
//! folder. Files over the size limit are listed but refused.

use crate::protocol::LibEntry;
use anyhow::{Result, bail};
use std::path::{Component, Path, PathBuf};

/// Most entries sent in one listing.
pub const MAX_ENTRIES: usize = 200;

/// The real path of `rel` inside `root`, or why not.
pub fn resolve(root: &Path, rel: &str) -> Result<PathBuf> {
    let root = root
        .canonicalize()
        .map_err(|_| anyhow::anyhow!("the library folder isn't there"))?;
    let rel = rel.trim().trim_start_matches('/').trim_end_matches('/');
    let mut p = root.clone();
    for c in Path::new(rel).components() {
        match c {
            Component::Normal(n) => {
                if n.to_string_lossy().starts_with('.') {
                    bail!("no such file");
                }
                p.push(n);
            }
            Component::CurDir => {}
            _ => bail!("no such file"),
        }
    }
    let real = p
        .canonicalize()
        .map_err(|_| anyhow::anyhow!("no such file"))?;
    if !real.starts_with(&root) {
        bail!("no such file");
    }
    Ok(real)
}

/// What's in folder `rel` of the library: folders first, then files, by name.
pub fn list(root: &Path, rel: &str) -> Result<Vec<LibEntry>> {
    let dir = resolve(root, rel)?;
    if !dir.is_dir() {
        bail!("not a folder");
    }
    let croot = root.canonicalize()?;
    let mut v: Vec<LibEntry> = std::fs::read_dir(&dir)?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                return None;
            }
            // Follow links, but only to things inside the library.
            let real = e.path().canonicalize().ok()?;
            if !real.starts_with(&croot) {
                return None;
            }
            let md = std::fs::metadata(&real).ok()?;
            Some(LibEntry {
                name,
                size: if md.is_dir() { 0 } else { md.len() },
                dir: md.is_dir(),
            })
        })
        .collect();
    v.sort_by(|a, b| {
        b.dir
            .cmp(&a.dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    v.truncate(MAX_ENTRIES);
    Ok(v)
}

/// A file to send: its name and contents, if allowed.
pub fn read(root: &Path, rel: &str, max_bytes: u64) -> Result<(String, Vec<u8>)> {
    let f = resolve(root, rel)?;
    if !f.is_file() {
        bail!("not a file");
    }
    let size = std::fs::metadata(&f)?.len();
    if size > max_bytes {
        bail!(
            "too big to send ({} KB; the limit is {} KB)",
            size / 1024,
            max_bytes / 1024
        );
    }
    let name = f
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or("file".into());
    Ok((name, std::fs::read(&f)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib() -> PathBuf {
        let d = std::env::temp_dir().join(format!("ws-lib-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("maps/old")).unwrap();
        std::fs::write(d.join("readme.txt"), "hello").unwrap();
        std::fs::write(d.join("maps/grid.png"), vec![7u8; 3000]).unwrap();
        std::fs::write(d.join(".secret"), "no").unwrap();
        d
    }

    #[test]
    fn lists_and_reads_inside_only() {
        let d = lib();
        let top = list(&d, "").unwrap();
        assert_eq!(
            top.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["maps", "readme.txt"]
        );
        assert!(top[0].dir && !top[1].dir && top[1].size == 5);
        assert_eq!(list(&d, "maps/").unwrap().len(), 2);
        assert_eq!(
            read(&d, "maps/grid.png", 10_000).unwrap(),
            ("grid.png".to_string(), vec![7u8; 3000])
        );
        assert!(
            read(&d, "maps/grid.png", 1000)
                .unwrap_err()
                .to_string()
                .contains("too big")
        );
        for bad in [
            "../etc/passwd",
            "/etc/passwd",
            ".secret",
            "maps/../../x",
            "maps/../.secret",
        ] {
            assert!(read(&d, bad, 1 << 20).is_err(), "{bad} should be refused");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn links_out_of_the_library_are_refused() {
        let d = lib();
        let outside = std::env::temp_dir().join(format!("ws-outside-{}", std::process::id()));
        std::fs::write(&outside, "private").unwrap();
        std::os::unix::fs::symlink(&outside, d.join("sneaky.txt")).unwrap();
        assert!(read(&d, "sneaky.txt", 1 << 20).is_err());
        assert!(!list(&d, "").unwrap().iter().any(|e| e.name == "sneaky.txt"));
        let _ = std::fs::remove_file(outside);
        let _ = std::fs::remove_dir_all(&d);
    }
}

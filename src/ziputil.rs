//! Built-in zip: bundle files and folders before sending them.

use anyhow::{Context, Result};
use std::collections::HashSet;
use std::io::{Cursor, Write};
use std::path::{Component, Path, PathBuf};
use zip::write::SimpleFileOptions;

pub struct Zipped {
    pub name: String,
    pub data: Vec<u8>,
    pub original_bytes: u64,
    pub files: usize,
}

impl Zipped {
    pub fn ratio(&self) -> f64 {
        if self.original_bytes == 0 {
            1.0
        } else {
            self.data.len() as f64 / self.original_bytes as f64
        }
    }
}

/// Zip `paths` (files and/or folders, recursively) into memory.
pub fn zip_paths(paths: &[PathBuf], name_hint: Option<&str>) -> Result<Zipped> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(9));
    let mut original = 0u64;
    let mut files = 0;
    // Entry names used so far, lowercased (unzipping on Windows or macOS
    // folds case).
    let mut used: HashSet<String> = HashSet::new();
    for p in paths {
        // "dir/sub/.." names "dir": resolve it, so the entries are named
        // after the real folder, not "../...".
        let p = if p.components().any(|c| matches!(c, Component::ParentDir)) {
            p.canonicalize()
                .with_context(|| format!("reading {}", p.display()))?
        } else {
            p.clone()
        };
        let base = p.parent().unwrap_or(Path::new(""));
        for entry in walkdir::WalkDir::new(&p).follow_links(false) {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }
            let rel = entry.path().strip_prefix(base).unwrap_or(entry.path());
            let rel = entry_name(rel, entry.path());
            let rel = unique_entry(&mut used, rel);
            let bytes = std::fs::read(entry.path())
                .with_context(|| format!("reading {}", entry.path().display()))?;
            original += bytes.len() as u64;
            files += 1;
            w.start_file(rel, opts)?;
            w.write_all(&bytes)?;
        }
    }
    let data = w.finish()?.into_inner();
    let name = match name_hint {
        Some(n) => n.to_string(),
        None if paths.len() == 1 => format!(
            "{}.zip",
            paths[0]
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "files".into())
        ),
        None => "files.zip".into(),
    };
    Ok(Zipped {
        name,
        data,
        original_bytes: original,
        files,
    })
}

/// A zip entry name for `rel`: only its ordinary components, joined with
/// '/' (never "..", a root or a drive). Falls back to the file's own name.
fn entry_name(rel: &Path, full: &Path) -> String {
    let parts: Vec<String> = rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(n) => Some(n.to_string_lossy().replace('\\', "/")),
            _ => None,
        })
        .flat_map(|s| {
            s.split('/')
                .filter(|c| !c.is_empty() && *c != "." && *c != "..")
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect();
    if parts.is_empty() {
        full.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|n| n != "..")
            .unwrap_or_else(|| "file".into())
    } else {
        parts.join("/")
    }
}

/// `name`, or "name-1.ext", "name-2.ext", ... if already in the zip.
fn unique_entry(used: &mut HashSet<String>, name: String) -> String {
    if used.insert(name.to_lowercase()) {
        return name;
    }
    let (dir, file) = match name.rfind('/') {
        Some(i) => (&name[..=i], &name[i + 1..]),
        None => ("", name.as_str()),
    };
    let (stem, ext) = match file.rfind('.') {
        Some(i) if i > 0 => (&file[..i], &file[i..]),
        _ => (file, ""),
    };
    (1..)
        .map(|i| format!("{dir}{stem}-{i}{ext}"))
        .find(|c| used.insert(c.to_lowercase()))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zips_a_folder_smaller() {
        let dir = std::env::temp_dir().join(format!("ws-zip-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), "CQ CQ CQ de N0CALL ".repeat(500)).unwrap();
        std::fs::write(dir.join("sub/b.txt"), "73 ".repeat(500)).unwrap();
        let z = zip_paths(std::slice::from_ref(&dir), None).unwrap();
        assert_eq!(z.files, 2);
        assert!(z.ratio() < 0.1, "ratio {}", z.ratio());
        let r = zip::ZipArchive::new(Cursor::new(z.data)).unwrap();
        assert_eq!(r.len(), 2);
        std::fs::remove_dir_all(dir).ok();
    }
}

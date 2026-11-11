//! Built-in zip: bundle files and folders before sending them.

use anyhow::{Context, Result};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;

pub struct Zipped {
    pub name: String,
    pub data: Vec<u8>,
    pub original_bytes: u64,
    pub files: usize,
}

impl Zipped {
    pub fn ratio(&self) -> f64 {
        if self.original_bytes == 0 { 1.0 } else { self.data.len() as f64 / self.original_bytes as f64 }
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
    for p in paths {
        let base = p.parent().unwrap_or(Path::new(""));
        for entry in walkdir::WalkDir::new(p).follow_links(false) {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }
            let rel = entry.path().strip_prefix(base).unwrap_or(entry.path());
            let rel = rel.to_string_lossy().replace('\\', "/");
            let bytes = std::fs::read(entry.path()).with_context(|| format!("reading {}", entry.path().display()))?;
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
            paths[0].file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "files".into())
        ),
        None => "files.zip".into(),
    };
    Ok(Zipped { name, data, original_bytes: original, files })
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

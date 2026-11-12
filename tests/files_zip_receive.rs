//! Sending (built-in zip) and receiving files: names from the air, resume,
//! size limits, odd inputs to the zipper.
//!
//! Note: Whistlestop never unzips what it receives (a received zip is saved
//! as the .zip file), so zip bombs and zip-slip on receive don't apply; what
//! matters is that the zips we *make* are well-formed for the other side.

use std::io::{Cursor, Read};
use std::path::PathBuf;
use whistlestop::protocol::*;
use whistlestop::ziputil::*;

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ws-itest-files-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn entries(data: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut z = zip::ZipArchive::new(Cursor::new(data.to_vec())).unwrap();
    (0..z.len())
        .map(|i| {
            let mut f = z.by_index(i).unwrap();
            let mut v = vec![];
            f.read_to_end(&mut v).unwrap();
            (f.name().unwrap().to_string(), v)
        })
        .collect()
}

#[test]
fn zips_a_tree_with_relative_forward_slash_names() {
    let d = tmp("tree");
    let root = d.join("Field Day ✓");
    std::fs::create_dir_all(root.join("logs/2026")).unwrap();
    std::fs::create_dir_all(root.join("empty")).unwrap();
    std::fs::write(root.join("a.txt"), "CQ ".repeat(1000)).unwrap();
    std::fs::write(root.join("logs/2026/b.adi"), "73").unwrap();
    std::fs::write(root.join("logs/zero.bin"), "").unwrap();
    let z = zip_paths(std::slice::from_ref(&root), None).unwrap();
    assert_eq!(z.name, "Field Day ✓.zip");
    assert_eq!(z.files, 3);
    assert_eq!(z.original_bytes, 3000 + 2);
    let mut e = entries(&z.data);
    e.sort();
    let names: Vec<&str> = e.iter().map(|x| x.0.as_str()).collect();
    assert_eq!(names, ["Field Day ✓/a.txt", "Field Day ✓/logs/2026/b.adi", "Field Day ✓/logs/zero.bin"]);
    for (n, _) in &e {
        assert!(!n.starts_with('/') && !n.contains('\\') && !n.split('/').any(|c| c == ".."), "{n}");
    }
    assert!(z.ratio() < 0.5, "{}", z.ratio());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn single_file_and_name_hints() {
    let d = tmp("single");
    let f = d.join("photo.jpg");
    std::fs::write(&f, [1u8, 2, 3]).unwrap();
    let z = zip_paths(std::slice::from_ref(&f), None).unwrap();
    assert_eq!(z.name, "photo.jpg.zip");
    assert_eq!(entries(&z.data), vec![("photo.jpg".to_string(), vec![1, 2, 3])]);
    let z = zip_paths(std::slice::from_ref(&f), Some("custom.zip")).unwrap();
    assert_eq!(z.name, "custom.zip");
    let g = d.join("other.txt");
    std::fs::write(&g, "x").unwrap();
    let z = zip_paths(&[f, g], None).unwrap();
    assert_eq!((z.name.as_str(), z.files), ("files.zip", 2));
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn empty_inputs() {
    let d = tmp("empty");
    let z = zip_paths(&[], None).unwrap();
    assert_eq!((z.files, z.original_bytes, z.ratio()), (0, 0, 1.0));
    assert!(entries(&z.data).is_empty());
    let e = d.join("nothing");
    std::fs::create_dir_all(&e).unwrap();
    let z = zip_paths(&[e], None).unwrap();
    assert_eq!(z.files, 0);
    assert!(zip_paths(&[d.join("does-not-exist")], None).is_err());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn big_compressible_file_declares_its_true_size() {
    let d = tmp("big");
    let f = d.join("zeros.bin");
    std::fs::write(&f, vec![0u8; 8 << 20]).unwrap();
    let z = zip_paths(std::slice::from_ref(&f), None).unwrap();
    assert!(z.data.len() < 100_000, "{}", z.data.len());
    let mut a = zip::ZipArchive::new(Cursor::new(z.data)).unwrap();
    assert_eq!(a.by_index(0).unwrap().size(), 8 << 20);
    let _ = std::fs::remove_dir_all(d);
}

/// Dropping two files with the same name from different folders (e.g.
/// `~/a/notes.txt` and `~/b/notes.txt`) gives two zip entries with the same
/// name, which the zip writer refuses: the whole send fails.
#[test]
fn same_name_from_two_folders() {
    let d = tmp("dup");
    std::fs::create_dir_all(d.join("a")).unwrap();
    std::fs::create_dir_all(d.join("b")).unwrap();
    std::fs::write(d.join("a/notes.txt"), "A").unwrap();
    std::fs::write(d.join("b/notes.txt"), "B").unwrap();
    let r = zip_paths(&[d.join("a/notes.txt"), d.join("b/notes.txt")], None);
    let z = r.unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(z.files, 2);
    assert_eq!(entries(&z.data).len(), 2);
    let _ = std::fs::remove_dir_all(d);
}

/// A path given with `..` in it ("dir/sub/..") produces entries named
/// "../file": a zip-slip archive for whoever unzips it at the other end.
#[test]
fn no_parent_components_in_entry_names() {
    let d = tmp("dotdot");
    std::fs::create_dir_all(d.join("dir/sub")).unwrap();
    std::fs::write(d.join("dir/a.txt"), "x").unwrap();
    let z = zip_paths(&[d.join("dir/sub/..")], None).unwrap();
    for (n, _) in entries(&z.data) {
        assert!(!n.split('/').any(|c| c == ".."), "entry {n:?}");
    }
    let _ = std::fs::remove_dir_all(d);
}

#[cfg(unix)]
#[test]
fn symlinks_are_not_followed_out_of_the_tree() {
    let d = tmp("links");
    std::fs::create_dir_all(d.join("send")).unwrap();
    std::fs::write(d.join("private.txt"), "private").unwrap();
    std::fs::write(d.join("send/ok.txt"), "ok").unwrap();
    std::os::unix::fs::symlink(d.join("private.txt"), d.join("send/link.txt")).unwrap();
    std::os::unix::fs::symlink(&d, d.join("send/up")).unwrap();
    let z = zip_paths(&[d.join("send")], None).unwrap();
    let e = entries(&z.data);
    assert_eq!(e, vec![("send/ok.txt".to_string(), b"ok".to_vec())]);
    let _ = std::fs::remove_dir_all(d);
}

// ------------------------------------------------------------- receiving

fn receive(dir: &std::path::Path, name: &str, data: &[u8]) -> anyhow::Result<PathBuf> {
    let sha = sha256(data);
    let mut inc = Incoming::new(dir, 1, name.into(), data.len() as u64, sha, false);
    for (i, c) in data.chunks(CHUNK).enumerate() {
        inc.chunk((i * CHUNK) as u64, c);
    }
    assert!(inc.complete());
    inc.finish(dir)
}

#[test]
fn names_from_the_air_cannot_escape_the_download_folder() {
    let base = tmp("names");
    let dl = base.join("dl");
    for name in [
        "../../evil.txt",
        "/etc/evil.txt",
        "..",
        ".",
        "",
        "a/b/c.txt",
        "./x.txt",
        "../",
        "/",
        "dir/..",
        "..\\..\\evil.txt",
        "C:\\Windows\\evil.txt",
        "\\\\server\\share\\evil.txt",
    ] {
        let p = receive(&dl, name, b"payload").unwrap_or_else(|e| panic!("{name:?}: {e}"));
        assert_eq!(p.parent().unwrap(), dl, "{name:?} saved at {p:?}");
        assert!(p.file_name().is_some_and(|n| n != ".." && n != "."), "{name:?} -> {p:?}");
        assert_eq!(std::fs::read(&p).unwrap(), b"payload");
    }
    // Nothing landed outside.
    let outside: Vec<_> = std::fs::read_dir(&base).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(outside, vec![std::ffi::OsString::from("dl")]);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn same_name_twice_is_numbered() {
    let base = tmp("unique");
    let a = receive(&base, "log.adi", b"1").unwrap();
    let b = receive(&base, "log.adi", b"22").unwrap();
    let c = receive(&base, "log.adi", b"333").unwrap();
    assert_eq!(a.file_name().unwrap(), "log.adi");
    assert_eq!(b.file_name().unwrap(), "log-1.adi");
    assert_eq!(c.file_name().unwrap(), "log-2.adi");
    let n = receive(&base, "README", b"x").unwrap();
    let n2 = receive(&base, "README", b"y").unwrap();
    assert_eq!(n.file_name().unwrap(), "README");
    assert_eq!(n2.file_name().unwrap(), "README-1");
    let _ = std::fs::remove_dir_all(base);
}

/// The name is the sender's; a 300-byte name (legal on the sender's side
/// in UTF-8, or simply hostile) is longer than Linux/macOS allow (255
/// bytes), so `finish` fails *after* the whole file has crossed the air.
/// It should be shortened (or fall back to received.bin) instead.
#[test]
fn over_long_names_are_shortened_not_fatal() {
    let base = tmp("long");
    for name in ["a".repeat(300) + ".txt", "日".repeat(100) + ".txt", "x\0y.txt".into()] {
        let r = receive(&base, &name, b"data");
        assert!(r.is_ok(), "{} bytes: {:?}", name.len(), r.err());
        let saved = r.unwrap().file_name().unwrap().to_string_lossy().into_owned();
        assert!(saved.len() <= MAX_NAME_BYTES && saved.ends_with(".txt"), "{saved:?}");
    }
    // Names Windows can't create are made legal there too.
    for (raw, want) in [
        ("x\0y.txt", "xy.txt"),
        ("CON", "_CON"),
        ("nul.tar.gz", "_nul.tar.gz"),
        ("com1.txt", "_com1.txt"),
        ("COM10.txt", "COM10.txt"),
        ("console.txt", "console.txt"),
        ("notes. . ", "notes"),
        ("a:b|c?.txt", "a_b_c_.txt"),
        ("\u{1}\u{7f}", "received.bin"),
        ("...", "received.bin"),
    ] {
        assert_eq!(safe_file_name(raw), want, "{raw:?}");
    }
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn checksum_mismatch_is_refused_and_forgotten() {
    let base = tmp("sha");
    let mut inc = Incoming::new(&base, 1, "f.bin".into(), 4, [0; 32], false);
    inc.chunk(0, b"abcd");
    inc.save_partial();
    assert!(inc.finish(&base).unwrap_err().to_string().contains("checksum"));
    // The bad partial is gone: a new offer with that sha starts from 0.
    let inc = Incoming::new(&base, 2, "f.bin".into(), 4, [0; 32], false);
    assert_eq!(inc.have(), 0);
    assert!(!base.join("f.bin").exists());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn chunks_beyond_the_offer_or_out_of_order_are_ignored() {
    let base = tmp("chunks");
    let data = b"0123456789";
    let mut inc = Incoming::new(&base, 1, "x".into(), 10, sha256(data), false);
    inc.chunk(5, b"56789"); // out of order
    assert_eq!(inc.have(), 0);
    inc.chunk(0, b"01234");
    inc.chunk(0, b"01234"); // repeat
    assert_eq!(inc.have(), 5);
    inc.chunk(5, b"56789EXTRA-BYTES");
    assert_eq!(inc.have(), 10);
    inc.chunk(10, b"more");
    inc.chunk(u64::MAX, b"x");
    assert!(inc.complete());
    let p = inc.finish(&base).unwrap();
    assert_eq!(std::fs::read(p).unwrap(), data);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn resume_ignores_a_partial_longer_than_the_offer() {
    let base = tmp("resume");
    let data = vec![5u8; 3000];
    let sha = sha256(&data);
    let mut inc = Incoming::new(&base, 1, "r.bin".into(), 3000, sha, false);
    inc.chunk(0, &data[..2048]);
    inc.save_partial();
    // Same sha, honest size: resumes.
    assert_eq!(Incoming::new(&base, 2, "r.bin".into(), 3000, sha, false).have(), 2048);
    // Same sha, but now claims to be smaller than what we hold: start over.
    assert_eq!(Incoming::new(&base, 3, "r.bin".into(), 1000, sha, false).have(), 0);
    // A huge claimed size doesn't allocate anything up front.
    let inc = Incoming::new(&base, 4, "r.bin".into(), u64::MAX, sha, false);
    assert_eq!(inc.have(), 2048);
    assert!(!inc.complete());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn outgoing_chunks_cover_the_file_exactly() {
    for len in [0usize, 1, CHUNK - 1, CHUNK, CHUNK + 1, 5 * CHUNK + 17] {
        let data: Vec<u8> = (0..len).map(|i| (i * 31) as u8).collect();
        for start in [0u64, 1, CHUNK as u64, len as u64, len as u64 + 5] {
            let mut o = Outgoing::new(9, "f".into(), data.clone(), false, 1);
            assert!(o.next_chunk().is_none(), "nothing before ANSWER");
            o.next = Some(start);
            let mut got: Vec<u8> = vec![];
            let mut expect_off = start;
            while let Some(Link::Chunk { offset, data: d, fid }) = o.next_chunk() {
                assert_eq!((fid, offset), (9, expect_off));
                assert!(d.len() <= CHUNK && !d.is_empty());
                expect_off += d.len() as u64;
                got.extend(d);
            }
            let s = (start as usize).min(len);
            assert_eq!(got, data[s..], "len {len} start {start}");
            assert!(o.finished_sending());
        }
    }
}

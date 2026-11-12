//! File library: nothing outside the folder may be reached (USER-GUIDE,
//! "File library"): no `..`, no absolute paths, no hidden files, no links
//! that lead outside it; files over the limit are listed but refused.

use std::path::{Path, PathBuf};
use whistlestop::config::Config;
use whistlestop::engine::{library_answer, library_file};
use whistlestop::library::*;
use whistlestop::protocol::Link;

struct Lib {
    base: PathBuf,
    root: PathBuf,
}

impl Drop for Lib {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

/// base/
///   lib/            <- the library
///     readme.txt, maps/grid.png, maps/old/, .secret, .hidden/inside.txt
///   lib-evil/x.txt  <- a sibling whose name starts like the library's
///   outside.txt
fn lib(tag: &str) -> Lib {
    let base = std::env::temp_dir().join(format!("ws-itest-lib-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let root = base.join("lib");
    std::fs::create_dir_all(root.join("maps/old")).unwrap();
    std::fs::create_dir_all(root.join(".hidden")).unwrap();
    std::fs::create_dir_all(base.join("lib-evil")).unwrap();
    std::fs::write(root.join("readme.txt"), "hello").unwrap();
    std::fs::write(root.join("maps/grid.png"), vec![7u8; 3000]).unwrap();
    std::fs::write(root.join(".secret"), "secret").unwrap();
    std::fs::write(root.join(".hidden/inside.txt"), "hidden").unwrap();
    std::fs::write(base.join("lib-evil/x.txt"), "evil").unwrap();
    std::fs::write(base.join("outside.txt"), "private").unwrap();
    Lib { base, root }
}

fn names(v: &[whistlestop::protocol::LibEntry]) -> Vec<String> {
    v.iter().map(|e| e.name.clone()).collect()
}

#[test]
fn traversal_attempts_are_refused() {
    let l = lib("trav");
    let root_s = l.root.to_string_lossy().to_string();
    let outside_abs = l.base.join("outside.txt").to_string_lossy().to_string();
    let long = "a".repeat(5000);
    let deep = "maps/".repeat(3000) + "grid.png";
    let bad: Vec<String> = vec![
        "..".into(),
        "../".into(),
        "../outside.txt".into(),
        "./../outside.txt".into(),
        "maps/../../outside.txt".into(),
        "maps/old/../../../outside.txt".into(),
        "maps/./../..".into(),
        "/../outside.txt".into(),
        "//../outside.txt".into(),
        outside_abs.clone(),
        format!("{root_s}/../outside.txt"),
        "../lib-evil/x.txt".into(),
        "..\\outside.txt".into(),
        "maps\\..\\..\\outside.txt".into(),
        "C:\\Windows\\win.ini".into(),
        "C:/Windows/win.ini".into(),
        "\\\\server\\share\\x".into(),
        "//server/share/x".into(),
        "\\\\?\\C:\\x".into(),
        "%2e%2e/outside.txt".into(),
        "~/outside.txt".into(),
        "\0".into(),
        "readme.txt\0../../outside.txt".into(),
        ".secret".into(),
        "./.secret".into(),
        "maps/../.secret".into(),
        ".hidden/inside.txt".into(),
        ".hidden".into(),
        ".".into(),
        "".into(),
        long,
        "maps/".repeat(3000),
        deep,
    ];
    for b in &bad {
        let r = read(&l.root, b, u64::MAX);
        if let Ok((name, data)) = &r {
            panic!("{b:?} served {name:?} ({} bytes: {:?})", data.len(), String::from_utf8_lossy(data));
        }
        // Listing a traversal path never shows what's outside.
        if let Ok(v) = list(&l.root, b) {
            let n = names(&v);
            assert!(
                !n.iter().any(|x| x == "outside.txt" || x == "lib" || x == "lib-evil" || x.starts_with('.')),
                "{b:?} listed {n:?}"
            );
        }
    }
}

#[test]
fn normal_paths_work_and_slashes_are_forgiven() {
    let l = lib("ok");
    for p in ["readme.txt", "/readme.txt", "./readme.txt", " readme.txt ", "maps/../readme.txt"] {
        // `maps/..` is refused outright (no `..` at all), the rest work.
        let r = read(&l.root, p, 1 << 20);
        if p.contains("..") {
            assert!(r.is_err(), "{p:?}");
        } else {
            assert_eq!(r.unwrap(), ("readme.txt".to_string(), b"hello".to_vec()), "{p:?}");
        }
    }
    assert_eq!(read(&l.root, "maps//grid.png", 1 << 20).unwrap().1.len(), 3000);
    assert_eq!(names(&list(&l.root, "maps/").unwrap()), ["old", "grid.png"]);
    assert_eq!(names(&list(&l.root, "/").unwrap()), ["maps", "readme.txt"]);
    assert!(list(&l.root, "readme.txt").unwrap_err().to_string().contains("not a folder"));
    assert!(read(&l.root, "maps", 1 << 20).unwrap_err().to_string().contains("not a file"));
}

#[test]
fn size_limit_boundaries() {
    let l = lib("size");
    assert!(read(&l.root, "maps/grid.png", 3000).is_ok(), "exactly the limit is allowed");
    let e = read(&l.root, "maps/grid.png", 2999).unwrap_err().to_string();
    assert!(e.contains("too big"), "{e}");
    std::fs::write(l.root.join("empty.txt"), "").unwrap();
    assert!(read(&l.root, "empty.txt", 0).is_ok());
    assert!(read(&l.root, "readme.txt", 0).is_err());
    // Listed even when too big to send.
    let v = list(&l.root, "maps").unwrap();
    assert!(v.iter().any(|e| e.name == "grid.png" && e.size == 3000 && !e.dir));
}

#[test]
fn empty_and_missing_folders() {
    let l = lib("empty");
    let empty = l.base.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    assert!(list(&empty, "").unwrap().is_empty());
    let missing = l.base.join("nope");
    let e = list(&missing, "").unwrap_err().to_string();
    assert!(e.contains("isn't there"), "{e}");
    assert!(read(&missing, "x", 1 << 20).is_err());
    // A file given as the library folder.
    assert!(list(&l.base.join("outside.txt"), "").is_err());
}

#[test]
fn listing_order_hidden_files_and_cap() {
    let l = lib("cap");
    for i in 0..250 {
        std::fs::write(l.root.join(format!("f{i:03}.txt")), "x").unwrap();
    }
    std::fs::create_dir_all(l.root.join("Zdir")).unwrap();
    std::fs::create_dir_all(l.root.join("adir")).unwrap();
    std::fs::write(l.root.join("B.txt"), "x").unwrap();
    let v = list(&l.root, "").unwrap();
    assert_eq!(v.len(), MAX_ENTRIES);
    let n = names(&v);
    // Folders first, then files, case-insensitive by name.
    assert_eq!(&n[..3], ["adir", "maps", "Zdir"]);
    assert_eq!(n[3], "B.txt");
    assert!(!n.iter().any(|x| x.starts_with('.')));
    assert!(v[..3].iter().all(|e| e.dir && e.size == 0));
}

#[test]
fn unicode_names() {
    let l = lib("uni");
    for name in ["Ünïcødé.txt", "日本語.txt", "emoji 🛰.txt", "with space.txt", "a\\b.txt"] {
        std::fs::write(l.root.join(name), name).unwrap();
        assert_eq!(read(&l.root, name, 1 << 20).unwrap().1, name.as_bytes(), "{name}");
    }
    let n = names(&list(&l.root, "").unwrap());
    assert!(n.contains(&"日本語.txt".to_string()));
}

/// Every file the library lists can be downloaded by the name it lists.
/// A file whose name starts or ends with a space is listed, but `resolve`
/// trims the request, so it can never be fetched.
#[test]
fn every_listed_file_can_be_fetched() {
    let l = lib("spaces");
    std::fs::write(l.root.join(" lead.txt"), "1").unwrap();
    std::fs::write(l.root.join("trail.txt "), "2").unwrap();
    for e in list(&l.root, "").unwrap() {
        if !e.dir {
            assert!(read(&l.root, &e.name, 1 << 20).is_ok(), "listed but unreadable: {:?}", e.name);
        }
    }
}

#[cfg(unix)]
mod links {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn links_leading_outside_are_refused_and_hidden() {
        let l = lib("links");
        symlink(l.base.join("outside.txt"), l.root.join("out.txt")).unwrap();
        symlink(&l.base, l.root.join("up")).unwrap();
        symlink("/", l.root.join("rootfs")).unwrap();
        symlink(l.base.join("lib-evil"), l.root.join("evil")).unwrap();
        symlink(l.base.join("lib-evil/x.txt"), l.root.join("evil.txt")).unwrap();
        symlink(l.root.join("nonexistent"), l.root.join("dangling")).unwrap();
        symlink(l.root.join("loop"), l.root.join("loop")).unwrap();
        for p in ["out.txt", "up/outside.txt", "rootfs/etc/passwd", "evil/x.txt", "evil.txt", "dangling", "loop"] {
            assert!(read(&l.root, p, u64::MAX).is_err(), "{p} served");
        }
        for p in ["up", "rootfs", "evil"] {
            assert!(list(&l.root, p).is_err(), "{p} listed");
        }
        let n = names(&list(&l.root, "").unwrap());
        for p in ["out.txt", "up", "rootfs", "evil", "evil.txt", "dangling", "loop"] {
            assert!(!n.contains(&p.to_string()), "{p} listed in {n:?}");
        }
    }

    #[test]
    fn links_inside_the_library_are_followed() {
        let l = lib("inlinks");
        symlink(l.root.join("maps/grid.png"), l.root.join("grid-link.png")).unwrap();
        symlink(l.root.join("maps"), l.root.join("maps-link")).unwrap();
        assert_eq!(read(&l.root, "grid-link.png", 1 << 20).unwrap().1.len(), 3000);
        assert_eq!(read(&l.root, "maps-link/grid.png", 1 << 20).unwrap().1.len(), 3000);
        let v = list(&l.root, "").unwrap();
        assert!(v.iter().any(|e| e.name == "maps-link" && e.dir));
    }

    #[test]
    fn library_folder_itself_may_be_a_link() {
        let l = lib("rootlink");
        let link = l.base.join("lib-link");
        symlink(&l.root, &link).unwrap();
        assert!(read(&link, "readme.txt", 1 << 20).is_ok());
        assert!(read(&link, "../outside.txt", 1 << 20).is_err());
    }

    /// Hidden files are off limits (USER-GUIDE: "no hidden files (names
    /// starting with `.`)"), but a link with an ordinary name that points
    /// to a hidden file *inside* the library is followed, because only the
    /// requested name is checked, not where it leads.
    #[test]
    fn links_to_hidden_files_are_refused() {
        let l = lib("hidlink");
        symlink(l.root.join(".secret"), l.root.join("notes.txt")).unwrap();
        symlink(l.root.join(".hidden"), l.root.join("stuff")).unwrap();
        assert!(read(&l.root, "notes.txt", 1 << 20).is_err(), "served .secret via notes.txt");
        assert!(read(&l.root, "stuff/inside.txt", 1 << 20).is_err(), "served .hidden/ via stuff/");
    }
}

fn cfg_for(root: &Path) -> Config {
    Config {
        share_library: true,
        library_dir: format!("  {}  ", root.display()),
        library_max_mb: 0.001, // ~1 KB
        ..Default::default()
    }
}

#[test]
fn engine_library_answers_follow_settings() {
    let l = lib("engine");
    let mut c = cfg_for(&l.root);
    let Link::ListAns { id, entries, note, path } = library_answer(&c, false, 42, "maps") else { panic!() };
    assert_eq!((id, path.as_str(), note.as_str()), (42, "maps", ""));
    assert_eq!(names(&entries), ["old", "grid.png"]);
    assert!(library_file(&c, false, "readme.txt").is_ok());
    assert!(library_file(&c, false, "maps/grid.png").unwrap_err().to_string().contains("too big"));
    // Odd limits never panic; negative and NaN mean nothing but empty files.
    for mb in [-1.0, f64::NAN, 0.0] {
        c.library_max_mb = mb;
        assert!(library_file(&c, false, "readme.txt").is_err(), "{mb}");
    }
    c.library_max_mb = f64::INFINITY;
    assert!(library_file(&c, false, "maps/grid.png").is_ok());
    // Traversal through the engine API.
    let Link::ListAns { entries, note, .. } = library_answer(&c, false, 1, "..") else { panic!() };
    assert!(entries.is_empty() && !note.is_empty());
    // Off, folder empty, or away-only.
    c.library_only_away = true;
    assert!(library_file(&c, false, "readme.txt").is_err());
    assert!(library_file(&c, true, "readme.txt").is_ok());
    c.library_dir = "   ".into();
    let Link::ListAns { entries, note, .. } = library_answer(&c, true, 1, "") else { panic!() };
    assert!(entries.is_empty() && note.contains("no file library"));
    c.library_dir = l.root.display().to_string();
    c.share_library = false;
    assert!(library_file(&c, true, "readme.txt").is_err());
}

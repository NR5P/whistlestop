//! Restarts must not leak: twenty Demo restarts leave no sockets or threads
//! behind. In a test binary of its own, because it counts this process's
//! open files and threads (Linux's /proc), and other tests running beside it
//! in the same process would open and close theirs meanwhile: on a 3-core
//! machine, soak_growth's other tests moved the count 65 -> 78 while this
//! test alone stays at 21 -> 21.

mod soak_common;

use soak_common::*;
use std::time::Duration;
use whistlestop::engine::Cmd;

/// Settings → Save & connect in Demo restarts the station. Twenty restarts
/// must not leave sockets or threads behind.
#[test]
fn demo_restarts_do_not_leak() {
    let stn = demo("N0RST", |_| {});
    std::thread::sleep(Duration::from_secs(2));
    let (f0, t0) = (fds(), threads());
    for _ in 0..20 {
        stn.send(Cmd::Start(stn.cfg.clone()));
        std::thread::sleep(Duration::from_millis(400));
        stn.wait("restarted", 20.0, |s| s.running && s.mercury_ok);
    }
    std::thread::sleep(Duration::from_secs(3));
    let (f1, t1) = (fds(), threads());
    eprintln!("20 Demo restarts: fds {f0} -> {f1}, threads {t0} -> {t1}");
    assert!(f1 <= f0 + 10, "file descriptors {f0} -> {f1} after 20 restarts");
    assert!(t1 <= t0 + 5, "threads {t0} -> {t1} after 20 restarts");
}

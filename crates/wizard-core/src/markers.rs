//! The done markers (DESIGN.md, Done markers). All paths are relative to a
//! `root`, `/` on a real system and a temp dir in tests.

use crate::fsutil::write_atomic;
use crate::ini::Ini;
use crate::timefmt::rfc3339_utc;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Telamon's marker, relative to the root.
pub const TELAMON_MARKER: &str = "etc/telamon/setup-done";
/// Atlas Wizard's marker (0.1.x), relative to the root: still written, because
/// the image's health check (and an image that still has Atlas Wizard, after a
/// rollback) read it, and still counts as done on a machine it was written on.
pub const ATLAS_MARKER: &str = "etc/atlasos/setup-done";
/// plasma-setup's marker, relative to the root.
pub const PLASMA_MARKER: &str = "etc/plasma-setup-done";

const MODE: u32 = 0o644;

/// Absolute path of Telamon's marker under `root`.
pub fn telamon_path(root: &Path) -> PathBuf {
    root.join(TELAMON_MARKER)
}

/// Absolute path of Atlas Wizard's marker under `root`.
pub fn atlas_path(root: &Path) -> PathBuf {
    root.join(ATLAS_MARKER)
}

/// Absolute path of the plasma-setup marker under `root`.
pub fn plasma_path(root: &Path) -> PathBuf {
    root.join(PLASMA_MARKER)
}

/// What `/etc/telamon/setup-done` (or `/etc/atlasos/setup-done`) says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    /// `Version`, when a number.
    pub version: Option<u32>,
    /// `Finished`, RFC 3339 text as written.
    pub finished: Option<String>,
    /// `Wizard`, the crate version that wrote it.
    pub wizard: Option<String>,
}

/// Which markers exist.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Present {
    /// `/etc/telamon/setup-done` exists.
    pub telamon: bool,
    /// `/etc/atlasos/setup-done` exists (Atlas Wizard's).
    pub atlas: bool,
    /// `/etc/plasma-setup-done` exists.
    pub plasma: bool,
}

impl Present {
    /// True when any marker exists: setup is done, whichever wizard did it.
    pub fn any(self) -> bool {
        self.telamon || self.atlas || self.plasma
    }

    /// True when every marker we write exists.
    pub fn all(self) -> bool {
        self.telamon && self.atlas && self.plasma
    }
}

/// A marker counts as there when it exists, and also when it cannot be
/// checked (`try_exists` fails: EACCES, SELinux, I/O error), so a stat
/// failure never reopens setup on a finished machine.
fn there(path: &Path) -> bool {
    path.try_exists().unwrap_or_else(|e| {
        log::error!("cannot check {}: {e}; counting it as there", path.display());
        true
    })
}

/// Which markers exist under `root` (fail-closed: one that cannot be checked
/// counts as there).
pub fn present(root: &Path) -> Present {
    Present {
        telamon: there(&telamon_path(root)),
        atlas: there(&atlas_path(root)),
        plasma: there(&plasma_path(root)),
    }
}

/// True when any marker exists, or cannot be checked: setup is done.
pub fn is_done(root: &Path) -> bool {
    present(root).any()
}

/// [`is_done`]; the name the root helper uses, to say it is fail-closed.
pub fn is_done_or_unknown(root: &Path) -> bool {
    is_done(root)
}

fn setup_text(finished: &str) -> String {
    format!(
        "[Setup]\nVersion=1\nFinished={finished}\nWizard={}\n",
        crate::VERSION
    )
}

/// Writes (replaces) `/etc/telamon/setup-done`, atomically, mode 0644. On a
/// machine Atlas Wizard set up, `Finished=` is the time its marker says.
///
/// # Errors
/// Any I/O error.
pub fn write_telamon(root: &Path, now: SystemTime) -> io::Result<()> {
    let finished = read_atlas(root)
        .ok()
        .flatten()
        .and_then(|m| m.finished)
        .filter(|f| f.len() <= 64 && f.chars().all(|c| c.is_ascii_graphic()))
        .unwrap_or_else(|| rfc3339_utc(now));
    write_atomic(&telamon_path(root), setup_text(&finished).as_bytes(), MODE)
}

/// Writes (replaces) `/etc/atlasos/setup-done`, atomically, mode 0644.
///
/// # Errors
/// Any I/O error.
pub fn write_atlas(root: &Path, now: SystemTime) -> io::Result<()> {
    write_atomic(
        &atlas_path(root),
        setup_text(&rfc3339_utc(now)).as_bytes(),
        MODE,
    )
}

/// Writes (replaces) `/etc/plasma-setup-done`, atomically, mode 0644.
///
/// # Errors
/// Any I/O error.
pub fn write_plasma(root: &Path, now: SystemTime) -> io::Result<()> {
    let text = format!(
        "Setup completed by telamon-wizard at {}\n",
        rfc3339_utc(now)
    );
    write_atomic(&plasma_path(root), text.as_bytes(), MODE)
}

/// Writes whichever marker is missing and leaves an existing one alone, so a
/// repeat after a crash is harmless. Telamon's is written first; Atlas
/// Wizard's and plasma-setup's follow (the image reads them until it moves to
/// Telamon's).
///
/// # Errors
/// Any I/O error; a marker already written stays.
pub fn write_missing(root: &Path, now: SystemTime) -> io::Result<()> {
    let p = present(root);
    if !p.telamon {
        write_telamon(root, now)?;
    }
    if !p.atlas {
        write_atlas(root, now)?;
    }
    if !p.plasma {
        write_plasma(root, now)?;
    }
    Ok(())
}

/// Reads `/etc/telamon/setup-done`. `Ok(None)` when it does not exist.
///
/// # Errors
/// A read error, or text that is not UTF-8.
pub fn read_telamon(root: &Path) -> io::Result<Option<Marker>> {
    read_marker(&telamon_path(root))
}

/// Reads `/etc/atlasos/setup-done`. `Ok(None)` when it does not exist.
///
/// # Errors
/// A read error, or text that is not UTF-8.
pub fn read_atlas(root: &Path) -> io::Result<Option<Marker>> {
    read_marker(&atlas_path(root))
}

fn read_marker(path: &Path) -> io::Result<Option<Marker>> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let ini = Ini::parse(&text);
    let get = |k| ini.get("Setup", k).map(String::from);
    Ok(Some(Marker {
        version: get("Version").and_then(|v| v.parse().ok()),
        finished: get("Finished"),
        wizard: get("Wizard"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, UNIX_EPOCH};

    fn t() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_791_201_600)
    }

    #[test]
    fn nothing_is_not_done() {
        let d = tempfile::tempdir().unwrap();
        assert!(!is_done(d.path()));
        assert_eq!(present(d.path()), Present::default());
        assert_eq!(read_atlas(d.path()).unwrap(), None);
        assert_eq!(read_telamon(d.path()).unwrap(), None);
    }

    #[test]
    fn telamon_marker_content_and_read_back() {
        let d = tempfile::tempdir().unwrap();
        write_telamon(d.path(), t()).unwrap();
        let text = fs::read_to_string(telamon_path(d.path())).unwrap();
        assert_eq!(
            text,
            format!(
                "[Setup]\nVersion=1\nFinished=2026-10-05T12:00:00Z\nWizard={}\n",
                crate::VERSION
            )
        );
        let m = read_telamon(d.path()).unwrap().unwrap();
        assert_eq!(m.finished.as_deref(), Some("2026-10-05T12:00:00Z"));
        assert!(is_done(d.path()));
        assert_eq!(
            present(d.path()),
            Present {
                telamon: true,
                atlas: false,
                plasma: false
            }
        );
    }

    /// A machine Atlas Wizard 0.1.x set up has /etc/atlasos/setup-done and
    /// /etc/plasma-setup-done: it is done, and the first boot of this wizard
    /// adds ours, with the time the machine was set up.
    #[test]
    fn a_machine_atlas_wizard_set_up_is_done_and_gets_ours() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("etc/atlasos")).unwrap();
        fs::write(
            atlas_path(d.path()),
            "[Setup]\nVersion=1\nFinished=2026-10-01T08:30:00Z\nWizard=0.1.1\n",
        )
        .unwrap();
        write_plasma(d.path(), t()).unwrap();
        assert!(is_done(d.path()) && is_done_or_unknown(d.path()));
        assert!(!present(d.path()).telamon && !present(d.path()).all());
        let old = fs::read(atlas_path(d.path())).unwrap();
        write_missing(d.path(), t()).unwrap();
        assert!(present(d.path()).all());
        assert_eq!(
            fs::read(atlas_path(d.path())).unwrap(),
            old,
            "theirs is left alone"
        );
        let m = read_telamon(d.path()).unwrap().unwrap();
        assert_eq!(m.finished.as_deref(), Some("2026-10-01T08:30:00Z"));
        assert_eq!(m.wizard.as_deref(), Some(crate::VERSION));
    }

    #[test]
    fn a_finished_time_that_is_not_a_time_is_not_copied() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("etc/atlasos")).unwrap();
        fs::write(
            atlas_path(d.path()),
            "[Setup]\nFinished=x\u{7}\ny\nWizard=0.1.1\n",
        )
        .unwrap();
        write_telamon(d.path(), t()).unwrap();
        let m = read_telamon(d.path()).unwrap().unwrap();
        assert_eq!(m.finished.as_deref(), Some("2026-10-05T12:00:00Z"));
    }

    #[test]
    fn atlas_wizards_marker_content_and_read_back() {
        let d = tempfile::tempdir().unwrap();
        write_atlas(d.path(), t()).unwrap();
        let text = fs::read_to_string(atlas_path(d.path())).unwrap();
        assert_eq!(
            text,
            format!(
                "[Setup]\nVersion=1\nFinished=2026-10-05T12:00:00Z\nWizard={}\n",
                crate::VERSION
            )
        );
        let m = read_atlas(d.path()).unwrap().unwrap();
        assert_eq!(m.version, Some(1));
        assert_eq!(m.finished.as_deref(), Some("2026-10-05T12:00:00Z"));
        assert_eq!(m.wizard.as_deref(), Some(crate::VERSION));
        assert!(is_done(d.path()));
    }

    #[test]
    fn plasma_marker_content() {
        let d = tempfile::tempdir().unwrap();
        write_plasma(d.path(), t()).unwrap();
        assert_eq!(
            fs::read_to_string(plasma_path(d.path())).unwrap(),
            "Setup completed by telamon-wizard at 2026-10-05T12:00:00Z\n"
        );
        assert!(is_done(d.path()));
        assert_eq!(
            present(d.path()),
            Present {
                telamon: false,
                atlas: false,
                plasma: true
            }
        );
    }

    #[test]
    fn modes_are_0644_and_no_temp_left() {
        let d = tempfile::tempdir().unwrap();
        write_missing(d.path(), t()).unwrap();
        for p in [
            telamon_path(d.path()),
            atlas_path(d.path()),
            plasma_path(d.path()),
        ] {
            assert_eq!(
                fs::metadata(&p).unwrap().permissions().mode() & 0o777,
                0o644
            );
        }
        for dir in [
            d.path().join("etc"),
            d.path().join("etc/telamon"),
            d.path().join("etc/atlasos"),
        ] {
            for e in fs::read_dir(dir).unwrap() {
                assert!(!e.unwrap().file_name().to_string_lossy().contains(".tmp-"));
            }
        }
    }

    #[test]
    fn write_missing_keeps_existing() {
        let d = tempfile::tempdir().unwrap();
        write_plasma(d.path(), t()).unwrap();
        let before = fs::read(plasma_path(d.path())).unwrap();
        write_missing(d.path(), t() + Duration::from_secs(99)).unwrap();
        assert_eq!(fs::read(plasma_path(d.path())).unwrap(), before);
        assert!(present(d.path()).all());
        write_missing(d.path(), t()).unwrap();
    }

    #[test]
    fn marker_that_cannot_be_checked_counts_as_there() {
        let d = tempfile::tempdir().unwrap();
        // /etc/telamon is a file: stat of etc/telamon/setup-done fails with
        // ENOTDIR, not NotFound
        fs::create_dir_all(d.path().join("etc")).unwrap();
        fs::write(d.path().join("etc/telamon"), b"").unwrap();
        let p = present(d.path());
        assert!(p.telamon && !p.atlas && !p.plasma);
        assert!(is_done(d.path()) && is_done_or_unknown(d.path()));
        // and so is Atlas Wizard's
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("etc")).unwrap();
        fs::write(d.path().join("etc/atlasos"), b"").unwrap();
        let p = present(d.path());
        assert!(!p.telamon && p.atlas && !p.plasma);
        assert!(is_done(d.path()) && is_done_or_unknown(d.path()));
    }

    #[test]
    fn pre_existing_plasma_marker_counts_as_done() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("etc")).unwrap();
        fs::write(plasma_path(d.path()), "").unwrap();
        assert!(is_done(d.path()));
    }

    #[test]
    fn failed_write_leaves_no_temp() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(telamon_path(d.path())).unwrap(); // a directory in the way
        assert!(write_telamon(d.path(), t()).is_err());
        let left: Vec<_> = fs::read_dir(d.path().join("etc/telamon"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(left.is_empty());
    }

    #[test]
    fn garbled_marker_reads_as_empty_fields() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("etc/telamon")).unwrap();
        fs::write(telamon_path(d.path()), "junk").unwrap();
        let m = read_telamon(d.path()).unwrap().unwrap();
        assert_eq!(
            m,
            Marker {
                version: None,
                finished: None,
                wizard: None
            }
        );
    }
}

#[cfg(test)]
mod props {
    use super::*;
    use proptest::prelude::*;
    use std::time::{Duration, UNIX_EPOCH};

    proptest! {
        /// A hostile `Finished=` in Atlas Wizard's marker can never put a
        /// second key, a line or a control character into ours.
        #[test]
        fn an_old_markers_text_cannot_inject_into_ours(finished in any::<String>()) {
            static RUNS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if !crate::budget::within(&RUNS, 100) {
                return Ok(());
            }
            let d = tempfile::tempdir().unwrap();
            fs::create_dir_all(d.path().join("etc/atlasos")).unwrap();
            fs::write(
                atlas_path(d.path()),
                format!("[Setup]\nVersion=1\nFinished={finished}\n"),
            )
            .unwrap();
            write_telamon(d.path(), UNIX_EPOCH + Duration::from_secs(1_791_201_600)).unwrap();
            let text = fs::read_to_string(telamon_path(d.path())).unwrap();
            prop_assert_eq!(text.lines().count(), 4, "{:?}", text);
            let m = read_telamon(d.path()).unwrap().unwrap();
            let f = m.finished.unwrap();
            prop_assert!(f.len() <= 64 && f.chars().all(|c| c.is_ascii_graphic()));
        }

        /// Any bytes in a marker file: reading never panics.
        #[test]
        fn any_marker_content_reads_or_errors(bytes in prop::collection::vec(any::<u8>(), 0..300)) {
            static RUNS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if !crate::budget::within(&RUNS, 100) {
                return Ok(());
            }
            let d = tempfile::tempdir().unwrap();
            fs::create_dir_all(d.path().join("etc/telamon")).unwrap();
            fs::write(telamon_path(d.path()), &bytes).unwrap();
            let _ = read_telamon(d.path());
            prop_assert!(is_done(d.path()));
        }
    }
}

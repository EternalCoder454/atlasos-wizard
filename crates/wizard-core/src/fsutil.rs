//! Atomic file writes: temp file in the same directory (created with
//! `O_EXCL`), fsync, rename, fsync of the directory.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// Writes `contents` to `path` atomically with the given permission `mode`
/// (applied exactly, independent of the umask). Missing parent directories
/// are created. A crash at any point leaves either the old file or the new
/// one, never a partial file; on failure the temp file is removed.
///
/// # Errors
/// Any I/O error, or `InvalidInput` when `path` has no file name.
pub fn write_atomic(path: &Path, contents: &[u8], mode: u32) -> io::Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    fs::create_dir_all(dir)?;

    let (tmp, mut file) = create_temp(dir, &name.to_string_lossy(), mode)?;
    let result = (|| {
        file.write_all(contents)?;
        // Set explicitly: the creation mode is masked by the umask.
        file.set_permissions(fs::Permissions::from_mode(mode))?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
        return result;
    }
    sync_dir(dir)
}

fn create_temp(dir: &Path, name: &str, mode: u32) -> io::Result<(std::path::PathBuf, File)> {
    let mut last = None;
    for _ in 0..16 {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = dir.join(format!(".{name}.tmp-{}-{n}", std::process::id()));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&tmp)
        {
            Ok(f) => return Ok((tmp, f)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("no temp name")))
}

/// Removes the temp files a crash left in `dir` (`.<name>.tmp-<pid>-<n>`,
/// from `write_atomic`) whose pid is not this process's. Only for a time
/// when no other process writes there (`prepare`, before the login screen).
/// Returns how many went; a missing directory is none.
///
/// # Errors
/// Any I/O error reading the directory or removing a leftover.
pub fn remove_stale_temps(dir: &Path) -> io::Result<usize> {
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let own = std::process::id().to_string();
    let mut n = 0;
    for entry in rd {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some((_, tail)) = name.strip_prefix('.').and_then(|r| r.rsplit_once(".tmp-")) else {
            continue;
        };
        let Some((pid, count)) = tail.split_once('-') else {
            continue;
        };
        let digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
        if !digits(pid) || !digits(count) || pid == own || !entry.file_type()?.is_file() {
            continue;
        }
        match fs::remove_file(entry.path()) {
            Ok(()) => n += 1,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    if n > 0 {
        sync_dir(dir)?;
    }
    Ok(n)
}

/// Flushes a directory's entries to disk (after a rename into it).
///
/// # Errors
/// Any I/O error opening or syncing the directory.
pub fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leftovers(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".tmp-"))
            .collect()
    }

    #[test]
    fn writes_with_mode_and_no_temp() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("sub/dir/f");
        write_atomic(&p, b"one", 0o644).unwrap();
        write_atomic(&p, b"two", 0o644).unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"two");
        assert_eq!(
            fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o644
        );
        assert!(leftovers(p.parent().unwrap()).is_empty());
    }

    #[test]
    fn mode_ignores_umask() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("f");
        write_atomic(&p, b"x", 0o600).unwrap();
        assert_eq!(
            fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn failure_cleans_temp_and_keeps_destination() {
        let d = tempfile::tempdir().unwrap();
        // A directory in the way makes the rename fail.
        let p = d.path().join("f");
        fs::create_dir(&p).unwrap();
        assert!(write_atomic(&p, b"x", 0o644).is_err());
        assert!(p.is_dir());
        assert!(leftovers(d.path()).is_empty());
    }

    #[test]
    fn stale_temps_go_and_nothing_else_does() {
        let d = tempfile::tempdir().unwrap();
        let own = format!(".state.json.tmp-{}-0", std::process::id());
        for n in [
            ".state.json.tmp-123-4",
            ".99-atlas-wizard.conf.tmp-7-0",
            &own,
            "state.json",
            ".hidden",
            ".x.tmp-12a-0",
            ".x.tmp--1",
            "y.tmp-1-1",
        ] {
            fs::write(d.path().join(n), "x").unwrap();
        }
        fs::create_dir(d.path().join(".dir.tmp-5-5")).unwrap();
        assert_eq!(remove_stale_temps(d.path()).unwrap(), 2);
        let mut left: Vec<_> = fs::read_dir(d.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        let mut want = vec![
            own.as_str(),
            "state.json",
            ".hidden",
            ".x.tmp-12a-0",
            ".x.tmp--1",
            "y.tmp-1-1",
            ".dir.tmp-5-5",
        ];
        want.sort_unstable();
        assert_eq!(left, want);
        assert_eq!(remove_stale_temps(&d.path().join("none")).unwrap(), 0);
    }

    #[test]
    fn no_file_name_is_an_error() {
        assert!(write_atomic(Path::new("/"), b"x", 0o644).is_err());
    }
}

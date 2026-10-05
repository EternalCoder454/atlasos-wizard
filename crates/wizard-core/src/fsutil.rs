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
    fn no_file_name_is_an_error() {
        assert!(write_atomic(Path::new("/"), b"x", 0o644).is_err());
    }
}

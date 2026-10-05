//! File helpers for the settings child: edit KDE-style ini files in place and
//! write files atomically without ever following a symlink at the final
//! component.

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Component, Path};

/// Largest config file read for an edit.
const MAX_READ: u64 = 1024 * 1024;

/// Reads a text file, refusing a symlink as the final component
/// (`O_NOFOLLOW`) and anything that is not a regular file. `Ok(None)` when it
/// does not exist.
///
/// # Errors
/// Any I/O error, a symlink, a non-regular file, a file over 1 MiB, or text
/// that is not UTF-8.
pub fn read_nofollow(path: &Path) -> io::Result<Option<String>> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    let mut buf = Vec::new();
    file.take(MAX_READ + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > MAX_READ {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "file too large"));
    }
    String::from_utf8(buf)
        .map(Some)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "not UTF-8"))
}

/// Writes `contents` to `path`: temp file beside it, fsync, rename, directory
/// fsync (see `wizard_core::fsutil::write_atomic`), after checking that the
/// final component is not a symlink and not a directory.
///
/// # Errors
/// Any I/O error, or a symlink or directory at `path`.
pub fn write_nofollow(path: &Path, contents: &[u8], mode: u32) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "refusing to write through a symlink",
            ));
        }
        Ok(m) if !m.is_file() => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a regular file",
            ));
        }
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    wizard_core::fsutil::write_atomic(path, contents, mode)
}

/// Creates `dir` and its missing parents below `base` with `mode`, one level
/// at a time; an existing level must be a real directory (not a symlink).
///
/// # Errors
/// Any I/O error, a symlink or file in the way, or a `dir` outside `base`.
pub fn ensure_dir_under(base: &Path, dir: &Path, mode: u32) -> io::Result<()> {
    let rel = dir
        .strip_prefix(base)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "directory outside its base"))?;
    let mut cur = base.to_path_buf();
    for c in rel.components() {
        let Component::Normal(name) = c else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "bad directory component",
            ));
        };
        cur.push(name);
        match fs::symlink_metadata(&cur) {
            Ok(m) if m.is_dir() => {}
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "a symlink or file is in the way of a directory",
                ));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                DirBuilder::new().mode(mode).create(&cur)?;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// The value of `key` in `[section]` of ini `text`, if present.
pub fn ini_get<'a>(text: &'a str, section: &str, key: &str) -> Option<&'a str> {
    let mut inside = false;
    for line in text.lines() {
        let t = line.trim();
        if let Some(name) = section_name(t) {
            inside = name == section;
        } else if inside
            && let Some((k, v)) = t.split_once('=')
            && k.trim_end() == key
        {
            return Some(v.trim());
        }
    }
    None
}

fn section_name(t: &str) -> Option<&str> {
    t.strip_prefix('[')?.strip_suffix(']')
}

/// Sets `key=value` in `[section]` of ini `text` (replacing the key's first
/// line, or adding it at the end of the section, or adding the section), and
/// leaves every other line as it was. `value` must be a single line.
///
/// # Errors
/// `InvalidInput` when `section`, `key` or `value` holds a line break or other
/// control character, or `key` holds `=`.
pub fn ini_set(text: &str, section: &str, key: &str, value: &str) -> io::Result<String> {
    let bad = |s: &str| s.chars().any(char::is_control);
    if bad(section) || bad(key) || bad(value) || key.contains('=') || key.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "bad ini section, key or value",
        ));
    }
    let new_line = format!("{key}={value}");
    let mut out: Vec<String> = Vec::new();
    let mut inside = false;
    let mut done = false;
    // where a new key goes: after the section's last non-blank line
    let mut insert_at: Option<usize> = None;
    for line in text.lines() {
        let t = line.trim();
        if let Some(name) = section_name(t) {
            inside = name == section;
            out.push(line.to_string());
            if inside && insert_at.is_none() {
                insert_at = Some(out.len());
            }
            continue;
        }
        if inside && !done && t.split_once('=').is_some_and(|(k, _)| k.trim_end() == key) {
            out.push(new_line.clone());
            done = true;
            continue;
        }
        out.push(line.to_string());
        if inside && !t.is_empty() {
            insert_at = Some(out.len());
        }
    }
    if !done {
        if let Some(i) = insert_at {
            out.insert(i, new_line);
        } else {
            if out.last().is_some_and(|l| !l.is_empty()) {
                out.push(String::new());
            }
            out.push(format!("[{section}]"));
            out.push(new_line);
        }
    }
    let mut s = out.join("\n");
    s.push('\n');
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn get_finds_keys_in_their_section_only() {
        let t = "[A]\nx=1\n[B]\nx = 2\ny=3\n";
        assert_eq!(ini_get(t, "A", "x"), Some("1"));
        assert_eq!(ini_get(t, "B", "x"), Some("2"));
        assert_eq!(ini_get(t, "B", "y"), Some("3"));
        assert_eq!(ini_get(t, "A", "y"), None);
        assert_eq!(ini_get(t, "C", "x"), None);
    }

    #[test]
    fn set_replaces_inserts_and_adds_sections() {
        let t = "# top\n[General]\nfont=a\n\n[WM]\nactiveFont=b\n";
        let r = ini_set(t, "General", "font", "z").unwrap();
        assert_eq!(r, "# top\n[General]\nfont=z\n\n[WM]\nactiveFont=b\n");
        let r = ini_set(t, "General", "menuFont", "m").unwrap();
        assert_eq!(
            r,
            "# top\n[General]\nfont=a\nmenuFont=m\n\n[WM]\nactiveFont=b\n"
        );
        let r = ini_set(t, "Other", "k", "v").unwrap();
        assert_eq!(
            r,
            "# top\n[General]\nfont=a\n\n[WM]\nactiveFont=b\n\n[Other]\nk=v\n"
        );
        assert_eq!(ini_set("", "S", "k", "v").unwrap(), "[S]\nk=v\n");
        // the last section, no trailing newline
        assert_eq!(
            ini_set("[S]\na=1", "S", "k", "v").unwrap(),
            "[S]\na=1\nk=v\n"
        );
    }

    #[test]
    fn set_is_idempotent_and_keeps_other_keys() {
        let t = "[S]\na=1\nk=old\nb=2\n";
        let once = ini_set(t, "S", "k", "v").unwrap();
        assert_eq!(once, "[S]\na=1\nk=v\nb=2\n");
        assert_eq!(ini_set(&once, "S", "k", "v").unwrap(), once);
    }

    #[test]
    fn set_refuses_line_breaks_and_bad_keys() {
        assert!(ini_set("", "S", "k", "a\nb").is_err());
        assert!(ini_set("", "S\n[X]", "k", "v").is_err());
        assert!(ini_set("", "S", "k=l", "v").is_err());
        assert!(ini_set("", "S", "", "v").is_err());
        assert!(ini_set("", "S", "k", "a\u{0}b").is_err());
    }

    #[test]
    fn read_and_write_refuse_symlinks() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("target");
        fs::write(&target, "secret").unwrap();
        let link = d.path().join("link");
        symlink(&target, &link).unwrap();
        assert!(read_nofollow(&link).is_err());
        assert!(write_nofollow(&link, b"x", 0o644).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "secret");
        assert_eq!(read_nofollow(&d.path().join("none")).unwrap(), None);
        assert_eq!(read_nofollow(&target).unwrap().as_deref(), Some("secret"));
        assert!(write_nofollow(d.path(), b"x", 0o644).is_err());
    }

    #[test]
    fn dirs_are_created_with_the_mode_and_symlinks_refused() {
        let d = tempfile::tempdir().unwrap();
        let deep = d.path().join(".config/atlas");
        ensure_dir_under(d.path(), &deep, 0o700).unwrap();
        ensure_dir_under(d.path(), &deep, 0o700).unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&d.path().join(".config")), 0o700);
        assert_eq!(mode(&deep), 0o700);
        let other = tempfile::tempdir().unwrap();
        symlink(other.path(), d.path().join("evil")).unwrap();
        assert!(ensure_dir_under(d.path(), &d.path().join("evil/x"), 0o700).is_err());
        assert!(ensure_dir_under(d.path(), Path::new("/elsewhere"), 0o700).is_err());
    }
}

//! Password rules, strength (libpwquality) and hashing (libxcrypt yescrypt).
//!
//! Passwords travel as bytes. Every buffer this module allocates for one is
//! zeroed when done. No error, `Debug` or `Display` output holds a password.
//! (libpwquality and libxcrypt keep their own internal copies; libxcrypt's
//! `crypt_data` area is zeroed here after use.)

// The FFI to two C libraries lives here and nowhere else.
#![allow(unsafe_code)]

use std::ffi::{CStr, c_char, c_int, c_ulong, c_void};
use std::ptr;
use std::sync::OnceLock;
use zeroize::{Zeroize, Zeroizing};

/// Shortest password, in characters (not bytes).
pub const MIN_CHARS: usize = 8;

/// Longest password libxcrypt accepts (512 with the terminating NUL).
pub const MAX_BYTES: usize = 511;

/// Passphrase size for the C functions' `crypt_data`.
const CRYPT_DATA_SIZE: usize = 32768;
const GENSALT_OUTPUT_SIZE: usize = 192;

#[link(name = "pwquality")]
unsafe extern "C" {
    fn pwquality_default_settings() -> *mut c_void;
    fn pwquality_free_settings(pwq: *mut c_void);
    fn pwquality_read_config(
        pwq: *mut c_void,
        cfgfile: *const c_char,
        aux: *mut *mut c_void,
    ) -> c_int;
    fn pwquality_check(
        pwq: *mut c_void,
        password: *const c_char,
        oldpassword: *const c_char,
        user: *const c_char,
        aux: *mut *mut c_void,
    ) -> c_int;
    fn pwquality_set_int_value(pwq: *mut c_void, setting: c_int, value: c_int) -> c_int;
    fn pwquality_set_str_value(pwq: *mut c_void, setting: c_int, value: *const c_char) -> c_int;
}

// From pwquality.h.
const PWQ_SETTING_DICT_PATH: c_int = 10;
const PWQ_SETTING_DICT_CHECK: c_int = 15;
const PWQ_ERROR_CRACKLIB_CHECK: c_int = -22;

/// A password no dictionary holds. When cracklib cannot read its dictionary
/// it refuses every password as a dictionary word, this one too; that is how
/// a missing dictionary is told apart from a real dictionary hit, with no
/// path and no (translated) message to compare.
const DICT_PROBE: &CStr = c"Wq7#zK2!vR9$mX4%";

/// Whether the system dictionary is missing, probed once per process (the
/// strength meter checks on every key press; a probe is a second full check).
/// The missing-dictionary error is logged with it, once.
static SYSTEM_DICT_MISSING: OnceLock<bool> = OnceLock::new();

#[link(name = "crypt")]
unsafe extern "C" {
    fn crypt_gensalt_rn(
        prefix: *const c_char,
        count: c_ulong,
        rbytes: *const c_char,
        nrbytes: c_int,
        output: *mut c_char,
        output_size: c_int,
    ) -> *mut c_char;
    fn crypt_rn(
        phrase: *const c_char,
        setting: *const c_char,
        data: *mut c_void,
        size: c_int,
    ) -> *mut c_char;
}

/// Strength of an accepted password, from libpwquality's 0-100 score.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Score(u8);

impl Score {
    /// The raw libpwquality score, 0 to 100.
    pub fn value(self) -> u8 {
        self.0
    }

    /// A step for a strength meter: 0 (weakest) to 4 (strongest).
    pub fn meter(self) -> u8 {
        match self.0 {
            0..=19 => 0,
            20..=39 => 1,
            40..=59 => 2,
            60..=79 => 3,
            _ => 4,
        }
    }
}

/// What libpwquality objected to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityIssue {
    /// Too close to the user name or full name.
    UserRelated,
    /// Found in the dictionary or a bad-words list.
    Dictionary,
    /// Too simple: palindrome, case changes only, rotated, too few
    /// character classes or digits and the like.
    TooSimple,
    /// Repeated or sequential characters.
    Repetitive,
    /// libpwquality's own length rule.
    TooShort,
    /// Anything else libpwquality reports.
    Other,
}

impl QualityIssue {
    fn from_code(code: c_int) -> QualityIssue {
        match code {
            -25 | -26 | -9 | -21 => QualityIssue::UserRelated,
            -22 | -28 => QualityIssue::Dictionary,
            -13..=-10 | -18..=-15 => QualityIssue::TooSimple,
            -19 | -27 | -29 => QualityIssue::Repetitive,
            -14 | -20 => QualityIssue::TooShort,
            _ => QualityIssue::Other,
        }
    }

    fn code(self) -> &'static str {
        match self {
            QualityIssue::UserRelated => "password-quality-user",
            QualityIssue::Dictionary => "password-quality-dictionary",
            QualityIssue::TooSimple => "password-quality-simple",
            QualityIssue::Repetitive => "password-quality-repetitive",
            QualityIssue::TooShort => "password-quality-short",
            QualityIssue::Other => "password-quality-other",
        }
    }
}

/// Why a password was refused or could not be checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordError {
    /// Fewer than 8 characters.
    TooShort,
    /// More than 511 bytes.
    TooLong,
    /// Not valid UTF-8.
    NotText,
    /// Holds a NUL byte, which C strings cannot carry.
    HasNul,
    /// Equals or contains the user name.
    ContainsUserName,
    /// Equals or contains a word of the full name.
    ContainsFullName,
    /// libpwquality refused it.
    Quality(QualityIssue),
    /// libpwquality could not run.
    Unavailable,
}

impl PasswordError {
    /// Stable identifier the GUI maps to translated text.
    pub fn code(self) -> &'static str {
        match self {
            PasswordError::TooShort => "password-too-short",
            PasswordError::TooLong => "password-too-long",
            PasswordError::NotText => "password-not-text",
            PasswordError::HasNul => "password-has-nul",
            PasswordError::ContainsUserName => "password-user-name",
            PasswordError::ContainsFullName => "password-full-name",
            PasswordError::Quality(q) => q.code(),
            PasswordError::Unavailable => "password-check-unavailable",
        }
    }
}

/// Why hashing failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashError {
    /// Holds a NUL byte.
    HasNul,
    /// More than 511 bytes.
    TooLong,
    /// libxcrypt failed (no entropy, unsupported method).
    Failed,
}

impl HashError {
    /// Stable identifier for logs and the GUI.
    pub fn code(self) -> &'static str {
        match self {
            HashError::HasNul => "hash-has-nul",
            HashError::TooLong => "hash-too-long",
            HashError::Failed => "hash-failed",
        }
    }
}

/// A NUL-terminated copy of `bytes`, zeroed on drop. `None` when `bytes`
/// holds a NUL.
fn c_buf(bytes: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
    if bytes.contains(&0) {
        return None;
    }
    let mut v = Vec::with_capacity(bytes.len() + 1);
    v.extend_from_slice(bytes);
    v.push(0);
    Some(Zeroizing::new(v))
}

/// Checks a candidate password for the account `user` / `full_name`.
///
/// Rules, in order: at least 8 characters; not equal to the user name and
/// not containing it (names of 3+ characters), nor any word of the full name
/// of 3+ characters, case-insensitively; then libpwquality with the system
/// configuration (dictionary included).
///
/// # Errors
/// The first rule broken, as a [`PasswordError`].
pub fn check(password: &[u8], user: &str, full_name: &str) -> Result<Score, PasswordError> {
    let text = std::str::from_utf8(password).map_err(|_| PasswordError::NotText)?;
    if password.len() > MAX_BYTES {
        return Err(PasswordError::TooLong);
    }
    if text.chars().count() < MIN_CHARS {
        return Err(PasswordError::TooShort);
    }
    if password.contains(&0) {
        return Err(PasswordError::HasNul);
    }
    related_to_names(text, user, full_name)?;
    pwquality(password, user)
}

/// The name rules of [`check`], on their own.
fn related_to_names(text: &str, user: &str, full_name: &str) -> Result<(), PasswordError> {
    let pw = Zeroizing::new(text.to_lowercase());
    let user = user.trim().to_lowercase();
    if !user.is_empty() && (*pw == user || (user.chars().count() >= 3 && pw.contains(&user))) {
        return Err(PasswordError::ContainsUserName);
    }
    let full = full_name.trim().to_lowercase();
    if !full.is_empty() && *pw == full {
        return Err(PasswordError::ContainsFullName);
    }
    for word in full.split(|c: char| !c.is_alphanumeric()) {
        if word.chars().count() >= 3 && pw.contains(word) {
            return Err(PasswordError::ContainsFullName);
        }
    }
    Ok(())
}

/// Frees libpwquality settings on drop.
struct Settings(*mut c_void);

impl Drop for Settings {
    fn drop(&mut self) {
        // SAFETY: the pointer came from pwquality_default_settings and is
        // freed exactly once.
        unsafe { pwquality_free_settings(self.0) };
    }
}

/// True when cracklib refuses a password no dictionary holds: it cannot read
/// its dictionary.
fn dict_missing(settings: &Settings) -> bool {
    // SAFETY: valid settings and a NUL-terminated probe.
    let probe = unsafe {
        pwquality_check(
            settings.0,
            DICT_PROBE.as_ptr(),
            ptr::null(),
            ptr::null(),
            ptr::null_mut(),
        )
    };
    probe == PWQ_ERROR_CRACKLIB_CHECK
}

fn pwquality(password: &[u8], user: &str) -> Result<Score, PasswordError> {
    pwquality_with(password, user, None)
}

/// [`pwquality`] with another dictionary path (tests).
fn pwquality_with(
    password: &[u8],
    user: &str,
    dict_path: Option<&CStr>,
) -> Result<Score, PasswordError> {
    let pw = c_buf(password).ok_or(PasswordError::HasNul)?;
    let user_c = c_buf(user.as_bytes());
    // SAFETY: default_settings returns a fresh object or NULL.
    let raw = unsafe { pwquality_default_settings() };
    if raw.is_null() {
        return Err(PasswordError::Unavailable);
    }
    let settings = Settings(raw);
    // A missing or bad system config leaves the built-in defaults, which is
    // still a real check; ignore its result on purpose.
    // SAFETY: valid settings, NULL path means the default file, NULL aux.
    let _ = unsafe { pwquality_read_config(settings.0, ptr::null(), ptr::null_mut()) };
    if let Some(path) = dict_path {
        // SAFETY: valid settings and a NUL-terminated string (copied).
        if unsafe { pwquality_set_str_value(settings.0, PWQ_SETTING_DICT_PATH, path.as_ptr()) } != 0
        {
            return Err(PasswordError::Unavailable);
        }
    }
    // A missing or unreadable dictionary would refuse every password, so no
    // account could be made: skip only the dictionary check then, and say so.
    let missing = match dict_path {
        None => *SYSTEM_DICT_MISSING.get_or_init(|| {
            let missing = dict_missing(&settings);
            if missing {
                log::error!(
                    "the password dictionary cannot be read (cracklib-dicts missing?); \
                     checking passwords without it"
                );
            }
            missing
        }),
        Some(_) => dict_missing(&settings),
    };
    if missing {
        // SAFETY: valid settings.
        if unsafe { pwquality_set_int_value(settings.0, PWQ_SETTING_DICT_CHECK, 0) } != 0 {
            return Err(PasswordError::Unavailable);
        }
    }
    let user_ptr = user_c.as_ref().map_or(ptr::null(), |u| u.as_ptr().cast());
    // SAFETY: all strings are NUL-terminated and outlive the call.
    let rc = unsafe {
        pwquality_check(
            settings.0,
            pw.as_ptr().cast(),
            ptr::null(),
            user_ptr,
            ptr::null_mut(),
        )
    };
    if rc < 0 {
        return Err(PasswordError::Quality(QualityIssue::from_code(rc)));
    }
    Ok(Score(rc.min(100) as u8))
}

/// Zeroed, aligned scratch area for `crypt_rn`.
#[repr(C, align(16))]
struct CryptData([u8; CRYPT_DATA_SIZE]);

impl Drop for CryptData {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

fn crypt_with(password: &[u8], setting: &std::ffi::CStr) -> Option<String> {
    let pw = c_buf(password)?;
    let mut data = Box::new(CryptData([0; CRYPT_DATA_SIZE]));
    // SAFETY: strings are NUL-terminated; data is CRYPT_DATA_SIZE bytes, the
    // size of struct crypt_data, and zero-initialised as required.
    let out = unsafe {
        crypt_rn(
            pw.as_ptr().cast(),
            setting.as_ptr(),
            (&raw mut *data).cast(),
            CRYPT_DATA_SIZE as c_int,
        )
    };
    if out.is_null() {
        return None;
    }
    // SAFETY: crypt_rn returned a NUL-terminated string inside `data`.
    let s = unsafe { std::ffi::CStr::from_ptr(out) }
        .to_str()
        .ok()?
        .to_owned();
    // On failure crypt_rn returns "*0" / "*1" style strings.
    if s.starts_with('*') { None } else { Some(s) }
}

/// Hashes a password with yescrypt (`$y$...`) and a fresh random salt.
///
/// # Errors
/// [`HashError`] for a password with a NUL or over 511 bytes, or when
/// libxcrypt fails.
pub fn hash(password: &[u8]) -> Result<String, HashError> {
    if password.contains(&0) {
        return Err(HashError::HasNul);
    }
    if password.len() > MAX_BYTES {
        return Err(HashError::TooLong);
    }
    let mut salt = [0 as c_char; GENSALT_OUTPUT_SIZE];
    // SAFETY: prefix is NUL-terminated; output buffer size is passed; NULL
    // rbytes asks libxcrypt for system randomness.
    let rc = unsafe {
        crypt_gensalt_rn(
            c"$y$".as_ptr(),
            0,
            ptr::null(),
            0,
            salt.as_mut_ptr(),
            GENSALT_OUTPUT_SIZE as c_int,
        )
    };
    if rc.is_null() {
        return Err(HashError::Failed);
    }
    // SAFETY: gensalt wrote a NUL-terminated string into `salt`.
    let setting = unsafe { std::ffi::CStr::from_ptr(salt.as_ptr()) };
    crypt_with(password, setting).ok_or(HashError::Failed)
}

/// True when `password` hashes to `hash` (any method libxcrypt knows).
/// Anything unusable (NUL in the password, a bad hash) is simply false.
pub fn verify(password: &[u8], hash: &str) -> bool {
    if password.len() > MAX_BYTES {
        return false;
    }
    let Ok(setting) = std::ffi::CString::new(hash) else {
        return false;
    };
    match crypt_with(password, &setting) {
        Some(got) => constant_time_eq(got.as_bytes(), hash.as_bytes()),
        None => false,
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &[u8] = b"violet-Harbor-93-lantern";

    #[test]
    fn too_short_counts_chars_not_bytes() {
        assert_eq!(check(b"abc", "u", ""), Err(PasswordError::TooShort));
        // 7 two-byte characters: 14 bytes, still too short.
        assert_eq!(
            check("ééééééé".as_bytes(), "u", ""),
            Err(PasswordError::TooShort)
        );
        // 8 characters is long enough for our rule; pwquality may still object.
        assert_ne!(
            check("éàüöäßñç".as_bytes(), "u", ""),
            Err(PasswordError::TooShort)
        );
    }

    #[test]
    fn encoding_and_nul_and_length() {
        assert_eq!(check(&[0xff; 12], "u", ""), Err(PasswordError::NotText));
        assert_eq!(check(b"abcdefgh\0ijk", "u", ""), Err(PasswordError::HasNul));
        assert_eq!(check(&[b'a'; 600], "u", ""), Err(PasswordError::TooLong));
    }

    #[test]
    fn user_and_full_name_words() {
        assert_eq!(
            check(b"AdaLovelace-1815", "ada", ""),
            Err(PasswordError::ContainsUserName)
        );
        assert_eq!(
            check(b"my-ADA-password", "ada", ""),
            Err(PasswordError::ContainsUserName)
        );
        assert_eq!(
            check(b"zzzzLOVELACEzzzz", "x", "Ada Lovelace"),
            Err(PasswordError::ContainsFullName)
        );
        assert_eq!(
            check(b"ada lovelace", "x", "Ada Lovelace"),
            Err(PasswordError::ContainsFullName)
        );
        // A short user name only blocks an exact match.
        assert_ne!(check(GOOD, "li", ""), Err(PasswordError::ContainsUserName));
    }

    #[test]
    fn dictionary_word_refused() {
        match check(b"password123", "bob", "Bob") {
            Err(PasswordError::Quality(_)) => {}
            other => panic!("expected a quality error, got {other:?}"),
        }
    }

    #[test]
    fn dictionary_probe_passes_a_working_dictionary() {
        // The image's dictionary (cracklib-dicts) is installed: the probe
        // must not read as a dictionary word, or the check would always be
        // skipped.
        assert_ne!(
            pwquality_with(DICT_PROBE.to_bytes(), "bob", None).err(),
            Some(PasswordError::Quality(QualityIssue::Dictionary))
        );
    }

    #[test]
    fn missing_dictionary_skips_only_the_dictionary_check() {
        let gone = Some(c"/nonexistent/pw_dict");
        // Accepted, not refused as a dictionary word.
        assert!(pwquality_with(GOOD, "bob", gone).is_ok());
        // The other rules still apply.
        match pwquality_with(b"abcdcba-abcdcba", "bob", gone) {
            Err(PasswordError::Quality(q)) => assert_ne!(q, QualityIssue::Dictionary),
            other => panic!("expected a non-dictionary quality error, got {other:?}"),
        }
        // A dictionary word passes the dictionary (none to look in), and
        // the real dictionary still refuses it.
        assert_ne!(
            pwquality_with(b"password123", "bob", gone).err(),
            Some(PasswordError::Quality(QualityIssue::Dictionary))
        );
        assert_eq!(
            pwquality_with(b"password123", "bob", None).err(),
            Some(PasswordError::Quality(QualityIssue::Dictionary))
        );
    }

    #[test]
    fn strong_password_accepted_with_meter() {
        let s = check(GOOD, "bob", "Bob Builder").unwrap();
        assert!(s.value() <= 100);
        assert!(s.meter() <= 4);
        assert!(s.meter() >= 2, "score {}", s.value());
    }

    #[test]
    fn meter_steps() {
        let m = |v| Score(v).meter();
        assert_eq!(
            [m(0), m(19), m(20), m(40), m(60), m(79), m(80), m(100)],
            [0, 0, 1, 2, 3, 3, 4, 4]
        );
    }

    #[test]
    fn hash_and_verify() {
        let h = hash(GOOD).unwrap();
        assert!(h.starts_with("$y$"), "{h}");
        assert!(verify(GOOD, &h));
        assert!(!verify(b"violet-Harbor-93-lanterm", &h));
        assert!(!verify(GOOD, "$y$garbage"));
        assert!(!verify(GOOD, ""));
        assert!(!verify(b"has\0nul", &h));
        // Fresh salt each time.
        assert_ne!(h, hash(GOOD).unwrap());
    }

    #[test]
    fn verify_reads_other_methods() {
        let h = crypt_with(b"password", c"$6$saltsalt$").unwrap();
        assert!(h.starts_with("$6$saltsalt$"));
        assert!(verify(b"password", &h));
        assert!(!verify(b"passwore", &h));
    }

    #[test]
    fn hash_rejects_bad_input() {
        assert_eq!(hash(b"a\0b"), Err(HashError::HasNul));
        assert_eq!(hash(&[b'a'; 512]), Err(HashError::TooLong));
        assert!(hash(&[b'a'; 511]).is_ok());
    }

    #[test]
    fn no_password_in_any_error_or_debug() {
        let secret = "Zq9-unique-secret-w0rd";
        let cases: Vec<String> = vec![
            format!("{:?}", check(secret.as_bytes(), &secret[..6], "")),
            format!("{:?}", check(secret.as_bytes(), "x", secret)),
            format!("{:?}", check(secret.as_bytes(), "x", "")),
            format!("{:?}", check(b"Zq9-\0secret-w0rd", "x", "")),
            format!("{:?}", check(&[b'Z'; 700], "x", "")),
            format!("{:?}", hash(b"Zq9\0-unique-secret-w0rd")),
            format!("{:?}", hash(secret.as_bytes()).map(|_| ())),
            format!("{:?}", Score(50)),
            format!("{:?}", PasswordError::Quality(QualityIssue::Dictionary)),
        ];
        let long = String::from_utf8(vec![b'Z'; 700]).unwrap();
        for c in cases {
            assert!(!c.contains(secret), "{c}");
            assert!(!c.contains(&long), "{c}");
        }
        let h = hash(secret.as_bytes()).unwrap();
        assert!(!h.contains(secret));
    }

    #[test]
    fn codes_distinct() {
        let all = [
            PasswordError::TooShort,
            PasswordError::TooLong,
            PasswordError::NotText,
            PasswordError::HasNul,
            PasswordError::ContainsUserName,
            PasswordError::ContainsFullName,
            PasswordError::Quality(QualityIssue::Dictionary),
            PasswordError::Quality(QualityIssue::TooSimple),
            PasswordError::Unavailable,
        ];
        let mut c: Vec<_> = all.iter().map(|e| e.code()).collect();
        c.sort();
        c.dedup();
        assert_eq!(c.len(), all.len());
    }
}

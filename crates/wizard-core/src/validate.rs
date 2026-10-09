//! User name and full name rules (DESIGN.md, Pages, Account).

use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

/// Names that may never be chosen for the new account.
pub const RESERVED_NAMES: &[&str] = &[
    "root",
    "bin",
    "daemon",
    "adm",
    "lp",
    "sync",
    "shutdown",
    "halt",
    "mail",
    "operator",
    "games",
    "ftp",
    "nobody",
    "telamon-setup",
    // Atlas Wizard's setup user, which a machine it set up still has.
    "atlas-setup",
    "plasma-setup",
    "plasmalogin",
    "sddm",
    "gdm",
    // Accounts and groups that packages create (now, or in a later image
    // update, when a sysusers entry of that name would collide with the
    // person's account) and whose names must stay the system's.
    "sys",
    "uucp",
    "news",
    "man",
    "proxy",
    "backup",
    "list",
    "irc",
    "www-data",
    "dbus",
    "polkitd",
    "sshd",
    "avahi",
    "chrony",
    "tss",
    "rtkit",
    "colord",
    "geoclue",
    "flatpak",
    "pipewire",
    "cups",
    "users",
    "sudo",
    "nogroup",
    "nfsnobody",
];

/// Reserved prefix: every `systemd-*` name is refused.
pub const RESERVED_PREFIX: &str = "systemd-";

/// Longest user name, in bytes.
pub const USER_NAME_MAX: usize = 32;

/// Longest full name, in bytes.
pub const FULL_NAME_MAX: usize = 255;

/// Why a user name was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameError {
    /// The name is empty.
    Empty,
    /// The name is longer than 32 bytes.
    TooLong,
    /// The first character is not `a-z` or `_`.
    BadStart,
    /// A later character is not `a-z`, `0-9`, `_` or `-`.
    BadChar,
    /// The name is reserved for the system.
    Reserved,
    /// The name is already in the passwd or group file.
    Exists,
    /// The passwd or group file could not be read.
    Unreadable,
}

impl NameError {
    /// Stable identifier the GUI maps to translated text.
    pub fn code(self) -> &'static str {
        match self {
            NameError::Empty => "name-empty",
            NameError::TooLong => "name-too-long",
            NameError::BadStart => "name-bad-start",
            NameError::BadChar => "name-bad-char",
            NameError::Reserved => "name-reserved",
            NameError::Exists => "name-exists",
            NameError::Unreadable => "name-unreadable",
        }
    }
}

/// Checks a user name against `^[a-z_][a-z0-9_-]{0,31}$` and the reserved
/// list. Does not look at passwd or group; see [`user_name_available`].
///
/// # Errors
/// The first rule the name breaks.
pub fn user_name(name: &str) -> Result<(), NameError> {
    let bytes = name.as_bytes();
    let Some(&first) = bytes.first() else {
        return Err(NameError::Empty);
    };
    if bytes.len() > USER_NAME_MAX {
        return Err(NameError::TooLong);
    }
    if !(first.is_ascii_lowercase() || first == b'_') {
        return Err(NameError::BadStart);
    }
    if !bytes
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_' || *b == b'-')
    {
        return Err(NameError::BadChar);
    }
    if RESERVED_NAMES.contains(&name) || name.starts_with(RESERVED_PREFIX) {
        return Err(NameError::Reserved);
    }
    Ok(())
}

/// True when `name` is the first field of any line of a passwd- or
/// group-format `reader`.
///
/// # Errors
/// Any read error.
pub fn name_listed<R: BufRead>(name: &str, reader: R) -> io::Result<bool> {
    for line in reader.split(b'\n') {
        let line = line?;
        let first = line.split(|b| *b == b':').next().unwrap_or(&[]);
        if first == name.as_bytes() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// [`user_name`] plus "not in the passwd or group file". A missing file
/// counts as empty.
///
/// # Errors
/// A rule the name breaks, [`NameError::Exists`], or
/// [`NameError::Unreadable`] when a file exists but cannot be read.
pub fn user_name_available(name: &str, passwd: &Path, group: &Path) -> Result<(), NameError> {
    user_name(name)?;
    for path in [passwd, group] {
        match File::open(path) {
            Ok(f) => match name_listed(name, BufReader::new(f)) {
                Ok(true) => return Err(NameError::Exists),
                Ok(false) => {}
                Err(_) => return Err(NameError::Unreadable),
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(NameError::Unreadable),
        }
    }
    Ok(())
}

/// Why a full name was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FullNameError {
    /// Longer than 255 bytes.
    TooLong,
    /// Holds `:`, `,` or `=` (they break passwd and the GECOS field).
    BadChar,
    /// Holds a newline or another control character, or an invisible
    /// format character that can disguise the name (bidi controls,
    /// zero-width space, BOM, tags).
    Control,
}

impl FullNameError {
    /// Stable identifier the GUI maps to translated text.
    pub fn code(self) -> &'static str {
        match self {
            FullNameError::TooLong => "full-name-too-long",
            FullNameError::BadChar => "full-name-bad-char",
            FullNameError::Control => "full-name-control",
        }
    }
}

/// Validates a full name and returns it trimmed. The empty string is fine.
///
/// # Errors
/// The first rule the (trimmed) name breaks.
pub fn full_name(name: &str) -> Result<&str, FullNameError> {
    let name = name.trim();
    if name.len() > FULL_NAME_MAX {
        return Err(FullNameError::TooLong);
    }
    for c in name.chars() {
        if c.is_control() || c == '\u{2028}' || c == '\u{2029}' || disguising(c) {
            return Err(FullNameError::Control);
        }
        if matches!(c, ':' | ',' | '=') {
            return Err(FullNameError::BadChar);
        }
    }
    Ok(name)
}

/// Format characters (Unicode Cf) that only change how a name looks: bidi
/// controls (a name shown backwards on the login screen), zero-width and
/// other invisible ones. ZWJ and ZWNJ (U+200C, U+200D) stay: names in
/// Persian, the Indic scripts and emoji sequences need them.
fn disguising(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{061C}' | '\u{180E}' | '\u{200B}' | '\u{200E}' | '\u{200F}'
        | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206F}'
        | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}' | '\u{E0000}'..='\u{E007F}')
}

/// Proposes a user name from a full name: the first word that leaves
/// anything, lowercased, accents folded to ASCII, everything else dropped,
/// at most 32 bytes. Returns the empty string when nothing valid results
/// (including reserved names).
pub fn derive_user_name(full_name: &str) -> String {
    for word in full_name.split_whitespace() {
        let mut out = String::new();
        for c in word.chars().flat_map(char::to_lowercase) {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                out.push(c);
            } else if let Some(s) = fold(c) {
                out.push_str(s);
            }
        }
        let trimmed = out.trim_start_matches(|c: char| c.is_ascii_digit() || c == '-');
        let mut name: String = trimmed.chars().take(USER_NAME_MAX).collect();
        while name.ends_with('-') {
            name.pop();
        }
        if !name.is_empty() {
            return if user_name(&name).is_ok() {
                name
            } else {
                String::new()
            };
        }
    }
    String::new()
}

/// ASCII replacement for a lowercase Latin letter with a diacritic.
fn fold(c: char) -> Option<&'static str> {
    const GROUPS: &[(&str, &str)] = &[
        ("àáâãäåāăą", "a"),
        ("çćĉċč", "c"),
        ("ďđð", "d"),
        ("èéêëēĕėęě", "e"),
        ("ĝğġģ", "g"),
        ("ĥħ", "h"),
        ("ìíîïĩīĭįı", "i"),
        ("ĵ", "j"),
        ("ķ", "k"),
        ("ĺļľŀł", "l"),
        ("ñńņňŉ", "n"),
        ("òóôõöøōŏő", "o"),
        ("ŕŗř", "r"),
        ("śŝşš", "s"),
        ("ţťŧ", "t"),
        ("ùúûüũūŭůűų", "u"),
        ("ŵ", "w"),
        ("ýÿŷ", "y"),
        ("źżž", "z"),
    ];
    match c {
        'ß' => return Some("ss"),
        'æ' => return Some("ae"),
        'œ' => return Some("oe"),
        'þ' => return Some("th"),
        _ => {}
    }
    GROUPS
        .iter()
        .find(|(chars, _)| chars.contains(c))
        .map(|(_, ascii)| *ascii)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn user_name_rules() {
        assert_eq!(user_name(""), Err(NameError::Empty));
        assert_eq!(user_name("ada"), Ok(()));
        assert_eq!(user_name("_x-1"), Ok(()));
        assert_eq!(user_name(&"a".repeat(32)), Ok(()));
        assert_eq!(user_name(&"a".repeat(33)), Err(NameError::TooLong));
        assert_eq!(user_name("1ada"), Err(NameError::BadStart));
        assert_eq!(user_name("-ada"), Err(NameError::BadStart));
        assert_eq!(user_name("Ada"), Err(NameError::BadStart));
        assert_eq!(user_name("adA"), Err(NameError::BadChar));
        assert_eq!(user_name("a.b"), Err(NameError::BadChar));
        assert_eq!(user_name("a b"), Err(NameError::BadChar));
        assert_eq!(user_name("ada\n"), Err(NameError::BadChar));
        assert_eq!(user_name("adé"), Err(NameError::BadChar));
    }

    #[test]
    fn reserved_names() {
        for n in RESERVED_NAMES {
            assert_eq!(user_name(n), Err(NameError::Reserved), "{n}");
        }
        assert_eq!(user_name("systemd-foo"), Err(NameError::Reserved));
        assert_eq!(user_name("systemd"), Ok(()));
        assert_eq!(user_name("rootx"), Ok(()));
    }

    #[test]
    fn codes_are_unique() {
        let all = [
            NameError::Empty,
            NameError::TooLong,
            NameError::BadStart,
            NameError::BadChar,
            NameError::Reserved,
            NameError::Exists,
            NameError::Unreadable,
        ];
        let mut codes: Vec<_> = all.iter().map(|e| e.code()).collect();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), all.len());
    }

    #[test]
    fn exists_in_passwd_and_group() {
        let d = tempfile::tempdir().unwrap();
        let passwd = d.path().join("passwd");
        let group = d.path().join("group");
        fs::write(
            &passwd,
            "root:x:0:0::/root:/bin/bash\nada:x:1000:1000::/home/ada:/bin/bash\n",
        )
        .unwrap();
        fs::write(&group, "wheel:x:10:ada\nstaff:x:50:\n").unwrap();
        assert_eq!(
            user_name_available("ada", &passwd, &group),
            Err(NameError::Exists)
        );
        assert_eq!(
            user_name_available("wheel", &passwd, &group),
            Err(NameError::Exists)
        );
        assert_eq!(
            user_name_available("staff", &passwd, &group),
            Err(NameError::Exists)
        );
        assert_eq!(user_name_available("bob", &passwd, &group), Ok(()));
        // Only the first field counts, not members or other fields.
        assert_eq!(user_name_available("x", &passwd, &group), Ok(()));
        assert_eq!(
            user_name_available("Bad", &passwd, &group),
            Err(NameError::BadStart)
        );
        let missing = d.path().join("nope");
        assert_eq!(user_name_available("bob", &missing, &missing), Ok(()));
        // A directory cannot be read as a file.
        assert_eq!(
            user_name_available("bob", d.path(), &group),
            Err(NameError::Unreadable)
        );
    }

    #[test]
    fn name_listed_reader() {
        assert!(name_listed("a", &b"a:x\nb:x"[..]).unwrap());
        assert!(!name_listed("c", &b"a:x\nb:x"[..]).unwrap());
        assert!(!name_listed("a", &b""[..]).unwrap());
    }

    #[test]
    fn full_name_rules() {
        assert_eq!(full_name(""), Ok(""));
        assert_eq!(full_name("  Ada Lovelace \t"), Ok("Ada Lovelace"));
        assert_eq!(full_name("Zoë Ünal"), Ok("Zoë Ünal"));
        assert_eq!(full_name("a:b"), Err(FullNameError::BadChar));
        assert_eq!(full_name("a,b"), Err(FullNameError::BadChar));
        assert_eq!(full_name("a=b"), Err(FullNameError::BadChar));
        assert_eq!(full_name("a\nb"), Err(FullNameError::Control));
        assert_eq!(full_name("a\tb"), Err(FullNameError::Control));
        assert_eq!(full_name("a\u{7f}b"), Err(FullNameError::Control));
        assert_eq!(full_name("a\u{0}b"), Err(FullNameError::Control));
        for c in [
            '\u{202E}',
            '\u{2066}',
            '\u{200B}',
            '\u{FEFF}',
            '\u{E0041}',
            '\u{00AD}',
        ] {
            assert_eq!(
                full_name(&format!("Ada{c}x")),
                Err(FullNameError::Control),
                "{c:?}"
            );
        }
        // joiners are part of real names
        assert_eq!(
            full_name("\u{0645}\u{06CC}\u{200C}\u{0634}\u{0648}\u{062F}"),
            Ok("\u{0645}\u{06CC}\u{200C}\u{0634}\u{0648}\u{062F}")
        );
        assert!(full_name("\u{1F469}\u{200D}\u{1F4BB}").is_ok());
        assert_eq!(full_name(&"a".repeat(255)), Ok("a".repeat(255).as_str()));
        assert_eq!(full_name(&"a".repeat(256)), Err(FullNameError::TooLong));
        // Bytes, not chars: 128 two-byte chars is 256 bytes.
        assert_eq!(full_name(&"é".repeat(128)), Err(FullNameError::TooLong));
        assert_eq!(full_name(&"é".repeat(127)).map(str::len), Ok(254));
    }

    #[test]
    fn derive() {
        assert_eq!(derive_user_name("Ada Lovelace"), "ada");
        assert_eq!(derive_user_name("  José Ángel"), "jose");
        assert_eq!(derive_user_name("Zoë"), "zoe");
        assert_eq!(derive_user_name("Müller"), "muller");
        assert_eq!(derive_user_name("Straße"), "strasse");
        assert_eq!(derive_user_name("Łukasz Ø"), "lukasz");
        assert_eq!(derive_user_name("O'Brien"), "obrien");
        assert_eq!(derive_user_name("张伟 Wei"), "wei");
        assert_eq!(derive_user_name("张伟"), "");
        assert_eq!(derive_user_name(""), "");
        assert_eq!(derive_user_name("   "), "");
        assert_eq!(derive_user_name("42 Ada"), "ada");
        assert_eq!(derive_user_name("root"), "");
        assert_eq!(derive_user_name("Systemd-x"), "");
        assert_eq!(derive_user_name(&"a".repeat(40)), "a".repeat(32));
        assert_eq!(derive_user_name("Anne-Marie"), "anne-marie");
    }

    #[test]
    fn derived_names_are_always_valid_or_empty() {
        for s in [
            "Ünïcödé Nämé",
            "x",
            "-",
            "_",
            "9",
            "Ａｄａ",
            "İstanbul",
            "a\u{301}da",
        ] {
            let n = derive_user_name(s);
            assert!(n.is_empty() || user_name(&n).is_ok(), "{s} -> {n}");
        }
    }
}

#[cfg(test)]
mod props {
    use super::*;
    use proptest::prelude::*;

    /// Characters that have broken parsers before: separators, controls,
    /// bidi and zero-width, joiners, combining marks, the last code point.
    fn nasty_char() -> impl Strategy<Value = char> {
        prop_oneof![
            4 => any::<char>(),
            4 => prop::sample::select(vec![
                ':', ',', '=', '\n', '\r', '\t', '\0', '/', '\\', '.', '-', '_', ' ', '$',
                '\u{7f}', '\u{85}', '\u{2028}', '\u{202e}', '\u{2066}', '\u{200b}', '\u{200d}',
                '\u{feff}', '\u{e0041}', '\u{301}', '\u{10ffff}', 'a', 'z', '0', '9',
            ]),
        ]
    }

    fn nasty_string(max: usize) -> impl Strategy<Value = String> {
        prop::collection::vec(nasty_char(), 0..max).prop_map(|v| v.into_iter().collect())
    }

    proptest! {
        /// Never panics, and an accepted name has the safe shape: 1 to 32
        /// bytes of `[a-z0-9_-]`, not starting with a digit or `-`, so it
        /// cannot be an option, a path, a number or a second field.
        #[test]
        fn user_name_accepts_only_the_safe_shape(s in nasty_string(48)) {
            if user_name(&s).is_ok() {
                prop_assert!((1..=USER_NAME_MAX).contains(&s.len()));
                let b = s.as_bytes();
                prop_assert!(b[0].is_ascii_lowercase() || b[0] == b'_');
                prop_assert!(b.iter().all(|c| c.is_ascii_lowercase()
                    || c.is_ascii_digit() || *c == b'_' || *c == b'-'));
                prop_assert!(!RESERVED_NAMES.contains(&s.as_str()));
                prop_assert!(!s.starts_with(RESERVED_PREFIX));
            }
        }

        /// Names made only of allowed characters are accepted unless too
        /// long, reserved, or badly started (nothing allowed is refused).
        #[test]
        fn user_name_refuses_nothing_it_should_accept(s in "[a-z_][a-z0-9_-]{0,31}") {
            let reserved = RESERVED_NAMES.contains(&s.as_str()) || s.starts_with(RESERVED_PREFIX);
            prop_assert_eq!(user_name(&s).is_ok(), !reserved);
        }

        /// Never panics on any text; what it accepts is trimmed, within
        /// 255 bytes and free of every character that could end the field,
        /// start another or disguise the name; accepting it again is the
        /// same answer.
        #[test]
        fn full_name_accepts_only_the_safe_shape(s in nasty_string(300)) {
            if let Ok(n) = full_name(&s) {
                prop_assert!(n.len() <= FULL_NAME_MAX);
                prop_assert_eq!(n, n.trim());
                let clean = n.chars().all(|c| !c.is_control()
                    && !matches!(c, ':' | ',' | '=' | '\u{2028}' | '\u{2029}')
                    && !disguising(c));
                prop_assert!(clean);
                prop_assert_eq!(full_name(n), Ok(n));
            }
        }

        /// A proposal is empty or a name `user_name` accepts.
        #[test]
        fn derived_names_are_empty_or_valid(s in nasty_string(80)) {
            let n = derive_user_name(&s);
            prop_assert!(n.is_empty() || user_name(&n).is_ok(), "{s:?} -> {n:?}");
        }

        /// The passwd/group reader never panics on any bytes.
        #[test]
        fn name_listed_never_panics(
            bytes in prop::collection::vec(any::<u8>(), 0..400),
            name in nasty_string(12),
        ) {
            let _ = name_listed(&name, &bytes[..]);
        }
    }

    #[test]
    fn accounts_that_packages_create_are_reserved() {
        for n in [
            "sshd",
            "dbus",
            "polkitd",
            "sys",
            "sudo",
            "www-data",
            "avahi",
            "chrony",
            "tss",
            "rtkit",
            "pipewire",
            "flatpak",
            "users",
            "nogroup",
            "root",
            "nobody",
            "telamon-setup",
            "atlas-setup",
        ] {
            assert_eq!(user_name(n), Err(NameError::Reserved), "{n}");
        }
        // and a name that only starts like one is a person's
        for n in ["sshd2", "dbus-x", "mary", "sysadmin"] {
            assert_eq!(user_name(n), Ok(()), "{n}");
        }
    }

    #[test]
    fn nasty_names() {
        let big = "a".repeat(10 * 1024 * 1024);
        let cases = [
            "ada\n",
            "ada\nroot",
            "ada\0",
            "\0",
            "ada:x:0:0",
            "..",
            "../etc",
            "a/b",
            "-rf",
            "--root=/",
            "-",
            "ada ",
            " ada",
            "ADA",
            "ada\u{202e}",
            "аda", // a Cyrillic а
            "ada$",
            "123",
            "0",
            "root",
            "ROOT",
            "root ",
            "systemd-network",
            big.as_str(),
        ];
        for c in cases {
            assert!(user_name(c).is_err(), "{:?}", &c[..c.len().min(20)]);
        }
    }

    #[test]
    fn nasty_full_names() {
        for c in [
            "Ada\nroot::0:0",
            "Ada\0",
            "root:x:0:0:Ada",
            "Ada,,,,",
            "role=admin",
            "Ada \u{202e}Lovelace",
            "Ada\u{2028}Lovelace",
            "Ada\u{85}Lovelace",
            "Ada\u{e0041}",
        ] {
            assert!(full_name(c).is_err(), "{c:?}");
        }
        let big = "é".repeat(10 * 1024 * 1024 / 2);
        assert_eq!(full_name(&big), Err(FullNameError::TooLong));
        assert_eq!(full_name(&" ".repeat(10 * 1024 * 1024)), Ok(""));
        // a leading dash is a name, not an option: `useradd -c <it> -- user`
        assert_eq!(full_name("-rf /"), Ok("-rf /"));
    }

    #[test]
    fn the_reserved_list_is_itself_valid_names() {
        for n in RESERVED_NAMES {
            let b = n.as_bytes();
            assert!(
                b[0].is_ascii_lowercase()
                    && b.iter().all(|c| c.is_ascii_lowercase()
                        || c.is_ascii_digit()
                        || *c == b'_'
                        || *c == b'-'),
                "{n} could never be typed as a name, so reserving it does nothing"
            );
        }
    }
}

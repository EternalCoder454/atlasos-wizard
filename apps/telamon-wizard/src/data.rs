//! The lists the pages show, built by parsing files (never by shelling out):
//! locales (glibc's locale archive, or `SUPPORTED`), keyboard layouts
//! (`xkb/rules/evdev.xml`) and time zones (`zone1970.tab`). Parsers take text
//! or bytes so tests need no system files; loaders cap what they read.

use serde::Serialize;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use wizard_core::choices::{xkb_layout, xkb_variant};

/// Largest file any loader reads (evdev.xml is about 2 MB).
const MAX_TEXT: u64 = 16 << 20;

pub const LOCALE_ARCHIVE: &str = "/usr/lib/locale/locale-archive";
pub const LOCALE_SUPPORTED: &str = "/usr/share/i18n/SUPPORTED";
pub const EVDEV_XML: &str = "/usr/share/X11/xkb/rules/evdev.xml";
pub const ZONE_TAB: &str = "/usr/share/zoneinfo/zone1970.tab";

fn read_text(path: &Path) -> io::Result<String> {
    let mut s = String::new();
    File::open(path)?.take(MAX_TEXT).read_to_string(&mut s)?;
    Ok(s)
}

/// A locale name worth offering: `xx_YY.UTF-8` shape, short, safe characters.
fn locale_ok(s: &str) -> bool {
    s.len() <= 64
        && (s.contains('_') || s.starts_with("C."))
        && (s.ends_with(".UTF-8") || s.contains(".UTF-8@"))
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '@'))
}

/// Normalizes `de_DE.utf8` / `de_DE.UTF-8` to `de_DE.UTF-8`; other codesets
/// give `None`.
fn norm_locale(name: &str) -> Option<String> {
    let (base, set) = name.split_once('.')?;
    let (set, rest) = set.split_once('@').map_or((set, ""), |(a, b)| (a, b));
    if !set.eq_ignore_ascii_case("utf8") && !set.eq_ignore_ascii_case("utf-8") {
        return None;
    }
    let mut out = format!("{base}.UTF-8");
    if !rest.is_empty() {
        out.push('@');
        out.push_str(rest);
    }
    locale_ok(&out).then_some(out)
}

fn finish_locales(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v.dedup();
    v
}

/// Names in glibc's locale archive: header, hash table, string pool (the
/// same fields `localectl list-locales` reads). Only those parts are read.
pub fn parse_locale_archive<R: Read + Seek>(r: &mut R) -> io::Result<Vec<String>> {
    let bad = || io::Error::new(io::ErrorKind::InvalidData, "not a locale archive");
    let mut h = [0u8; 32];
    r.read_exact(&mut h)?;
    let u = |i: usize| u32::from_le_bytes([h[i * 4], h[i * 4 + 1], h[i * 4 + 2], h[i * 4 + 3]]);
    if u(0) != 0xde02_0109 {
        return Err(bad());
    }
    let (ho, hsize) = (u64::from(u(2)), u64::from(u(4)));
    let (so, ssize) = (u64::from(u(5)), u64::from(u(7)));
    if hsize == 0 || hsize > 1 << 20 || ssize == 0 || ssize > 8 << 20 {
        return Err(bad());
    }
    let mut table = vec![0u8; (hsize * 12) as usize];
    r.seek(SeekFrom::Start(ho))?;
    r.read_exact(&mut table)?;
    let mut pool = vec![0u8; ssize as usize];
    r.seek(SeekFrom::Start(so))?;
    r.read_exact(&mut pool)?;
    let mut out = Vec::new();
    for e in table.as_chunks::<12>().0.iter() {
        let name_off = u64::from(u32::from_le_bytes([e[4], e[5], e[6], e[7]]));
        let rec = u32::from_le_bytes([e[8], e[9], e[10], e[11]]);
        if rec == 0 || name_off < so || name_off >= so + ssize {
            continue;
        }
        let tail = &pool[(name_off - so) as usize..];
        let end = tail.iter().position(|b| *b == 0).unwrap_or(tail.len());
        if let Some(n) = std::str::from_utf8(&tail[..end]).ok().and_then(norm_locale) {
            out.push(n);
        }
    }
    Ok(finish_locales(out))
}

/// Lines of glibc's `SUPPORTED` (`de_DE.UTF-8 UTF-8`).
pub fn parse_supported(text: &str) -> Vec<String> {
    finish_locales(
        text.lines()
            .filter_map(|l| l.split_whitespace().next())
            .filter(|n| locale_ok(n))
            .map(str::to_string)
            .collect(),
    )
}

/// Used when the system has no locale list at all.
pub fn builtin_locales() -> Vec<String> {
    [
        "de_DE", "en_GB", "en_US", "es_ES", "fr_FR", "it_IT", "ja_JP", "nl_NL", "pl_PL", "pt_BR",
        "ru_RU", "zh_CN",
    ]
    .iter()
    .map(|l| format!("{l}.UTF-8"))
    .collect()
}

/// UTF-8 locales installed as directories (`/usr/lib/locale/C.utf8`).
pub fn parse_locale_dirs(dir: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    finish_locales(
        rd.filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|e| norm_locale(&e.file_name().to_string_lossy()))
            .collect(),
    )
}

/// The installed locales: the archive and the directories together, else
/// `SUPPORTED`, else the built-in short list.
pub fn load_locales() -> Vec<String> {
    let mut all = parse_locale_dirs(Path::new("/usr/lib/locale"));
    all.extend(load_archive());
    let all = finish_locales(all);
    if !all.is_empty() {
        return all;
    }
    if let Ok(t) = read_text(Path::new(LOCALE_SUPPORTED)) {
        let v = parse_supported(&t);
        if !v.is_empty() {
            return v;
        }
    }
    builtin_locales()
}

fn load_archive() -> Vec<String> {
    if let Ok(mut f) = File::open(LOCALE_ARCHIVE) {
        match parse_locale_archive(&mut f) {
            Ok(v) => return v,
            Err(e) => log::warn!("{LOCALE_ARCHIVE}: {e}"),
        }
    }
    Vec::new()
}

/// A keyboard layout variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Variant {
    pub name: String,
    pub description: String,
}

/// A keyboard layout with its variants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Layout {
    pub name: String,
    pub description: String,
    pub variants: Vec<Variant>,
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Reads `evdev.xml`: each `<layout>`'s and `<variant>`'s `<configItem>`
/// name and description. Names that are not valid XKB names are dropped.
pub fn parse_layouts(xml: &str) -> Vec<Layout> {
    let mut out: Vec<Layout> = Vec::new();
    let mut in_variant = false;
    let mut field: Option<&str> = None;
    let (mut name, mut desc) = (String::new(), String::new());
    let mut rest = xml;
    while let Some(lt) = rest.find('<') {
        let text = &rest[..lt];
        rest = &rest[lt..];
        if rest.starts_with("<!--") {
            rest = rest.find("-->").map_or("", |i| &rest[i + 3..]);
            continue;
        }
        let Some(gt) = rest.find('>') else { break };
        let tag = &rest[1..gt];
        rest = &rest[gt + 1..];
        // text before this tag belongs to the open field
        match field {
            Some("name") => name = unescape(text.trim()),
            Some("description") => desc = unescape(text.trim()),
            _ => {}
        }
        field = None;
        match tag {
            "layout" => {
                out.push(Layout {
                    name: String::new(),
                    description: String::new(),
                    variants: Vec::new(),
                });
                in_variant = false;
            }
            "variant" => in_variant = true,
            "name" => field = Some("name"),
            "description" => field = Some("description"),
            "/configItem" => {
                if let Some(l) = out.last_mut() {
                    if in_variant {
                        l.variants.push(Variant {
                            name: std::mem::take(&mut name),
                            description: std::mem::take(&mut desc),
                        });
                    } else if l.name.is_empty() {
                        l.name = std::mem::take(&mut name);
                        l.description = std::mem::take(&mut desc);
                    }
                }
                name.clear();
                desc.clear();
            }
            "/variant" => in_variant = false,
            _ => {}
        }
    }
    out.retain(|l| xkb_layout(&l.name));
    for l in &mut out {
        l.variants.retain(|v| xkb_variant(&v.name));
        if l.description.is_empty() {
            l.description = l.name.clone();
        }
    }
    out.sort_by(|a, b| {
        a.description
            .to_lowercase()
            .cmp(&b.description.to_lowercase())
    });
    out
}

/// The keyboard layouts of this system, `None` when the file is unreadable.
pub fn load_layouts() -> io::Result<Vec<Layout>> {
    let l = parse_layouts(&read_text(Path::new(EVDEV_XML))?);
    if l.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "no layouts"));
    }
    Ok(l)
}

/// A time zone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Zone {
    pub id: String,
    pub countries: String,
    pub comment: String,
}

/// A zone id timedated may be given: `Area/City` characters only, every
/// part non-empty and starting with a letter (so no `//`, no trailing `/`,
/// no part that is `-x` or a number).
pub fn zone_id_ok(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '+'))
        && id
            .split('/')
            .all(|part| part.chars().next().is_some_and(|c| c.is_ascii_alphabetic()))
}

/// Reads `zone1970.tab` (`CC[,CC]<TAB>coordinates<TAB>zone<TAB>comment`).
/// Comments and malformed lines are skipped. `UTC` is added.
pub fn parse_zones(text: &str) -> Vec<Zone> {
    let mut v: Vec<Zone> = text
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let mut f = l.split('\t');
            let (cc, _, id) = (f.next()?, f.next()?, f.next()?);
            let comment = f.next().unwrap_or("");
            zone_id_ok(id).then(|| Zone {
                id: id.to_string(),
                countries: cc.to_string(),
                comment: comment.to_string(),
            })
        })
        .collect();
    v.push(Zone {
        id: "UTC".into(),
        countries: String::new(),
        comment: String::new(),
    });
    v.sort_by(|a, b| a.id.cmp(&b.id));
    v.dedup_by(|a, b| a.id == b.id);
    v
}

/// The time zones of this system.
pub fn load_zones() -> io::Result<Vec<Zone>> {
    let z = parse_zones(&read_text(Path::new(ZONE_TAB))?);
    if z.len() < 2 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "no zones"));
    }
    Ok(z)
}

/// The zone `/etc/localtime` points at, when it can be told.
pub fn current_zone() -> Option<String> {
    let t = std::fs::read_link("/etc/localtime").ok()?;
    let s = t.to_string_lossy();
    let id = s.split("zoneinfo/").nth(1)?;
    zone_id_ok(id).then(|| id.to_string())
}

/// A host name hostnamed will take as a static name: one DNS label (RFC
/// 1123: lower-case ASCII letters, digits and `-`, 63 at most, not starting
/// or ending with `-`), and not `localhost`, which is the name that asks for
/// a real one.
pub fn hostname_ok(s: &str) -> bool {
    !s.is_empty()
        && s != "localhost"
        && s.len() <= 63
        && !s.starts_with('-')
        && !s.ends_with('-')
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The host names that still ask for a real one.
pub fn hostname_is_default(s: &str) -> bool {
    let s = s.trim().to_ascii_lowercase();
    s.is_empty() || s.starts_with("localhost") || s == "fedora"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const XML: &str = r#"<?xml version="1.0"?>
<xkbConfig><layoutList>
<layout><configItem><name>us</name><shortDescription>en</shortDescription><description>English (US)</description></configItem>
<variantList><variant><configItem><name>dvorak</name><description>English (Dvorak)</description></configItem></variant>
<variant><configItem><name>bad name</name><description>x</description></configItem></variant></variantList></layout>
<!-- c --><layout><configItem><name>de</name><description>German &amp; more</description></configItem></layout>
<layout><configItem><name>../x</name><description>Evil</description></configItem></layout>
</layoutList></xkbConfig>"#;

    #[test]
    fn layouts_parse_and_filter() {
        let l = parse_layouts(XML);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].name, "us");
        let de = l.iter().find(|x| x.name == "de").unwrap();
        assert_eq!(de.description, "German & more");
        let us = l.iter().find(|x| x.name == "us").unwrap();
        assert_eq!(us.variants.len(), 1);
        assert_eq!(us.variants[0].name, "dvorak");
    }

    #[test]
    fn zones_parse() {
        let z = parse_zones(
            "# c\nDE,AT\t+52+013\tEurope/Berlin\tGermany\nXX\t+0+0\t../etc\tbad\nshort\n",
        );
        let ids: Vec<_> = z.iter().map(|z| z.id.as_str()).collect();
        assert_eq!(ids, ["Europe/Berlin", "UTC"]);
        assert!(
            !zone_id_ok("a/../b") && !zone_id_ok("/etc") && zone_id_ok("America/Port-au-Prince")
        );
    }

    #[test]
    fn supported_and_norm() {
        let v = parse_supported(
            "de_DE.UTF-8 UTF-8\nde_DE ISO-8859-1\nC.UTF-8 UTF-8\nbad;x_Y.UTF-8 UTF-8\n",
        );
        assert_eq!(v, ["C.UTF-8", "de_DE.UTF-8"]);
        assert_eq!(norm_locale("fr_CA.utf8").as_deref(), Some("fr_CA.UTF-8"));
        assert_eq!(norm_locale("fr_CA.iso88591"), None);
        assert_eq!(
            norm_locale("sr_RS.utf8@latin").as_deref(),
            Some("sr_RS.UTF-8@latin")
        );
    }

    #[test]
    fn archive_names_read() {
        // header 32 bytes, hash table (2 entries) at 32, pool after.
        let pool_off = 32 + 24;
        let pool = b"en_US.utf8\0xx\0";
        let mut b = Vec::new();
        for v in [
            0xde02_0109u32,
            1,
            32,
            1,
            2,
            pool_off,
            pool.len() as u32,
            pool.len() as u32,
        ] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        // entry 0: used, name at pool start; entry 1: unused
        let mut t = vec![0u8; 24];
        t[4..8].copy_from_slice(&pool_off.to_le_bytes());
        t[8..12].copy_from_slice(&1u32.to_le_bytes());
        b.extend(t);
        b.extend_from_slice(pool);
        assert_eq!(
            parse_locale_archive(&mut Cursor::new(b)).unwrap(),
            ["en_US.UTF-8"]
        );
        assert!(parse_locale_archive(&mut Cursor::new(vec![0u8; 64])).is_err());
    }

    #[test]
    fn locale_dirs_only_utf8() {
        let d = tempfile::tempdir().unwrap();
        for n in ["C.utf8", "en_US.utf8", "de_DE.iso88591"] {
            std::fs::create_dir(d.path().join(n)).unwrap();
        }
        std::fs::write(d.path().join("locale-archive"), b"x").unwrap();
        assert_eq!(parse_locale_dirs(d.path()), ["C.UTF-8", "en_US.UTF-8"]);
    }

    #[test]
    fn hostnames() {
        assert!(
            hostname_ok("ada-pc") && !hostname_ok("Ada") && !hostname_ok("-a") && !hostname_ok("")
        );
        assert!(hostname_is_default("localhost.localdomain") && hostname_is_default("fedora"));
        assert!(!hostname_is_default("ada-pc"));
    }
}

#[cfg(test)]
mod props {
    use super::*;
    use proptest::prelude::*;

    fn text() -> impl Strategy<Value = String> {
        prop_oneof![
            3 => any::<String>(),
            2 => prop::sample::select(vec![
                "<layout><configItem><name>us</name><description>English</description></configItem>",
                "</configItem>", "<variant>", "</variant>", "<name>", "</name>", "<!--", "-->",
                "<description>", "</layout>", "<layout>", "&amp;", "&lt;", "<", ">", "x",
            ]).prop_map(String::from),
        ]
    }

    proptest! {
        /// Hostnames: accepted ones are one lower-case DNS label.
        #[test]
        fn hostname_ok_accepts_only_one_label(s in any::<String>(), t in "[a-z0-9-]{0,70}") {
            for s in [s, t] {
                if hostname_ok(&s) {
                    prop_assert!(!s.is_empty() && s.len() <= 63 && s != "localhost");
                    prop_assert!(!s.starts_with('-') && !s.ends_with('-'));
                    prop_assert!(s.bytes().all(|b| b.is_ascii_lowercase()
                        || b.is_ascii_digit() || b == b'-'));
                }
            }
        }

        /// Zone ids: accepted ones cannot leave the zoneinfo directory or
        /// look like an option, and every part is a name.
        #[test]
        fn zone_id_ok_accepts_only_zoneinfo_shaped_names(s in any::<String>(), t in "[A-Za-z0-9_+/.-]{0,70}") {
            for s in [s, t] {
                if zone_id_ok(&s) {
                    prop_assert!(s.len() <= 64 && !s.starts_with('/') && !s.starts_with('-'));
                    prop_assert!(!s.contains("..") && !s.contains("//") && !s.ends_with('/'));
                    prop_assert!(s.split('/').all(|p| p.as_bytes()[0].is_ascii_alphabetic()));
                }
            }
        }

        /// Locale names the lists offer are short and made of safe characters.
        #[test]
        fn offered_locales_are_safe(s in any::<String>(), lines in prop::collection::vec(any::<String>(), 0..6)) {
            if let Some(n) = norm_locale(&s) {
                prop_assert!(locale_ok(&n));
            }
            for n in parse_supported(&lines.join("\n")) {
                prop_assert!(locale_ok(&n) && n.len() <= 64);
            }
        }

        /// The XML and tab readers never panic on any text and offer only
        /// names that `set_keyboard` and `set_timezone` will take.
        #[test]
        fn lists_never_panic_and_offer_only_valid_names(t in prop::collection::vec(text(), 0..30)) {
            let t = t.concat();
            for l in parse_layouts(&t) {
                prop_assert!(xkb_layout(&l.name));
                prop_assert!(l.variants.iter().all(|v| xkb_variant(&v.name)));
            }
            for z in parse_zones(&t.replace(' ', "\t")) {
                prop_assert!(zone_id_ok(&z.id), "{}", z.id);
            }
        }
    }

    proptest! {
        /// The locale archive reader never panics on a header with any
        /// fields followed by any bytes, and reads no more than its caps.
        #[test]
        fn locale_archive_reader_survives_any_bytes(
            fields in prop::array::uniform8(any::<u32>()),
            small in prop::array::uniform8(0u32..200),
            tail in prop::collection::vec(any::<u8>(), 0..600),
            use_small in any::<bool>(),
        ) {
            static RUNS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if !crate::budget::within(&RUNS, 300) {
                return Ok(());
            }
            let f = if use_small { small } else { fields };
            let mut b = Vec::new();
            for (i, v) in f.iter().enumerate() {
                let v = if i == 0 { 0xde02_0109 } else { *v };
                b.extend_from_slice(&v.to_le_bytes());
            }
            b.extend_from_slice(&tail);
            let r = parse_locale_archive(&mut std::io::Cursor::new(b));
            if let Ok(names) = r {
                prop_assert!(names.iter().all(|n| locale_ok(n)));
            }
        }
    }

    #[test]
    fn nasty_names() {
        let big = "a".repeat(10 * 1024 * 1024);
        for h in [
            "",
            "-a",
            "a-",
            "A",
            "a.b",
            "a b",
            "a\nb",
            "a\0b",
            "localhost",
            "ada_pc",
            "é",
            "../x",
            "-rf",
            "--help",
            "a/b",
            big.as_str(),
            &"a".repeat(64),
        ] {
            assert!(!hostname_ok(h), "{:?}", &h[..h.len().min(20)]);
        }
        assert!(hostname_ok(&"a".repeat(63)) && hostname_ok("ada-pc") && hostname_ok("0"));
        for z in [
            "",
            "/",
            "/etc/passwd",
            "../x",
            "a/../b",
            "a//b",
            "a/",
            "-x",
            "Europe/-x",
            "1/2",
            "a b",
            "a\nb",
            "a\0b",
            "Europe/Berlin\n",
            big.as_str(),
        ] {
            assert!(!zone_id_ok(z), "{:?}", &z[..z.len().min(20)]);
        }
        for z in [
            "UTC",
            "Europe/Berlin",
            "America/Port-au-Prince",
            "Etc/GMT+5",
            "America/Argentina/Buenos_Aires",
        ] {
            assert!(zone_id_ok(z), "{z}");
        }
        assert!(parse_layouts(&"<layout>".repeat(100_000)).is_empty());
        assert_eq!(parse_zones(&"x\t\t../etc\n".repeat(100_000)).len(), 1);
    }
}

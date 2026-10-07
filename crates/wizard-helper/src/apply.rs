//! `telamon-wizard-helper apply-user-settings`: the child that writes the new
//! account's settings. It runs as that account (the helper dropped to its uid
//! and gid before exec), reads the validated choices as JSON on stdin, and
//! writes only under its own home.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use wizard_core::choices::{self, Choices, Value};

use crate::paths::{HC_SCHEME_FILE, Paths, SYSTEM_KDEGLOBALS};
use crate::safefs::{ensure_dir_under, ini_get, ini_set, read_nofollow, write_nofollow};

/// Longest a `plasma-apply-*` tool may run.
pub const TOOL_TIMEOUT: Duration = Duration::from_secs(30);

/// Largest input accepted on stdin.
const MAX_INPUT: u64 = 16 * 1024;

/// The colour scheme file name (without `.colors`) for high contrast.
pub const HC_SCHEME: &str = "AtlasOSHighContrast";

/// `kdeglobals` keys that hold a font, as `(section, key)`.
pub const FONT_KEYS: [(&str, &str); 6] = [
    ("General", "font"),
    ("General", "menuFont"),
    ("General", "toolBarFont"),
    ("General", "smallestReadableFont"),
    ("General", "fixed"),
    ("WM", "activeFont"),
];

/// Runs the fixed tools. A fake in tests.
pub trait Tools {
    /// Runs `program` with `args`; `Err` carries what to log.
    fn run(&self, program: &Path, args: &[&str]) -> Result<(), String>;
}

/// Runs the tools for real: no shell, 30 s, killed with its process group.
pub struct RealTools;

impl Tools for RealTools {
    fn run(&self, program: &Path, args: &[&str]) -> Result<(), String> {
        use std::os::unix::process::CommandExt;
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(|e| format!("cannot start {}: {}", program.display(), e.kind()))?;
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(status)) => {
                    return Err(format!("{} exited with {status}", program.display()));
                }
                Ok(None) if start.elapsed() >= TOOL_TIMEOUT => {
                    if let Some(pgid) = rustix::process::Pid::from_raw(child.id() as i32) {
                        let _ = rustix::process::kill_process_group(
                            pgid,
                            rustix::process::Signal::KILL,
                        );
                    }
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("{} ran over {TOOL_TIMEOUT:?}", program.display()));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(e) => return Err(format!("waiting for {}: {}", program.display(), e.kind())),
            }
        }
    }
}

/// Scales the point size (the second comma-separated field) of a Qt font
/// string and keeps the rest. `None` when the string has no usable size.
pub fn scale_font(font: &str, factor: f64) -> Option<String> {
    let mut fields: Vec<&str> = font.split(',').collect();
    let size: f64 = fields.get(1)?.trim().parse().ok()?;
    if !(size.is_finite() && size > 0.0 && size <= 200.0) {
        return None;
    }
    let scaled = ((size * factor) * 100.0).round() / 100.0;
    let text = format!("{scaled}");
    fields[1] = &text;
    Some(fields.join(","))
}

/// Converts the JSON the helper sends into the choices map's values.
fn json_to_value(v: &serde_json::Value, depth: u8) -> Option<Value> {
    use serde_json::Value as J;
    match v {
        J::String(s) => Some(Value::Str(s.clone())),
        J::Bool(b) => Some(Value::Bool(*b)),
        J::Number(n) => n.as_f64().map(Value::F64),
        J::Object(m) if depth < 2 => {
            let mut out = BTreeMap::new();
            for (k, v) in m {
                out.insert(k.clone(), json_to_value(v, depth + 1)?);
            }
            Some(Value::Map(out))
        }
        _ => None,
    }
}

/// Parses and validates the choices JSON (again: the child trusts nothing).
///
/// # Errors
/// Text for the log.
pub fn parse_choices(json: &[u8]) -> Result<Choices, String> {
    let v: serde_json::Value =
        serde_json::from_slice(json).map_err(|e| format!("choices JSON: {e}"))?;
    let serde_json::Value::Object(m) = v else {
        return Err("choices JSON is not an object".into());
    };
    let mut map = BTreeMap::new();
    for (k, v) in &m {
        map.insert(
            k.clone(),
            json_to_value(v, 0).ok_or_else(|| format!("choices JSON: bad value for {k}"))?,
        );
    }
    choices::validate(map).map_err(|e| format!("choices refused: {}", e.code()))
}

/// The JSON the helper sends the child for validated `c`.
pub fn choices_json(c: &Choices) -> Vec<u8> {
    let mut m = serde_json::Map::new();
    m.insert(
        "look".into(),
        match c.look {
            choices::Look::Light => "light",
            choices::Look::Dark => "dark",
        }
        .into(),
    );
    m.insert("accent".into(), c.accent.into());
    m.insert("text_scale".into(), c.text_scale.factor().into());
    m.insert("high_contrast".into(), c.high_contrast.into());
    m.insert("screen_reader".into(), c.screen_reader.into());
    m.insert("crash_reports".into(), c.crash_reports.into());
    if let Some(k) = &c.keyboard {
        m.insert(
            "keyboard".into(),
            serde_json::json!({"layout": k.layout, "variant": k.variant}),
        );
    }
    serde_json::Value::Object(m).to_string().into_bytes()
}

/// Everything the writes need.
pub struct Env<'a> {
    /// The account's home directory.
    pub home: &'a Path,
    /// Where `/etc`, `/usr` are (`/` on a real system).
    pub paths: &'a Paths,
}

fn merge_ini(path: &Path, edits: &[(&str, &str, String)], mode: u32) -> Result<(), String> {
    let fail = |what: &str, e: std::io::Error| format!("{}: {what}: {e}", path.display());
    let mut text = read_nofollow(path)
        .map_err(|e| fail("read", e))?
        .unwrap_or_default();
    for (section, key, value) in edits {
        text = ini_set(&text, section, key, value).map_err(|e| fail("edit", e))?;
    }
    write_nofollow(path, text.as_bytes(), mode).map_err(|e| fail("write", e))
}

/// Writes the settings and runs the apply tools. Returns the errors of the
/// file writes (the tools' failures are logged, never errors).
///
/// # Errors
/// One line per file that could not be written.
pub fn apply(env: &Env<'_>, c: &Choices, tools: &dyn Tools) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    let config = env.home.join(".config");
    let mut step = |name: &str, r: Result<(), String>| match r {
        Ok(()) => log::info!("settings: wrote {name}"),
        Err(e) => {
            log::error!("settings: {name} failed: {e}");
            errors.push(format!("{name}: {e}"));
        }
    };

    // the base dirs of the home, 0700 as the XDG spec says
    for rel in [".config", ".local", ".local/share", ".local/state"] {
        let r = ensure_dir_under(env.home, &env.home.join(rel), 0o700).map_err(|e| e.to_string());
        if r.is_err() {
            step(rel, r);
        }
    }

    if c.screen_reader {
        step(
            "kaccessrc",
            merge_ini(
                &config.join("kaccessrc"),
                &[("ScreenReader", "Enabled", "true".into())],
                0o644,
            ),
        );
    }
    if let Some(k) = &c.keyboard {
        step(
            "kxkbrc",
            merge_ini(
                &config.join("kxkbrc"),
                &[
                    ("Layout", "LayoutList", k.layout.clone()),
                    ("Layout", "VariantList", k.variant.clone()),
                    ("Layout", "Use", "true".into()),
                ],
                0o644,
            ),
        );
    }
    // The choice goes where Telamon.Ui 2 apps read it (`~/.config/telamon`)
    // and where Atlas.Ui 1.x apps, which have not moved yet, still do
    // (`~/.config/atlas`).
    for (dir, what) in [
        ("telamon", "crash-reporting.toml"),
        ("atlas", "crash-reporting.toml (Atlas.Ui 1.x apps)"),
    ] {
        let crash_dir = config.join(dir);
        step(
            what,
            ensure_dir_under(env.home, &crash_dir, 0o700)
                .and_then(|()| {
                    telamon_framework_system::crash::Settings {
                        enabled: c.crash_reports,
                    }
                    .save_to(&crash_dir.join("crash-reporting.toml"))
                })
                .map_err(|e| e.to_string()),
        );
    }

    // The look and the accent through KDE's own tools. A missing tool or a
    // failure is not fatal: the account still works, with the default look.
    let tool = |name: &str| env.paths.join(format!("/usr/bin/{name}"));
    let apply_tool = |name: &str, args: &[&str]| match tools.run(&tool(name), args) {
        Ok(()) => log::info!("settings: ran {name}"),
        Err(e) => log::warn!("settings: {e} (not fatal)"),
    };
    apply_tool("plasma-apply-lookandfeel", &["--apply", c.look.theme_id()]);
    apply_tool("plasma-apply-colorscheme", &["--accent-color", c.accent]);

    // kdeglobals last: the apply tools above rewrite colours and may reset fonts
    let mut edits: Vec<(&str, &str, String)> = Vec::new();
    let factor = c.text_scale.factor();
    if factor != 1.0 {
        let base = read_nofollow(&env.paths.join(SYSTEM_KDEGLOBALS))
            .ok()
            .flatten()
            .unwrap_or_default();
        for (section, key) in FONT_KEYS {
            match ini_get(&base, section, key).and_then(|f| scale_font(f, factor)) {
                Some(scaled) => edits.push((section, key, scaled)),
                None => log::warn!("settings: no base font for [{section}] {key}; left as it is"),
            }
        }
    }
    if c.high_contrast {
        if env.paths.join(HC_SCHEME_FILE).is_file() {
            edits.push(("General", "ColorScheme", HC_SCHEME.into()));
        } else {
            log::warn!("settings: the high contrast colour scheme is not installed; skipped");
        }
    }
    if !edits.is_empty() {
        step(
            "kdeglobals",
            merge_ini(&config.join("kdeglobals"), &edits, 0o644),
        );
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Entry point of the subcommand. Returns the process exit code.
pub fn run_child() -> i32 {
    if rustix::process::geteuid().is_root() {
        log::error!("apply-user-settings refuses to run as root");
        return 2;
    }
    let mut input = Vec::new();
    if let Err(e) = std::io::stdin().take(MAX_INPUT + 1).read_to_end(&mut input) {
        log::error!("cannot read the choices: {}", e.kind());
        return 1;
    }
    if input.len() as u64 > MAX_INPUT {
        log::error!("the choices are too large");
        return 1;
    }
    let c = match parse_choices(&input) {
        Ok(c) => c,
        Err(e) => {
            log::error!("{e}");
            return 1;
        }
    };
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        log::error!("HOME is not set");
        return 1;
    };
    if !home.is_absolute() {
        log::error!("HOME is not absolute");
        return 1;
    }
    let paths = Paths::from_env();
    match apply(
        &Env {
            home: &home,
            paths: &paths,
        },
        &c,
        &RealTools,
    ) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    #[derive(Default)]
    struct Fake {
        calls: RefCell<Vec<(String, Vec<String>)>>,
        fail: bool,
    }
    impl Tools for Fake {
        fn run(&self, program: &Path, args: &[&str]) -> Result<(), String> {
            self.calls.borrow_mut().push((
                program.to_string_lossy().into_owned(),
                args.iter().map(|s| s.to_string()).collect(),
            ));
            if self.fail {
                Err("boom".into())
            } else {
                Ok(())
            }
        }
    }

    const SYSTEM: &str = "[General]\nfont=Noto Sans,10,-1,5,400,0,0,0,0,0,0,0,0,0,0,1\n\
menuFont=Noto Sans,10,-1,5,400,0,0,0,0,0,0,0,0,0,0,1\nfixed=Hack,9.5,-1,5,400,0,0,0,0,0,0,0,0,0,0,1,Regular\n\
toolBarFont=Noto Sans,9,-1,5,400,0,0,0,0,0,0,0,0,0,0,1\nsmallestReadableFont=Noto Sans,8,-1,5,400,0,0,0,0,0,0,0,0,0,0,1\n\
[WM]\nactiveFont=Noto Sans,10,-1,5,700,0,0,0,0,0,0,0,0,0,0,1\n";

    fn root() -> (tempfile::TempDir, Paths) {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("etc/xdg")).unwrap();
        fs::write(d.path().join("etc/xdg/kdeglobals"), SYSTEM).unwrap();
        fs::create_dir_all(d.path().join("home/ada")).unwrap();
        let p = Paths::with_root(d.path());
        (d, p)
    }

    fn all_on() -> Choices {
        choices::validate([
            ("look".to_string(), Value::Str("dark".into())),
            ("accent".to_string(), Value::Str("#e93a9a".into())),
            ("text_scale".to_string(), Value::F64(1.25)),
            ("high_contrast".to_string(), Value::Bool(true)),
            ("screen_reader".to_string(), Value::Bool(true)),
            ("crash_reports".to_string(), Value::Bool(true)),
            (
                "keyboard".to_string(),
                Value::Map(BTreeMap::from([
                    ("layout".to_string(), Value::Str("de".into())),
                    ("variant".to_string(), Value::Str("nodeadkeys".into())),
                ])),
            ),
        ])
        .unwrap()
    }

    #[test]
    fn scales_the_point_size_and_keeps_the_rest() {
        assert_eq!(
            scale_font("Noto Sans,10,-1,5,400,0", 1.25).as_deref(),
            Some("Noto Sans,12.5,-1,5,400,0")
        );
        assert_eq!(
            scale_font("Hack,9.5,-1", 1.5).as_deref(),
            Some("Hack,14.25,-1")
        );
        assert_eq!(scale_font("Hack,10", 1.0).as_deref(), Some("Hack,10"));
        assert_eq!(scale_font("Hack", 1.5), None);
        assert_eq!(scale_font("Hack,abc", 1.5), None);
        assert_eq!(scale_font("Hack,-3", 1.5), None);
        assert_eq!(scale_font("Hack,nan", 1.5), None);
    }

    #[test]
    fn choices_survive_the_json_round_trip() {
        let c = all_on();
        let back = parse_choices(&choices_json(&c)).unwrap();
        assert_eq!(back, c);
        let d = Choices::default();
        assert_eq!(parse_choices(&choices_json(&d)).unwrap(), d);
        assert!(parse_choices(b"[]").is_err());
        assert!(parse_choices(b"not json").is_err());
        assert!(parse_choices(br#"{"look":"neon"}"#).is_err());
        assert!(parse_choices(br#"{"surprise":true}"#).is_err());
        assert!(parse_choices(br#"{"look":null}"#).is_err());
    }

    #[test]
    fn writes_every_file_and_runs_the_tools() {
        let (d, paths) = root();
        fs::create_dir_all(d.path().join("usr/share/color-schemes")).unwrap();
        fs::write(
            d.path()
                .join("usr/share/color-schemes/AtlasOSHighContrast.colors"),
            "",
        )
        .unwrap();
        let home = d.path().join("home/ada");
        let tools = Fake::default();
        apply(
            &Env {
                home: &home,
                paths: &paths,
            },
            &all_on(),
            &tools,
        )
        .unwrap();

        let kde = fs::read_to_string(home.join(".config/kdeglobals")).unwrap();
        assert_eq!(
            ini_get(&kde, "General", "font"),
            Some("Noto Sans,12.5,-1,5,400,0,0,0,0,0,0,0,0,0,0,1")
        );
        assert_eq!(
            ini_get(&kde, "General", "fixed"),
            Some("Hack,11.88,-1,5,400,0,0,0,0,0,0,0,0,0,0,1,Regular")
        );
        assert_eq!(
            ini_get(&kde, "WM", "activeFont"),
            Some("Noto Sans,12.5,-1,5,700,0,0,0,0,0,0,0,0,0,0,1")
        );
        assert_eq!(
            ini_get(&kde, "General", "ColorScheme"),
            Some("AtlasOSHighContrast")
        );
        let acc = fs::read_to_string(home.join(".config/kaccessrc")).unwrap();
        assert_eq!(ini_get(&acc, "ScreenReader", "Enabled"), Some("true"));
        let kxkb = fs::read_to_string(home.join(".config/kxkbrc")).unwrap();
        assert_eq!(ini_get(&kxkb, "Layout", "LayoutList"), Some("de"));
        assert_eq!(ini_get(&kxkb, "Layout", "VariantList"), Some("nodeadkeys"));
        assert_eq!(ini_get(&kxkb, "Layout", "Use"), Some("true"));
        let crash = fs::read_to_string(home.join(".config/telamon/crash-reporting.toml")).unwrap();
        assert!(crash.contains("enabled = true"), "{crash}");
        let mode = |p: &str| fs::metadata(home.join(p)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(".config"), 0o700);
        assert_eq!(mode(".local/state"), 0o700);

        let calls = tools.calls.borrow();
        assert_eq!(
            *calls,
            vec![
                (
                    d.path()
                        .join("usr/bin/plasma-apply-lookandfeel")
                        .to_string_lossy()
                        .into_owned(),
                    vec![
                        "--apply".to_string(),
                        "org.telamon.dark.desktop".to_string()
                    ]
                ),
                (
                    d.path()
                        .join("usr/bin/plasma-apply-colorscheme")
                        .to_string_lossy()
                        .into_owned(),
                    vec!["--accent-color".to_string(), "#E93A9A".to_string()]
                ),
            ]
        );
    }

    #[test]
    fn defaults_write_only_the_crash_switch() {
        let (d, paths) = root();
        let home = d.path().join("home/ada");
        let tools = Fake::default();
        apply(
            &Env {
                home: &home,
                paths: &paths,
            },
            &Choices::default(),
            &tools,
        )
        .unwrap();
        assert!(!home.join(".config/kdeglobals").exists());
        assert!(!home.join(".config/kaccessrc").exists());
        assert!(!home.join(".config/kxkbrc").exists());
        let crash = fs::read_to_string(home.join(".config/telamon/crash-reporting.toml")).unwrap();
        assert!(crash.contains("enabled = false"), "{crash}");
        assert_eq!(tools.calls.borrow().len(), 2);
    }

    #[test]
    fn high_contrast_without_the_scheme_is_skipped() {
        let (d, paths) = root();
        let home = d.path().join("home/ada");
        let c = Choices {
            high_contrast: true,
            ..Choices::default()
        };
        apply(
            &Env {
                home: &home,
                paths: &paths,
            },
            &c,
            &Fake::default(),
        )
        .unwrap();
        assert!(!home.join(".config/kdeglobals").exists());
    }

    #[test]
    fn existing_keys_are_kept_and_tool_failures_are_not_fatal() {
        let (d, paths) = root();
        let home = d.path().join("home/ada");
        fs::create_dir_all(home.join(".config")).unwrap();
        fs::write(
            home.join(".config/kdeglobals"),
            "[KDE]\nwidgetStyle=Breeze\n[General]\nfont=old\n",
        )
        .unwrap();
        let c = Choices {
            text_scale: choices::TextScale::Larger,
            ..Choices::default()
        };
        let tools = Fake {
            fail: true,
            ..Fake::default()
        };
        apply(
            &Env {
                home: &home,
                paths: &paths,
            },
            &c,
            &tools,
        )
        .unwrap();
        let kde = fs::read_to_string(home.join(".config/kdeglobals")).unwrap();
        assert_eq!(ini_get(&kde, "KDE", "widgetStyle"), Some("Breeze"));
        assert_eq!(
            ini_get(&kde, "General", "font"),
            Some("Noto Sans,15,-1,5,400,0,0,0,0,0,0,0,0,0,0,1")
        );
    }

    #[test]
    fn a_symlinked_file_is_refused_and_reported() {
        let (d, paths) = root();
        let home = d.path().join("home/ada");
        fs::create_dir_all(home.join(".config")).unwrap();
        let victim = d.path().join("victim");
        fs::write(&victim, "keep").unwrap();
        std::os::unix::fs::symlink(&victim, home.join(".config/kaccessrc")).unwrap();
        let c = Choices {
            screen_reader: true,
            ..Choices::default()
        };
        let r = apply(
            &Env {
                home: &home,
                paths: &paths,
            },
            &c,
            &Fake::default(),
        );
        assert!(r.is_err());
        assert_eq!(fs::read_to_string(&victim).unwrap(), "keep");
    }
}

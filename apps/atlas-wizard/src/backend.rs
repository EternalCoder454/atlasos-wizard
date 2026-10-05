//! The app-wide QObject: how the wizard was started, the lists and answers
//! the pages show, and every action they take. All system work (D-Bus, files)
//! runs on a worker thread and posts back through `qt_thread()`, so the GUI
//! thread never blocks. The only file read on the GUI thread is the small
//! answers file, once, while the object is built.

use crate::answers::{self, Answers};
use crate::errors::Fail;
use crate::system::{Demo, Real, Res, System};
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use wizard_core::{password, validate};
use zeroize::Zeroizing;

unsafe extern "C" {
    /// In `cpp/main.cpp`.
    fn atlas_set_text_scale(scale: f64);
}

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        /// True with `ATLAS_WIZARD_DEMO=1`: no helper, no system services,
        /// every call answers from canned data.
        #[qproperty(bool, demo)]
        /// True with `--welcome`: the first-login extras (fingerprint, PIN),
        /// not the full setup.
        #[qproperty(bool, welcome_mode, cxx_name = "welcomeMode")]
        /// True once the wizard has finished; only then may the window close.
        #[qproperty(bool, finished)]
        /// The start-up facts (`Startup` as JSON) have arrived.
        #[qproperty(bool, ready)]
        #[qproperty(QString, startup_json, cxx_name = "startupJson")]
        /// The saved answers (`Answers` as JSON), read once at start.
        #[qproperty(QString, answers_json, cxx_name = "answersJson")]
        /// The lists, as JSON, set by `loadList`.
        #[qproperty(QString, languages_json, cxx_name = "languagesJson")]
        #[qproperty(QString, layouts_json, cxx_name = "layoutsJson")]
        #[qproperty(QString, zones_json, cxx_name = "zonesJson")]
        #[qproperty(QString, networks_json, cxx_name = "networksJson")]
        #[namespace = "atlas_wizard"]
        type Backend = super::BackendRust;

        /// Reads the start-up facts on a worker; sets `ready` when done.
        #[qinvokable]
        fn init(self: Pin<&mut Backend>);

        /// Loads a list (`languages`, `layouts`, `zones`, `networks`) on a
        /// worker; `listLoaded` follows.
        #[qinvokable]
        #[cxx_name = "loadList"]
        fn load_list(self: Pin<&mut Backend>, what: &QString);

        /// Applies a choice to the system (`language`, `keyboard`,
        /// `timezone`, `hostname`); `completed` follows.
        #[qinvokable]
        fn apply(self: Pin<&mut Backend>, what: &QString, a: &QString, b: &QString);

        /// Turns `screenReader` or `highContrast` on or off live.
        #[qinvokable]
        #[cxx_name = "setOption"]
        fn set_option(self: Pin<&mut Backend>, what: &QString, on: bool);

        /// Connects to a Wi-Fi network; `completed("wifi", ...)` follows.
        #[qinvokable]
        #[cxx_name = "connectWifi"]
        fn connect_wifi(
            self: Pin<&mut Backend>,
            device: &QString,
            ap: &QString,
            ssid: &QString,
            password: &QString,
            hidden: bool,
        );

        /// Checks the account fields on a worker; `accountChecked` follows
        /// with the same `generation`.
        #[qinvokable]
        #[cxx_name = "checkAccount"]
        fn check_account(
            self: Pin<&mut Backend>,
            generation: i32,
            full_name: &QString,
            user_name: &QString,
            password: &QString,
        );

        /// Creates the account through the helper (120 s timeout);
        /// `completed("account", ...)` follows.
        #[qinvokable]
        #[cxx_name = "createAccount"]
        fn create_account(
            self: Pin<&mut Backend>,
            user_name: &QString,
            full_name: &QString,
            password: &QString,
            autologin: bool,
        );

        /// Finish, then EndSetup; `completed("finish", ...)` follows.
        #[qinvokable]
        #[cxx_name = "finishSetup"]
        fn finish_setup(self: Pin<&mut Backend>);

        /// Sets the application font to the base font times `scale`.
        #[qinvokable]
        #[cxx_name = "setTextScale"]
        fn set_text_scale(self: &Backend, scale: f64);

        /// The user name suggested for a full name (cheap, so synchronous).
        #[qinvokable]
        #[cxx_name = "deriveUserName"]
        fn derive_user_name(self: &Backend, full_name: &QString) -> QString;

        /// Keeps the answers (JSON) in memory and on disk, atomically.
        #[qinvokable]
        #[cxx_name = "saveAnswers"]
        fn save_answers(self: Pin<&mut Backend>, json: &QString);

        /// A list finished loading; `code` is empty on success.
        #[qsignal]
        #[cxx_name = "listLoaded"]
        fn list_loaded(self: Pin<&mut Backend>, what: QString, code: QString);

        /// An action finished: `what` names it, `code` is the error code
        /// (empty when `ok`), `text` the English text for the log.
        #[qsignal]
        fn completed(
            self: Pin<&mut Backend>,
            what: QString,
            ok: bool,
            code: QString,
            text: QString,
        );

        /// The result of `checkAccount`: error codes ("" for none) and the
        /// password strength meter (-1 for no password, else 0 to 4).
        #[qsignal]
        #[cxx_name = "accountChecked"]
        fn account_checked(
            self: Pin<&mut Backend>,
            generation: i32,
            name_code: QString,
            full_name_code: QString,
            password_code: QString,
            score: i32,
        );
    }

    impl cxx_qt::Threading for Backend {}

    // Lets Rust create the object (see `atlas_backend_new` in lib.rs).
    #[namespace = "rust::cxxqtlib1"]
    unsafe extern "C++" {
        include!("cxx-qt-lib/common.h");

        #[cxx_name = "make_unique"]
        fn backend_make_unique() -> UniquePtr<Backend>;
    }
}

/// Whether `ATLAS_WIZARD_DEMO` asks for demo mode: exactly "1".
fn demo_from(value: Option<&std::ffi::OsStr>) -> bool {
    value.is_some_and(|v| v == "1")
}

/// Whether `--welcome` is among the arguments (the program name is skipped).
/// Anything else is ignored: the framework and Qt take their own options.
fn welcome_from<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    args.into_iter().skip(1).any(|a| a.as_ref() == "--welcome")
}

/// The answers file: fixed outside demo mode; demo keeps none unless a test
/// names one (the environment is never trusted otherwise).
fn answers_path(demo: bool, env: Option<std::ffi::OsString>) -> Option<PathBuf> {
    if demo {
        env.map(PathBuf::from)
    } else {
        Some(PathBuf::from(answers::DEFAULT_PATH))
    }
}

/// Where answers go and who may write them: later saves win.
struct Persist {
    path: Option<PathBuf>,
    latest: AtomicU64,
    writing: Mutex<()>,
}

pub struct BackendRust {
    demo: bool,
    welcome_mode: bool,
    finished: bool,
    ready: bool,
    startup_json: QString,
    answers_json: QString,
    languages_json: QString,
    layouts_json: QString,
    zones_json: QString,
    networks_json: QString,
    sys: Arc<dyn System>,
    answers: Arc<Mutex<Answers>>,
    persist: Arc<Persist>,
}

impl Default for BackendRust {
    fn default() -> Self {
        let demo = demo_from(std::env::var_os("ATLAS_WIZARD_DEMO").as_deref());
        // Demo keeps no state unless a test names a file.
        let path = answers_path(demo, std::env::var_os("ATLAS_WIZARD_ANSWERS"));
        let loaded = path.as_deref().map(Answers::load).unwrap_or_default();
        let sys: Arc<dyn System> = if demo {
            Arc::new(Demo)
        } else {
            Arc::new(Real::default())
        };
        Self {
            demo,
            welcome_mode: welcome_from(std::env::args_os()),
            finished: false,
            ready: false,
            startup_json: QString::default(),
            answers_json: QString::from(loaded.to_json().as_str()),
            languages_json: QString::default(),
            layouts_json: QString::default(),
            zones_json: QString::default(),
            networks_json: QString::default(),
            sys,
            answers: Arc::new(Mutex::new(loaded)),
            persist: Arc::new(Persist {
                path,
                latest: AtomicU64::new(0),
                writing: Mutex::new(()),
            }),
        }
    }
}

/// Runs `f`, turning a panic into a failure so the page is always answered.
fn guarded<T>(f: impl FnOnce() -> Res<T>) -> Res<T> {
    catch_unwind(AssertUnwindSafe(f))
        .unwrap_or_else(|_| Err(Fail::new("worker", "An internal error stopped the step.")))
}

fn q(s: &str) -> QString {
    QString::from(s)
}

fn json_of<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "[]".into())
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl qobject::Backend {
    /// Runs `work` on a worker thread and then `post` on the GUI thread. If
    /// the thread cannot start, `post` still runs, with the failure.
    fn run<T, W, P>(self: Pin<&mut Self>, name: &'static str, work: W, post: P)
    where
        T: Send + 'static,
        W: FnOnce() -> Res<T> + Send + 'static,
        P: FnOnce(Pin<&mut qobject::Backend>, Res<T>) + Send + 'static,
    {
        let qt = self.qt_thread();
        let slot = Arc::new(Mutex::new(Some(post)));
        let for_thread = Arc::clone(&slot);
        let spawned = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let result = guarded(work);
                let post = lock(&for_thread).take();
                if let Some(post) = post {
                    let _ = qt.queue(move |o| post(o, result));
                }
            });
        if let Err(e) = spawned {
            log::error!("starting {name}: {e}");
            let post = lock(&slot).take();
            if let Some(post) = post {
                post(self, Err(Fail::new("worker", e.to_string())));
            }
        }
    }

    /// Emits `completed` for `what` from a result, logging a failure.
    fn report(mut self: Pin<&mut Self>, what: &str, r: &Res<()>) {
        match r {
            Ok(()) => {
                log::info!("{what}: done");
                self.as_mut()
                    .completed(q(what), true, QString::default(), QString::default());
            }
            Err(f) => {
                log::warn!("{what}: {f}");
                self.as_mut()
                    .completed(q(what), false, q(&f.code), q(&f.text));
            }
        }
    }

    pub fn init(self: Pin<&mut Self>) {
        let sys = Arc::clone(&self.rust().sys);
        self.run(
            "start",
            move || Ok(sys.startup()),
            |mut o, r| {
                if let Ok(s) = r {
                    o.as_mut().set_startup_json(q(&json_of(&s)));
                }
                o.as_mut().set_ready(true);
            },
        );
    }

    pub fn load_list(self: Pin<&mut Self>, what: &QString) {
        let what = what.to_string();
        let sys = Arc::clone(&self.rust().sys);
        let w = what.clone();
        self.run(
            "list",
            move || match w.as_str() {
                "languages" => sys.languages().map(|v| json_of(&v)),
                "layouts" => sys.layouts().map(|v| json_of(&v)),
                "zones" => sys.zones().map(|v| json_of(&v)),
                "networks" => sys.wifi_scan().map(|v| json_of(&v)),
                _ => Err(Fail::new("bad-argument", "unknown list")),
            },
            move |mut o, r| match r {
                Ok(json) => {
                    let json = q(&json);
                    match what.as_str() {
                        "languages" => o.as_mut().set_languages_json(json),
                        "layouts" => o.as_mut().set_layouts_json(json),
                        "zones" => o.as_mut().set_zones_json(json),
                        _ => o.as_mut().set_networks_json(json),
                    }
                    o.as_mut().list_loaded(q(&what), QString::default());
                }
                Err(f) => {
                    log::warn!("list {what}: {f}");
                    o.as_mut().list_loaded(q(&what), q(&f.code));
                }
            },
        );
    }

    pub fn apply(self: Pin<&mut Self>, what: &QString, a: &QString, b: &QString) {
        let (what, a, b) = (what.to_string(), a.to_string(), b.to_string());
        let sys = Arc::clone(&self.rust().sys);
        let w = what.clone();
        self.run(
            "apply",
            move || match w.as_str() {
                "language" => sys.set_language(&a),
                "keyboard" => sys.set_keyboard(&a, &b),
                "timezone" => sys.set_timezone(&a),
                "hostname" => sys.set_hostname(&a),
                _ => Err(Fail::new("bad-argument", "unknown setting")),
            },
            move |mut o, r| o.as_mut().report(&what, &r),
        );
    }

    pub fn set_option(self: Pin<&mut Self>, what: &QString, on: bool) {
        let what = what.to_string();
        let sys = Arc::clone(&self.rust().sys);
        let w = what.clone();
        self.run(
            "option",
            move || match w.as_str() {
                "screenReader" => sys.screen_reader(on),
                "highContrast" => sys.high_contrast(on),
                _ => Err(Fail::new("bad-argument", "unknown option")),
            },
            move |mut o, r| o.as_mut().report(&what, &r),
        );
    }

    pub fn connect_wifi(
        self: Pin<&mut Self>,
        device: &QString,
        ap: &QString,
        ssid: &QString,
        password: &QString,
        hidden: bool,
    ) {
        let (device, ap, ssid) = (device.to_string(), ap.to_string(), ssid.to_string());
        let password = Zeroizing::new(password.to_string());
        let sys = Arc::clone(&self.rust().sys);
        // the SSID is not secret; the password is never logged
        log::info!("wifi: connecting to {ssid:?}");
        self.run(
            "wifi",
            move || sys.wifi_connect(&device, &ap, &ssid, password, hidden),
            |mut o, r| o.as_mut().report("wifi", &r),
        );
    }

    pub fn check_account(
        self: Pin<&mut Self>,
        generation: i32,
        full_name: &QString,
        user_name: &QString,
        password: &QString,
    ) {
        let (full, user) = (full_name.to_string(), user_name.to_string());
        let pw = Zeroizing::new(password.to_string().into_bytes());
        self.run(
            "check",
            move || {
                let name = validate::user_name_available(
                    &user,
                    Path::new("/etc/passwd"),
                    Path::new("/etc/group"),
                )
                .err()
                .map_or("", |e| e.code());
                let full_code = validate::full_name(&full).err().map_or("", |e| e.code());
                let (pw_code, score) = if pw.is_empty() {
                    ("", -1)
                } else {
                    match password::check(&pw, &user, &full) {
                        Ok(s) => ("", i32::from(s.meter())),
                        Err(e) => (e.code(), 0),
                    }
                };
                Ok((name, full_code, pw_code, score))
            },
            move |mut o, r| {
                if let Ok((n, f, p, s)) = r {
                    o.as_mut().account_checked(generation, q(n), q(f), q(p), s);
                }
            },
        );
    }

    pub fn create_account(
        self: Pin<&mut Self>,
        user_name: &QString,
        full_name: &QString,
        password: &QString,
        autologin: bool,
    ) {
        let (user, full) = (user_name.to_string(), full_name.to_string());
        let pw = Zeroizing::new(password.to_string().into_bytes());
        let sys = Arc::clone(&self.rust().sys);
        log::info!("account: creating {user:?}");
        // the password bytes are zeroed when `pw` drops at the end of the call
        self.run(
            "account",
            move || sys.create_account(&user, &full, pw, autologin).map(|_| ()),
            |mut o, r| o.as_mut().report("account", &r),
        );
    }

    pub fn finish_setup(self: Pin<&mut Self>) {
        let choices = lock(&self.rust().answers).choices();
        let sys = Arc::clone(&self.rust().sys);
        self.run(
            "finish",
            move || {
                wizard_core::choices::validate(&choices)
                    .map_err(|e| Fail::new("choices-bad-value", e.code()))?;
                sys.finish(&choices)?;
                sys.end_setup()
            },
            |mut o, r| {
                if r.is_ok() {
                    o.as_mut().set_finished(true);
                }
                o.as_mut().report("finish", &r);
            },
        );
    }

    pub fn set_text_scale(&self, scale: f64) {
        if [1.0, 1.25, 1.5].contains(&scale) {
            // SAFETY: called on the GUI thread (a QML invokable), where Qt
            // allows changing the application font.
            unsafe { atlas_set_text_scale(scale) };
        }
    }

    pub fn derive_user_name(&self, full_name: &QString) -> QString {
        q(&validate::derive_user_name(&full_name.to_string()))
    }

    pub fn save_answers(self: Pin<&mut Self>, json: &QString) {
        let a = match Answers::from_json(&json.to_string()) {
            Ok(a) => a,
            Err(e) => {
                log::warn!("answers: refused: {e}");
                return;
            }
        };
        *lock(&self.rust().answers) = a.clone();
        let persist = Arc::clone(&self.rust().persist);
        let Some(path) = persist.path.clone() else {
            return;
        };
        let seq = persist.latest.fetch_add(1, Ordering::SeqCst) + 1;
        let spawned = std::thread::Builder::new()
            .name("answers".into())
            .spawn(move || {
                let _w = lock(&persist.writing);
                // a newer save is already waiting: skip this one
                if persist.latest.load(Ordering::SeqCst) != seq {
                    return;
                }
                if let Err(e) = a.save(&path) {
                    log::warn!("answers: {}: {e}", path.display());
                }
            });
        if let Err(e) = spawned {
            log::error!("starting answers writer: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn demo_only_for_one() {
        assert!(demo_from(Some(OsStr::new("1"))));
        assert!(!demo_from(Some(OsStr::new("0"))));
        assert!(!demo_from(Some(OsStr::new(""))));
        assert!(!demo_from(None));
    }

    #[test]
    fn answers_path_env_only_in_demo() {
        let e = || Some(std::ffi::OsString::from("/tmp/x.json"));
        assert_eq!(answers_path(true, e()), Some(PathBuf::from("/tmp/x.json")));
        assert_eq!(answers_path(true, None), None);
        assert_eq!(
            answers_path(false, e()),
            Some(PathBuf::from(answers::DEFAULT_PATH))
        );
    }

    #[test]
    fn welcome_flag() {
        assert!(welcome_from(["atlas-wizard", "--welcome"]));
        assert!(!welcome_from(["atlas-wizard"]));
        // The program name is never a flag.
        assert!(!welcome_from(["--welcome"]));
    }
}

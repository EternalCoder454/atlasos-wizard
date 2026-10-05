//! The app-wide QObject: how the wizard was started and whether it is done.
//! Slow work runs on a worker thread and posts back through `qt_thread()`,
//! so the GUI thread never blocks.

#[cxx_qt::bridge]
pub mod qobject {
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
        #[namespace = "atlas_wizard"]
        type Backend = super::BackendRust;
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

pub struct BackendRust {
    demo: bool,
    welcome_mode: bool,
    finished: bool,
}

impl Default for BackendRust {
    fn default() -> Self {
        Self {
            demo: demo_from(std::env::var_os("ATLAS_WIZARD_DEMO").as_deref()),
            welcome_mode: welcome_from(std::env::args_os()),
            finished: false,
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
    fn welcome_flag() {
        assert!(welcome_from(["atlas-wizard", "--welcome"]));
        assert!(!welcome_from(["atlas-wizard"]));
        // The program name is never a flag.
        assert!(!welcome_from(["--welcome"]));
    }
}

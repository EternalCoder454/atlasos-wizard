//! `telamon-wizard-boot prepare | fallback`; see the library for what they do.

use std::process::ExitCode;
use telamon_framework_core::AppInfo;
use wizard_boot::cmd::{Runner, SystemRunner};
use wizard_boot::console::Tty;
use wizard_boot::fallback::{self, Outcome};
use wizard_boot::paths::Paths;
use wizard_boot::{prepare, sig};

/// The program that runs the external commands: the real one, or in a test
/// build with a test root the fake that edits files under the root.
fn runner(paths: &Paths) -> Box<dyn Runner> {
    #[cfg(feature = "test-root")]
    if std::env::var_os("TELAMON_WIZARD_TEST_ROOT").is_some() {
        let fake = wizard_boot::fake::Fake::new(paths);
        let fake = match std::env::var("TELAMON_WIZARD_TEST_FAIL") {
            Ok(list) => fake.failing_list(&list),
            Err(_) => fake,
        };
        return Box::new(fake);
    }
    let _ = paths;
    Box::new(SystemRunner)
}

fn main() -> ExitCode {
    // The journal identifier is `telamon-wizard-boot` (from the id below).
    telamon_framework_core::log::init(&AppInfo {
        name: "Telamon Wizard Boot".into(),
        id: "net.eterneon.telamon.wizard-boot".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        repo: "atlasos-wizard".into(),
    });
    let mut args = std::env::args().skip(1);
    let (Some(cmd), None) = (args.next(), args.next()) else {
        eprintln!("usage: telamon-wizard-boot prepare | fallback");
        return ExitCode::from(2);
    };
    let paths = Paths::system();
    let run = runner(&paths);
    match cmd.as_str() {
        "prepare" => {
            // Always exit 0. A failed boot unit must never be what keeps a
            // machine from reaching a login screen, so a problem is logged
            // (and a panic caught) instead of failing the unit.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                prepare::run(&paths, run.as_ref())
            }));
            if result.is_err() {
                log::error!("prepare panicked; the boot goes on");
            }
            ExitCode::SUCCESS
        }
        "fallback" => {
            // The password is typed into this process: no core dump of it,
            // whatever the system's core pattern (the unit also sets
            // LimitCORE=0). Not fatal: the account matters more.
            if let Err(e) = rustix::process::set_dumpable_behavior(
                rustix::process::DumpableBehavior::NotDumpable,
            ) {
                log::error!("cannot turn core dumps off: {e}");
            }
            sig::install();
            let mut tty = Tty::new();
            match fallback::run(&paths, run.as_ref(), &mut tty) {
                Outcome::Done | Outcome::Terminated => ExitCode::SUCCESS,
                // systemd restarts the unit (Restart=on-failure, limited).
                Outcome::Hangup => ExitCode::from(1),
            }
        }
        _ => {
            eprintln!("usage: telamon-wizard-boot prepare | fallback");
            ExitCode::from(2)
        }
    }
}

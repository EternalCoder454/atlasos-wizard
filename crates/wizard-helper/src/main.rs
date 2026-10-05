//! `atlas-wizard-helper`: the D-Bus activated root helper of the AtlasOS
//! Wizard. With the argument `apply-user-settings` (only ever started by the
//! helper itself, as the new account) it writes that account's settings.

use std::process::ExitCode;
use std::sync::Arc;

use atlas_framework_core::app_info;
use wizard_helper::paths::Paths;
use wizard_helper::service::{Service, serve};
use wizard_helper::{apply, system_core};

fn main() -> ExitCode {
    let app = app_info! {
        name: "Atlas Wizard Helper",
        id: "net.eterneon.atlas.wizard-helper",
        repo: "atlasos-wizard",
    };
    // journal identifier: atlas-wizard-helper (stderr without a journal)
    atlas_framework_core::log::init(&app);

    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] => run_service(),
        ["apply-user-settings"] => ExitCode::from(u8::try_from(apply::run_child()).unwrap_or(1)),
        _ => {
            eprintln!("usage: atlas-wizard-helper (started by D-Bus; no arguments)");
            ExitCode::from(2)
        }
    }
}

fn run_service() -> ExitCode {
    // two workers: the helper is small and mostly waiting
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            log::error!("cannot start the runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let paths = Paths::from_env();
    let result = rt.block_on(async {
        let conn = zbus::connection::Builder::system()?.build().await?;
        let core = system_core(paths, &conn);
        serve(conn, Service::new(Arc::new(core))).await
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::error!("the helper stopped: {e}");
            ExitCode::FAILURE
        }
    }
}

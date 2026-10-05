//! AtlasOS Setup, Rust side. `cpp/main.cpp` only hands over to the framework;
//! every QObject QML talks to is defined here.

mod backend;

use std::ffi::c_void;

// cxx-qt-build's generated initializer calls into cxx-qt-lib; keep the crate
// linked even while no bridge uses one of its types.
extern crate cxx_qt_lib;

// Who this app is, for atlas-framework: the framework takes the names, the
// logger and the crash hooks from it. The ID is the desktop file, the icon
// and the single-instance D-Bus name.
atlas_framework_ui::app! {
    name: "AtlasOS Setup",
    id: "net.eterneon.atlas.wizard",
    repo: "atlasos-wizard",
    // The oldest Atlas.Ui this app works with; the spec's Requires says the same.
    ui: "1.4.0",
}

/// Called once from `main.cpp`. Returns the `Backend` QObject, which C++ hands
/// to the QML engine. Ownership passes to the caller (a QObject with no parent).
#[unsafe(no_mangle)]
pub extern "C" fn atlas_backend_new() -> *mut c_void {
    backend::qobject::backend_make_unique().into_raw().cast()
}

// The framework starts the app (atlas-framework-ui, include/atlas/app.h): Qt,
// the app ID and names, one instance per session, logging and crash hooks.
// Then it loads the QML module's Main with the Rust backend. All app logic is
// in Rust (src/); this file only glues.

// Defined in src/lib.rs.
extern "C" void *atlas_backend_new();
// atlas-framework-ui (include/atlas/app.h), linked in with the Rust library;
// declared here as that header's comment allows.
extern "C" int atlas_app_run(int argc, char *argv[], const char *qmlModule, const char *qmlType, void *(*makeBackend)());

int main(int argc, char *argv[])
{
    return atlas_app_run(argc, argv, "net.eterneon.atlas.wizard", "Main", atlas_backend_new);
}

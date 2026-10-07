// The framework starts the app (telamon-framework-ui, include/telamon/app.h): Qt,
// the app ID and names, one instance per session, logging and crash hooks.
// Then it loads the QML module's Main with the Rust backend. All app logic is
// in Rust (src/); this file only glues.

// Defined in src/lib.rs.
extern "C" void *telamon_backend_new();
// telamon-framework-ui (include/telamon/app.h), linked in with the Rust library;
// declared here as that header's comment allows.
extern "C" int telamon_app_run(int argc, char *argv[], const char *qmlModule, const char *qmlType, void *(*makeBackend)());

#include <QGuiApplication>
#include <QFont>

// Sets the application font to its first-seen size times `scale`. Kirigami's
// and Telamon.Ui's sizes come from the application font, so they follow.
extern "C" void telamon_set_text_scale(double scale)
{
    static const qreal base = QGuiApplication::font().pointSizeF();
    QFont f = QGuiApplication::font();
    f.setPointSizeF(base * scale);
    QGuiApplication::setFont(f);
}

int main(int argc, char *argv[])
{
    return telamon_app_run(argc, argv, "net.eterneon.telamon.wizard", "Main", telamon_backend_new);
}

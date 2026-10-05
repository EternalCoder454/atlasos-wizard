import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

AtlasWindow {
    id: root

    // The Rust backend; atlas_app_run sets it.
    required property var backend

    title: AtlasApp.name
    width: Kirigami.Units.gridUnit * 40
    height: Kirigami.Units.gridUnit * 30
    visibility: Window.FullScreen
    visible: true
    LayoutMirroring.enabled: Qt.application.layoutDirection === Qt.RightToLeft
    LayoutMirroring.childrenInherit: true

    // The window stays until the wizard is done: it is the whole session.
    onClosing: close => {
        if (!root.backend.finished) {
            close.accepted = false;
        }
    }

    WelcomePage {
        anchors.fill: parent
        backend: root.backend
    }
}

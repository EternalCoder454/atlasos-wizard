import QtQuick
import Atlas.Ui

// TODO: the first-login extras (fingerprint, PIN) of `atlas-wizard --welcome`.
Item {
    AtlasEmptyState {
        anchors.centerIn: parent
        title: qsTr("Welcome Back")
        text: qsTr("Fingerprint and PIN setup will appear here.")
    }
}

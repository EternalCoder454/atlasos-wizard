import QtQuick
import Telamon.Ui

// TODO: the first-login extras (fingerprint, PIN) of `atlas-wizard --welcome`.
Item {
    TelamonEmptyState {
        anchors.centerIn: parent
        title: qsTr("Welcome Back")
        text: qsTr("Fingerprint and PIN setup will appear here.")
    }
}

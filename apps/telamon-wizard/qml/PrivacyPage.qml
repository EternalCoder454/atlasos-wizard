import QtQuick
import QtQuick.Layouts
import Telamon.Ui

WizardPage {
    id: page

    property bool crashReports: false
    signal toggled(bool on)

    title: qsTr("Privacy")
    subtitle: qsTr("Help improve Telamon OS, only if you want to.")

    Section {
        Layout.fillWidth: true
        SectionRow {
            title: qsTr("Send Crash Reports")
            subtitle: qsTr("When an app crashes, send what went wrong: the app and its version, the error, and the part of the program that was running. Never your files, passwords or personal data.")
            showSwitch: true
            switchChecked: page.crashReports
            onSwitchToggled: checked => page.toggled(checked)
        }
    }
    Item {
        Layout.fillHeight: true
    }
}

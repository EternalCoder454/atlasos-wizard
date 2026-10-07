import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Telamon.Ui

WizardPage {
    id: page

    property bool screenReader: false
    property bool largerText: false
    property bool highContrast: false
    property bool highContrastAvailable: false
    signal optionToggled(string what, bool on)

    stepId: "welcome"
    title: qsTr("Welcome to Telamon OS")
    subtitle: qsTr("Let's set up your computer. This takes a few minutes.")

    Kirigami.Icon {
        Layout.alignment: Qt.AlignHCenter
        Layout.preferredWidth: Kirigami.Units.iconSizes.enormous
        Layout.preferredHeight: Kirigami.Units.iconSizes.enormous
        source: "telamon"
        fallback: "distributor-logo"
    }
    Section {
        Layout.fillWidth: true
        title: qsTr("Accessibility")
        SectionRow {
            title: qsTr("Screen Reader")
            subtitle: qsTr("Read the screen aloud. Meta+Alt+S turns it on from anywhere.")
            showSwitch: true
            switchChecked: page.screenReader
            onSwitchToggled: checked => page.optionToggled("screenReader", checked)
        }
        SectionRow {
            title: qsTr("Larger Text")
            showSwitch: true
            switchChecked: page.largerText
            onSwitchToggled: checked => page.optionToggled("largerText", checked)
        }
        SectionRow {
            visible: page.highContrastAvailable
            title: qsTr("High Contrast")
            showSwitch: true
            switchChecked: page.highContrast
            onSwitchToggled: checked => page.optionToggled("highContrast", checked)
        }
    }
    Item {
        Layout.fillHeight: true
    }
}

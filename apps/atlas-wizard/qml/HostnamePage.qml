import QtQuick
import QtQuick.Layouts
import Atlas.Ui

WizardPage {
    id: page

    property string hostname
    readonly property bool valid: /^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$/.test(hostname)
    signal edited

    stepId: "hostname"
    title: qsTr("Computer Name")
    subtitle: qsTr("This is how the computer appears on your network.")
    canAdvance: valid
    skippable: errorText !== ""

    AtlasTextField {
        Layout.fillWidth: true
        placeholderText: qsTr("Computer Name")
        text: page.hostname
        maximumLength: 63
        errorText: page.hostname !== "" && !page.valid ? qsTr("Use lowercase letters, digits and hyphens, not starting or ending with a hyphen.") : ""
        Accessible.name: qsTr("Computer Name")
        onTextEdited: {
            page.hostname = text.toLowerCase();
            page.edited();
        }
    }
    Item {
        Layout.fillHeight: true
    }
}

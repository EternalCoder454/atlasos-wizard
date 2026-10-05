pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

WizardPage {
    id: page

    property string look: "light"
    property string accent: "#6858E2" // atlas-lint: allow-raw
    readonly property var accents: [
        { color: "#6858E2", name: qsTr("Violet") }, { color: "#E93A9A", name: qsTr("Pink") }, // atlas-lint: allow-raw
        { color: "#E93D58", name: qsTr("Red") }, { color: "#E9643A", name: qsTr("Orange") }, // atlas-lint: allow-raw
        { color: "#E8CB2D", name: qsTr("Yellow") }, { color: "#3DD425", name: qsTr("Green") }, // atlas-lint: allow-raw
        { color: "#00D3B8", name: qsTr("Teal") }, { color: "#1D99F3", name: qsTr("Blue") }, // atlas-lint: allow-raw
        { color: "#9B59D0", name: qsTr("Purple") } // atlas-lint: allow-raw
    ]
    signal edited

    stepId: "appearance"
    title: qsTr("Appearance")
    subtitle: qsTr("Choose a look and an accent color. You can change them later in Settings.")

    component Mock: Item {
        id: mock
        property bool dark: false
        anchors.fill: parent
        Rectangle {
            anchors.fill: parent
            color: mock.dark ? "#1f1f27" : "#eceef2" // atlas-lint: allow-raw
        }
        Rectangle {
            x: parent.width * 0.12
            y: parent.height * 0.14
            width: parent.width * 0.55
            height: parent.height * 0.55
            radius: 6 // atlas-lint: allow-raw
            color: mock.dark ? "#2b2b36" : "#ffffff" // atlas-lint: allow-raw
            Rectangle {
                width: parent.width
                height: parent.height * 0.2
                radius: 6 // atlas-lint: allow-raw
                color: page.accent
            }
        }
        Rectangle {
            anchors.bottom: parent.bottom
            width: parent.width
            height: parent.height * 0.14
            color: mock.dark ? "#14141a" : "#d9dce3" // atlas-lint: allow-raw
        }
    }

    RowLayout {
        Layout.alignment: Qt.AlignHCenter
        spacing: Kirigami.Units.largeSpacing
        WizardChoiceCard {
            text: qsTr("AtlasOS Light")
            checked: page.look === "light"
            onClicked: {
                page.look = "light";
                page.edited();
            }
            Mock {
                dark: false
            }
        }
        WizardChoiceCard {
            text: qsTr("AtlasOS Dark")
            checked: page.look === "dark"
            onClicked: {
                page.look = "dark";
                page.edited();
            }
            Mock {
                dark: true
            }
        }
    }
    AtlasLabel {
        textStyle: AtlasLabel.Heading
        text: qsTr("Accent Color")
    }
    WizardAccentPicker {
        id: picker
        model: page.accents
        currentIndex: Math.max(0, page.accents.findIndex(a => a.color.toLowerCase() === page.accent.toLowerCase()))
        onActivated: i => {
            page.accent = page.accents[i].color;
            page.edited();
        }
    }
    Item {
        Layout.fillHeight: true
    }
}

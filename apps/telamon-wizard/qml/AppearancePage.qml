pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import Telamon.Ui

WizardPage {
    id: page

    property string look: "light"
    property string accent: "#6858E2" // telamon-lint: allow-raw
    // The offered accent colours (wizard-core's ACCENTS), violet first.
    // telamon-lint: allow-raw
    readonly property var colors: ["#6858E2", "#E93A9A", "#E93D58", "#E9643A", "#E8CB2D", "#3DD425", "#00D3B8", "#1D99F3", "#9B59D0"]
    readonly property var names: [qsTr("Violet"), qsTr("Pink"), qsTr("Red"), qsTr("Orange"), qsTr("Yellow"), qsTr("Green"), qsTr("Teal"), qsTr("Blue"), qsTr("Purple")]
    readonly property var accents: colors.map((c, i) => ({
                color: c,
                name: names[i]
            }))
    readonly property int accentIndex: Math.max(0, page.accents.findIndex(a => a.color.toLowerCase() === page.accent.toLowerCase()))
    signal edited

    title: qsTr("Appearance")
    subtitle: qsTr("Choose a look and an accent color. You can change them later in Settings.")

    QQC2.ButtonGroup {
        id: looks
    }
    RowLayout {
        Layout.fillWidth: true
        Layout.topMargin: TelamonStyle.spacingLarge
        spacing: TelamonStyle.spacingXXLarge
        Item {
            Layout.fillWidth: true
        }
        Repeater {
            model: [
                {
                    id: "light",
                    name: qsTr("Telamon OS Light"),
                    dark: false
                },
                {
                    id: "dark",
                    name: qsTr("Telamon OS Dark"),
                    dark: true
                }
            ]
            WizardChoiceCard {
                id: card
                required property var modelData
                Layout.preferredWidth: implicitWidth
                text: modelData.name
                QQC2.ButtonGroup.group: looks
                checked: page.look === modelData.id
                onToggled: {
                    if (checked) {
                        page.look = modelData.id;
                        page.edited();
                    }
                }
                WizardThemePreview {
                    anchors.fill: parent
                    dark: card.modelData.dark
                    accent: page.accent
                }
            }
        }
        Item {
            Layout.fillWidth: true
        }
    }
    RowLayout {
        Layout.alignment: Qt.AlignHCenter
        Layout.topMargin: TelamonStyle.spacingLarge
        spacing: TelamonStyle.spacing
        TelamonLabel {
            textStyle: TelamonLabel.Heading
            text: qsTr("Accent Color")
        }
        TelamonLabel {
            textStyle: TelamonLabel.Caption
            text: page.accents[page.accentIndex].name
        }
    }
    TelamonAccentPicker {
        Layout.alignment: Qt.AlignHCenter
        model: page.accents
        currentIndex: page.accentIndex
        Accessible.name: qsTr("Accent Color")
        onActivated: i => {
            page.accent = page.accents[i].color;
            page.edited();
        }
    }
    Item {
        Layout.fillHeight: true
    }
}

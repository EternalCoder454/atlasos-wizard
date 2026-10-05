import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

// Stand-in for AtlasChoiceCard (Atlas.Ui 1.5.0): a checkable AbstractButton
// with a picture (`source`, or items declared inside as a preview), a label,
// a check circle, a checked ring and a hover ring.
QQC2.AbstractButton {
    id: control

    property url source
    property real aspectRatio: 1.6
    default property alias preview: previewHost.data

    checkable: true
    padding: Kirigami.Units.smallSpacing
    implicitWidth: Kirigami.Units.gridUnit * 13
    implicitHeight: implicitWidth / aspectRatio + label.implicitHeight + Kirigami.Units.largeSpacing * 2
    Accessible.role: Accessible.RadioButton
    Accessible.name: text
    Accessible.checked: checked

    background: Rectangle {
        radius: Kirigami.Units.cornerRadius
        color: "transparent"
        border.width: control.checked ? 3 : (control.hovered || control.visualFocus ? 2 : 1)
        border.color: control.checked ? AtlasStyle.accent : (control.hovered || control.visualFocus ? Qt.alpha(AtlasStyle.accent, 0.5) : AtlasStyle.separator)
    }
    contentItem: ColumnLayout {
        spacing: Kirigami.Units.smallSpacing
        Item {
            id: previewHost
            Layout.fillWidth: true
            Layout.preferredHeight: width / control.aspectRatio
            clip: true
            Image {
                anchors.fill: parent
                source: control.source
                fillMode: Image.PreserveAspectCrop
                visible: control.source.toString() !== ""
            }
            Rectangle {
                width: Kirigami.Units.gridUnit * 1.2
                height: width
                radius: width / 2
                anchors.top: parent.top
                anchors.right: parent.right
                anchors.margins: Kirigami.Units.smallSpacing
                color: control.checked ? AtlasStyle.accent : Qt.alpha(Kirigami.Theme.backgroundColor, 0.7)
                border.width: 1
                border.color: control.checked ? AtlasStyle.accent : AtlasStyle.separator
                Kirigami.Icon {
                    anchors.centerIn: parent
                    width: parent.width * 0.7
                    height: width
                    source: "emblem-ok-symbolic"
                    color: "white" // atlas-lint: allow-raw
                    visible: control.checked
                }
            }
        }
        AtlasLabel {
            id: label
            Layout.alignment: Qt.AlignHCenter
            textStyle: AtlasLabel.Heading
            text: control.text
        }
    }
}

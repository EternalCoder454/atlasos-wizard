pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Templates as T
import org.kde.kirigami as Kirigami
import Telamon.Ui

// A choice shown as a picture with its name under it: TelamonChoiceCard's look
// (a check circle beside the name, a ring in the accent colour around the
// picture when chosen, a fainter one on hover and keyboard focus), but the
// picture is whatever is declared inside, not an image file, so a preview can
// be drawn with shapes. Stand-in until TelamonChoiceCard takes content. It is
// a checkable button: put the cards in a ButtonGroup so choosing one clears
// the others. A binding on `checked` survives the user's choice, as in
// TelamonChoiceCard: the new value is held for one turn of the event loop.
T.AbstractButton {
    id: control

    property real aspectRatio: 1.6
    default property alias preview: frame.data

    // The ring sits just outside the picture, inside the card's padding.
    readonly property real _ring: TelamonStyle.spacingSmall
    property bool _edit: false
    property bool _editing: false
    readonly property Binding _hold: Binding {
        target: control
        property: "checked"
        value: control._edit
        when: control._editing
        restoreMode: Binding.RestoreBinding
    }
    function _release(): void {
        control._editing = false;
    }
    onToggled: {
        control._edit = control.checked;
        control._editing = true;
        Qt.callLater(control._release);
    }

    checkable: true
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus
    padding: control._ring
    implicitWidth: Kirigami.Units.gridUnit * 17 + leftPadding + rightPadding
    implicitHeight: column.implicitHeight + topPadding + bottomPadding

    Accessible.role: Accessible.RadioButton
    Accessible.name: control.text
    Accessible.checkable: true
    Accessible.checked: control.checked

    background: Item {
        TelamonFocusRing {
            gap: 2
            radius: TelamonStyle.radiusLarge + control._ring + gap
            shown: control.visualFocus
        }
    }

    contentItem: ColumnLayout {
        id: column
        spacing: TelamonStyle.spacing

        Item {
            Layout.fillWidth: true
            Layout.preferredHeight: width / control.aspectRatio

            Rectangle {
                anchors.fill: parent
                anchors.margins: -control._ring
                radius: TelamonStyle.radiusLarge + control._ring
                color: "transparent"
                border.width: control.checked ? 3 : 2
                border.color: control.checked ? TelamonStyle.accent : control.enabled && (control.hovered || control.visualFocus) ? (TelamonStyle.highContrast ? TelamonStyle.accent : TelamonStyle.alpha(TelamonStyle.accent, 0.45)) : "transparent"
                Accessible.ignored: true
                Behavior on border.color {
                    ColorAnimation {
                        duration: TelamonStyle.durationShort
                    }
                }
                Behavior on border.width {
                    enabled: !TelamonStyle.reducedMotion
                    NumberAnimation {
                        duration: TelamonStyle.durationShort
                        easing.type: Easing.OutCubic
                    }
                }
            }
            // The picture's own frame; the declared preview fills it.
            Item {
                id: frame
                anchors.fill: parent
                Accessible.ignored: true
            }
            // A hairline edge, so a light picture stands out from a light card.
            Rectangle {
                anchors.fill: parent
                radius: TelamonStyle.radiusLarge
                color: "transparent"
                border.width: 1
                border.color: TelamonStyle.highContrast ? TelamonStyle.controlBorder : TelamonStyle.alpha(TelamonStyle.text, 0.15)
                Accessible.ignored: true
            }
        }

        RowLayout {
            Layout.alignment: Qt.AlignHCenter
            Layout.topMargin: TelamonStyle.spacingSmall
            spacing: TelamonStyle.spacing

            // The check circle: filled with the accent colour when chosen.
            Rectangle {
                id: circle
                implicitWidth: Math.round(Kirigami.Units.gridUnit * 0.9)
                implicitHeight: implicitWidth
                radius: width / 2
                color: control.checked ? TelamonStyle.accent : "transparent"
                border.width: control.checked ? 0 : 1
                border.color: TelamonStyle.controlBorder
                Accessible.ignored: true
                Behavior on color {
                    ColorAnimation {
                        duration: TelamonStyle.durationShort
                    }
                }
                WizardIcon {
                    anchors.centerIn: parent
                    width: Math.round(circle.width * 0.75)
                    height: width
                    visible: control.checked
                    source: "checkmark"
                    isMask: true
                    color: TelamonStyle.accentText
                    Accessible.ignored: true
                }
            }
            Text {
                Layout.maximumWidth: Math.max(0, column.width - circle.width - TelamonStyle.spacing)
                text: control.text
                elide: Text.ElideRight
                font.family: TelamonStyle.fontFamily
                font.pointSize: TelamonStyle.fontSizeBody
                font.weight: Font.DemiBold
                color: control.enabled ? TelamonStyle.text : TelamonStyle.textDisabled
                textFormat: Text.PlainText
                Accessible.ignored: true
            }
        }
    }
}

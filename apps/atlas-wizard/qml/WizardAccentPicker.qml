pragma ComponentBehavior: Bound
import QtQuick
import org.kde.kirigami as Kirigami
import Telamon.Ui

// Stand-in for TelamonAccentPicker (Telamon.Ui 1.5.0): `model` is colours or
// `{color, name}`; `currentIndex`, read-only `currentColor`, `activated(int)`;
// arrows, Home and End move. Accessible name: the name or "Accent color N".
FocusScope {
    id: control

    property var model: []
    property int currentIndex: 0
    readonly property color currentColor: colorAt(currentIndex)
    signal activated(int index)

    function colorAt(i: int): color {
        const m = control.model[i];
        return m === undefined ? "transparent" : (m.color !== undefined ? m.color : m);
    }
    function nameAt(i: int): string {
        const m = control.model[i];
        return m !== undefined && m.name !== undefined ? m.name : qsTr("Accent color %1").arg(i + 1);
    }
    function _go(i: int): void {
        const n = Math.max(0, Math.min(control.model.length - 1, i));
        if (n !== control.currentIndex) {
            control.currentIndex = n;
            control.activated(n);
        }
    }

    activeFocusOnTab: true
    implicitWidth: row.implicitWidth
    implicitHeight: row.implicitHeight
    Accessible.role: Accessible.RadioButton
    Accessible.name: qsTr("Accent color")
    Accessible.description: nameAt(currentIndex)
    Keys.onLeftPressed: control._go(control.currentIndex + (control.LayoutMirroring.enabled ? 1 : -1))
    Keys.onRightPressed: control._go(control.currentIndex + (control.LayoutMirroring.enabled ? -1 : 1))
    Keys.onPressed: event => {
        if (event.key === Qt.Key_Home) {
            control._go(0);
            event.accepted = true;
        } else if (event.key === Qt.Key_End) {
            control._go(control.model.length - 1);
            event.accepted = true;
        }
    }

    Row {
        id: row
        spacing: Kirigami.Units.largeSpacing
        Repeater {
            model: control.model.length
            Rectangle {
                id: swatch
                required property int index
                readonly property bool current: index === control.currentIndex
                width: Kirigami.Units.gridUnit * 2
                height: width
                radius: width / 2
                color: control.colorAt(index)
                border.width: current ? 3 : 0
                border.color: Kirigami.Theme.textColor
                Rectangle {
                    anchors.centerIn: parent
                    width: parent.width * 0.35
                    height: width
                    radius: width / 2
                    color: "white" // telamon-lint: allow-raw
                    visible: swatch.current
                }
                Rectangle {
                    anchors.fill: parent
                    anchors.margins: -4
                    radius: width / 2
                    color: "transparent"
                    border.width: 2
                    border.color: TelamonStyle.accent
                    visible: control.activeFocus && swatch.current
                }
                Accessible.role: Accessible.RadioButton
                Accessible.name: control.nameAt(index)
                Accessible.checked: current
                HoverHandler {
                    id: hover
                    cursorShape: Qt.PointingHandCursor
                }
                scale: hover.hovered ? 1.1 : 1
                TapHandler {
                    onTapped: {
                        control.forceActiveFocus();
                        control._go(swatch.index);
                    }
                }
            }
        }
    }
}

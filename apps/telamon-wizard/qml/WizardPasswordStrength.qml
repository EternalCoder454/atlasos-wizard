pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Telamon.Ui

// Stand-in for TelamonPasswordStrength (Telamon.Ui 1.5.0): `score` 0 to 4, -1 for
// nothing typed (empty bar, no label); `text` empty for the built-in label.
RowLayout {
    id: control

    property int score: -1
    property string text
    readonly property var labels: [qsTr("Very Weak"), qsTr("Weak"), qsTr("Fair"), qsTr("Good"), qsTr("Strong")]
    readonly property string label: score < 0 ? "" : (text !== "" ? text : labels[Math.min(4, score)])
    readonly property color tint: score <= 1 ? Kirigami.Theme.negativeTextColor : (score === 2 ? Kirigami.Theme.neutralTextColor : Kirigami.Theme.positiveTextColor)

    spacing: Kirigami.Units.smallSpacing
    Accessible.role: Accessible.ProgressBar
    Accessible.name: qsTr("Password strength")
    Accessible.description: score < 0 ? "" : qsTr("%1, %2 of 4").arg(label).arg(score)

    Repeater {
        model: 4
        Rectangle {
            id: seg
            required property int index
            Layout.fillWidth: true
            implicitHeight: Kirigami.Units.smallSpacing
            radius: height / 2
            color: control.score >= 0 && seg.index < Math.max(1, control.score) ? control.tint : Qt.alpha(Kirigami.Theme.textColor, 0.15)
        }
    }
    TelamonLabel {
        Layout.preferredWidth: Kirigami.Units.gridUnit * 5
        textStyle: TelamonLabel.Caption
        text: control.label
    }
}

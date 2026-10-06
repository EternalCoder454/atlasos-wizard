pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

// Stand-in for Atlas.Ui 1.5.0's AtlasOnboarding additions (same API):
// nextText/finishText/backText, busy, autoAdvance + advanceRequested(index),
// canGoBack, stepStyle. Local extra: a page with `hidden: true` is left out
// of the steps and of Back/Next.
Item {
    id: control

    enum StepStyle {
        Column,
        Dots
    }

    default property list<Item> pages
    property int currentIndex: 0
    readonly property int count: pages.length
    property bool showSteps: true
    property bool showSkip: false
    property string nextText
    property string finishText
    property string backText
    property bool busy: false
    property bool autoAdvance: true
    property bool canGoBack: true
    property int stepStyle: WizardOnboarding.StepStyle.Dots

    signal finished
    signal skipped(int index)
    signal advanceRequested(int index)

    Accessible.role: Accessible.Pane
    Accessible.name: qsTr("Setup")

    function _hidden(i: int): bool {
        return control.pages[i]["hidden"] === true;
    }
    // The visible pages' indexes, in order.
    readonly property var order: {
        const r = [];
        for (let i = 0; i < control.pages.length; ++i) {
            if (!_hidden(i)) {
                r.push(i);
            }
        }
        return r;
    }
    // How many shown pages come before this one. Counted rather than looked
    // up with indexOf: a page can hide itself while it is the current one
    // (Account does once it has made the user), and indexOf then gave -1,
    // which became position 0, so Next jumped back to the first page after
    // Welcome (Language, or Wi-Fi when the installer had set the language).
    readonly property int position: {
        let n = 0;
        for (const i of order) {
            if (i < currentIndex) {
                ++n;
            }
        }
        return n;
    }
    readonly property bool isLast: order.length === 0 || order[order.length - 1] <= currentIndex
    readonly property var page: currentIndex >= 0 && currentIndex < count ? pages[currentIndex] : null
    readonly property bool canAdvanceNow: page !== null && page["canAdvance"] !== false && !busy
    readonly property bool pageSkippable: page !== null && page["skippable"] === true

    function next(): void {
        if (control.busy) {
            return;
        }
        if (control.isLast) {
            control.finished();
        } else {
            control.currentIndex = control.order.find(i => i > control.currentIndex);
        }
    }
    function back(): void {
        if (control.canGoBack && !control.busy && control.position > 0) {
            control.currentIndex = control.order[control.position - 1];
        }
    }
    function skip(): void {
        if (control.busy || control.page === null) {
            return;
        }
        control.skipped(control.currentIndex);
        if (!control.isLast) {
            control.next();
        }
    }
    // Next was used: the app decides when to go on, or this does.
    function _nextPressed(): void {
        if (!control.canAdvanceNow) {
            return;
        }
        control.advanceRequested(control.currentIndex);
        if (control.autoAdvance) {
            control.next();
        }
    }

    onCurrentIndexChanged: priv.show(true)
    onPagesChanged: priv.show(false)
    Component.onCompleted: priv.show(false)

    QtObject {
        id: priv
        function show(animated: bool): void {
            for (let i = 0; i < control.pages.length; ++i) {
                const p = control.pages[i];
                if (p.parent !== host) {
                    p.parent = host;
                }
                p.anchors.fill = host;
                p.visible = i === control.currentIndex;
            }
            if (animated) {
                fade.restart();
            }
        }
    }

    Shortcut {
        sequence: "Alt+Left"
        enabled: control.canGoBack && !control.busy
        onActivated: control.back()
    }

    Rectangle {
        id: card
        anchors.centerIn: parent
        width: Math.min(parent.width - Kirigami.Units.gridUnit * 2, Kirigami.Units.gridUnit * 46)
        height: Math.min(parent.height - Kirigami.Units.gridUnit * 2, Kirigami.Units.gridUnit * 36)
        radius: Kirigami.Units.cornerRadius * 2
        color: Kirigami.Theme.backgroundColor
        border.width: 1
        border.color: AtlasStyle.separator

        RowLayout {
            anchors.fill: parent
            anchors.margins: Kirigami.Units.gridUnit * 1.5
            spacing: Kirigami.Units.gridUnit

            ColumnLayout {
                Layout.fillHeight: true
                Layout.preferredWidth: Kirigami.Units.gridUnit * 10
                visible: control.stepStyle === WizardOnboarding.StepStyle.Column && control.showSteps
                Repeater {
                    model: control.order
                    StepItem {
                        id: step
                        required property int index
                        required property int modelData
                        Layout.fillWidth: true
                        number: step.index + 1
                        text: String(control.pages[step.modelData]["title"] ?? "")
                        current: step.modelData === control.currentIndex
                        done: step.index < control.position
                        onClicked: if (control.canGoBack && !control.busy) control.currentIndex = step.modelData
                    }
                }
                Item {
                    Layout.fillHeight: true
                }
            }

            ColumnLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: Kirigami.Units.largeSpacing

                Row {
                    Layout.alignment: Qt.AlignHCenter
                    visible: control.stepStyle === WizardOnboarding.StepStyle.Dots && control.showSteps
                    spacing: Kirigami.Units.smallSpacing
                    Accessible.role: Accessible.ProgressBar
                    Accessible.name: qsTr("Step %1 of %2").arg(control.position + 1).arg(control.order.length)
                    Repeater {
                        model: control.order.length
                        Rectangle {
                            id: dot
                            required property int index
                            readonly property bool current: index === control.position
                            width: current ? Kirigami.Units.gridUnit * 1.4 : Kirigami.Units.gridUnit * 0.5
                            height: Kirigami.Units.gridUnit * 0.5
                            radius: height / 2
                            color: current ? AtlasStyle.accent : (index < control.position ? Qt.alpha(AtlasStyle.accent, 0.45) : Qt.alpha(Kirigami.Theme.textColor, 0.2))
                            Behavior on width {
                                NumberAnimation {
                                    duration: AtlasStyle.duration
                                }
                            }
                        }
                    }
                }

                Item {
                    id: host
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    NumberAnimation {
                        id: fade
                        target: host
                        property: "opacity"
                        from: 0
                        to: 1
                        duration: AtlasStyle.duration
                    }
                }

                Rectangle {
                    Layout.fillWidth: true
                    implicitHeight: 1
                    color: AtlasStyle.separator
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: Kirigami.Units.largeSpacing

                    SecondaryButton {
                        visible: control.canGoBack && control.position > 0
                        enabled: !control.busy
                        text: control.backText !== "" ? control.backText : qsTr("Back")
                        onClicked: control.back()
                    }
                    Item {
                        Layout.fillWidth: true
                    }
                    TextButton {
                        visible: control.showSkip || control.pageSkippable
                        enabled: !control.busy
                        text: qsTr("Skip")
                        onClicked: control.skip()
                    }
                    PrimaryButton {
                        text: control.isLast ? (control.finishText !== "" ? control.finishText : qsTr("Finish")) : (control.nextText !== "" ? control.nextText : qsTr("Next"))
                        enabled: control.canAdvanceNow
                        busy: control.busy
                        Accessible.description: control.busy ? qsTr("Busy") : ""
                        onClicked: control._nextPressed()
                    }
                }
            }
        }
    }
}

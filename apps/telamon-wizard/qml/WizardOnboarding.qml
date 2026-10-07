pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Telamon.Ui

// The wizard's TelamonOnboarding: finishText, busy, autoAdvance +
// advanceRequested(index), canGoBack as there, with the step dots only. Local
// extras: a page with `hidden: true` is left out of the steps and of
// Back/Next, and a page's `submitted` signal (Return in one of its fields)
// is Next.
Item {
    id: control

    default property list<Item> pages
    property int currentIndex: 0
    readonly property int count: pages.length
    property string finishText
    property bool busy: false
    property bool autoAdvance: true
    property bool canGoBack: true

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

    onCurrentIndexChanged: {
        priv.show(true);
        preload.restart();
    }
    onPagesChanged: priv.show(false)
    Component.onCompleted: {
        priv.show(false);
        preload.restart();
    }

    // The page's `submitted` (Return in a field) is Next. The step's item,
    // once made.
    Connections {
        target: control.page ? control.page["item"] : null
        ignoreUnknownSignals: true
        function onSubmitted(): void {
            control._nextPressed();
        }
    }
    // The next page is made a moment after this one shows, while the user
    // reads, so Next never waits for it.
    Timer {
        id: preload
        interval: 400
        onTriggered: {
            const n = control.order.find(i => i > control.currentIndex);
            if (n !== undefined) {
                control.pages[n].ensure();
            }
        }
    }

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
        radius: TelamonStyle.radiusLarge * 1.5
        color: TelamonStyle.surface
        border.width: 1
        border.color: TelamonStyle.separator

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: Kirigami.Units.gridUnit * 1.5
            spacing: TelamonStyle.spacingLarge

            Row {
                Layout.alignment: Qt.AlignHCenter
                visible: control.order.length > 1
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
                        color: current ? TelamonStyle.accent : (index < control.position ? TelamonStyle.alpha(TelamonStyle.accent, 0.45) : TelamonStyle.alpha(TelamonStyle.text, 0.2))
                        Behavior on width {
                            NumberAnimation {
                                duration: TelamonStyle.duration
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
                // A page fades in by a card-coloured cover fading out: the
                // page is drawn once and only the cover's alpha changes
                // each frame, where fading the page itself would redraw
                // it (and its text) through an offscreen layer.
                Rectangle {
                    id: cover
                    anchors.fill: parent
                    z: 100
                    color: TelamonStyle.surface
                    opacity: 0
                    visible: opacity > 0
                    Accessible.ignored: true
                }
                NumberAnimation {
                    id: fade
                    target: cover
                    property: "opacity"
                    from: 1
                    to: 0
                    duration: TelamonStyle.duration
                }
            }

            Rectangle {
                Layout.fillWidth: true
                implicitHeight: 1
                color: TelamonStyle.separator
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.largeSpacing

                SecondaryButton {
                    visible: control.canGoBack && control.position > 0
                    enabled: !control.busy
                    text: qsTr("Back")
                    onClicked: control.back()
                }
                Item {
                    Layout.fillWidth: true
                }
                TextButton {
                    visible: control.pageSkippable
                    enabled: !control.busy
                    text: qsTr("Skip")
                    onClicked: control.skip()
                }
                PrimaryButton {
                    text: control.isLast ? (control.finishText !== "" ? control.finishText : qsTr("Finish")) : qsTr("Next")
                    enabled: control.canAdvanceNow
                    busy: control.busy
                    Accessible.description: control.busy ? qsTr("Busy") : ""
                    onClicked: control._nextPressed()
                }
            }
        }
    }
}

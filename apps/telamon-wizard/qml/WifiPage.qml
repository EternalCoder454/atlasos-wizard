import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Telamon.Ui

WizardPage {
    id: page

    required property var backend
    property bool connected: false
    property bool scanning: false
    property bool connecting: false
    property var networks: []
    property int selected: -1
    property bool hiddenNetwork: false
    readonly property var current: selected >= 0 && selected < networks.length ? networks[selected] : null

    stepId: "wifi"
    title: qsTr("Wi-Fi")
    subtitle: qsTr("Connect to the internet for updates, or skip this for now.")
    skippable: true
    canAdvance: !connecting

    function scan(): void {
        scanning = true;
        errorText = "";
        backend.loadList("networks");
    }
    function load(): void {
        if (networks.length === 0 && !scanning) {
            scan();
        }
    }
    function connectDone(ok: bool, code: string): void {
        connecting = false;
        pass.text = "";
        if (ok) {
            connected = true;
            errorText = "";
        } else {
            errorText = page.errorFor(code);
        }
    }
    function _connect(): void {
        connecting = true;
        errorText = "";
        if (hiddenNetwork) {
            backend.connectWifi("", "/", hiddenSsid.text, pass.text, true);
        } else {
            backend.connectWifi(current.device, current.ap, current.ssid, pass.text, false);
        }
    }
    Connections {
        target: page.backend
        function onListLoaded(what: string, code: string): void {
            if (what !== "networks") {
                return;
            }
            page.scanning = false;
            if (code !== "") {
                page.errorText = page.errorFor(code);
                return;
            }
            page.networks = JSON.parse(page.backend.networksJson);
            page.selected = -1;
        }
    }

    InfoBanner {
        Layout.fillWidth: true
        type: "info"
        text: qsTr("You're connected.")
        shown: page.connected
    }
    RowLayout {
        Layout.fillWidth: true
        TelamonLabel {
            Layout.fillWidth: true
            textStyle: TelamonLabel.Heading
            text: qsTr("Available Networks")
        }
        SecondaryButton {
            text: qsTr("Scan Again")
            busy: page.scanning
            onClicked: page.scan()
        }
    }
    TelamonListView {
        Layout.fillWidth: true
        Layout.fillHeight: true
        Layout.minimumHeight: Kirigami.Units.gridUnit * 6
        visible: !page.hiddenNetwork
        Accessible.name: qsTr("Available Networks")
        textRole: "ssid"
        subtitleRole: "info"
        placeholderText: page.scanning ? qsTr("Looking for networks…") : qsTr("No networks found.")
        model: page.networks.map(n => ({ ssid: n.ssid, info: (n.secure ? qsTr("Secured") : qsTr("Open")) + " · " + qsTr("%1% signal").arg(n.strength) }))
        onCurrentIndexChanged: page.selected = currentIndex
    }
    TelamonTextField {
        id: hiddenSsid
        Layout.fillWidth: true
        visible: page.hiddenNetwork
        placeholderText: qsTr("Network Name")
        maximumLength: 32
    }
    TelamonPasswordField {
        id: pass
        Layout.fillWidth: true
        visible: page.hiddenNetwork || (page.current !== null && page.current.secure)
        placeholderText: qsTr("Password")
        onAccepted: if (connectButton.enabled) page._connect()
    }
    RowLayout {
        Layout.fillWidth: true
        TelamonCheckBox {
            text: qsTr("Hidden Network")
            checked: page.hiddenNetwork
            onToggled: page.hiddenNetwork = checked
        }
        Item {
            Layout.fillWidth: true
        }
        PrimaryButton {
            id: connectButton
            text: qsTr("Connect")
            busy: page.connecting
            enabled: !page.connecting && (page.hiddenNetwork ? hiddenSsid.text.length > 0 : page.current !== null) && (!pass.visible || pass.text.length > 0 || (!page.hiddenNetwork && !page.current.secure))
            onClicked: page._connect()
        }
    }
}

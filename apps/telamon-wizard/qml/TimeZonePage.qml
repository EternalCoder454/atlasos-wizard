import QtQuick
import QtQuick.Layouts

WizardPage {
    id: page

    required property var backend
    property string zone
    signal chosen(string id)

    stepId: "timezone"
    title: qsTr("Time Zone")
    subtitle: qsTr("Choose where you are, so the clock is right.")
    skippable: errorText !== ""

    property var items: []
    property bool loading: true

    function load(): void {
        if (items.length === 0) {
            backend.loadList("zones");
        }
    }
    Connections {
        target: page.backend
        function onListLoaded(what: string, code: string): void {
            if (what !== "zones") {
                return;
            }
            page.loading = false;
            if (code !== "") {
                page.errorText = page.errorFor(code);
                return;
            }
            page.items = JSON.parse(page.backend.zonesJson).map(z => ({ id: z.id, text: z.id.replace(/_/g, " "), subtitle: z.comment }));
        }
    }

    WizardPicker {
        Layout.fillWidth: true
        Layout.fillHeight: true
        items: page.items
        loading: page.loading
        currentId: page.zone
        placeholder: qsTr("Search Time Zones")
        onPicked: id => page.chosen(id)
    }
}

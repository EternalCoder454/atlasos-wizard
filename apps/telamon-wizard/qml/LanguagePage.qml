import QtQuick
import QtQuick.Layouts

WizardPage {
    id: page

    required property var backend
    property string language
    signal chosen(string locale)

    title: qsTr("Language")
    subtitle: qsTr("Choose the language for menus and messages.")
    skippable: errorText !== ""

    property var items: []
    property bool loading: true

    function load(): void {
        if (items.length === 0) {
            backend.loadList("languages");
        }
    }
    function _label(code: string): string {
        const l = Qt.locale(code.split(".")[0].split("@")[0]);
        const n = l.nativeLanguageName;
        if (!n) {
            return code;
        }
        const t = l.nativeTerritoryName;
        return n.charAt(0).toUpperCase() + n.slice(1) + (t ? " (" + t + ")" : "");
    }
    Connections {
        target: page.backend
        function onListLoaded(what: string, code: string): void {
            if (what !== "languages") {
                return;
            }
            page.loading = false;
            if (code !== "") {
                page.errorText = page.errorFor(code);
                return;
            }
            const rows = JSON.parse(page.backend.languagesJson).map(c => ({
                        id: c,
                        text: page._label(c),
                        subtitle: c.split(".")[0]
                    }));
            rows.sort((a, b) => a.text.localeCompare(b.text));
            page.items = rows;
        }
    }

    WizardPicker {
        Layout.fillWidth: true
        Layout.fillHeight: true
        items: page.items
        loading: page.loading
        currentId: page.language
        placeholder: qsTr("Search Languages")
        onPicked: id => page.chosen(id)
    }
}

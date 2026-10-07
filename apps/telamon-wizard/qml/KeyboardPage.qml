import QtQuick
import QtQuick.Layouts
import Telamon.Ui

WizardPage {
    id: page

    required property var backend
    property string layout
    property string variant
    signal chosen(string layout, string variant)

    title: qsTr("Keyboard")
    subtitle: qsTr("Choose your keyboard layout.")
    skippable: errorText !== ""

    property var layouts: []
    property var items: []
    property bool loading: true
    readonly property var variants: {
        for (const l of layouts) {
            if (l.name === layout) {
                return l.variants;
            }
        }
        return [];
    }

    function load(): void {
        if (layouts.length === 0) {
            backend.loadList("layouts");
        }
    }
    Connections {
        target: page.backend
        function onListLoaded(what: string, code: string): void {
            if (what !== "layouts") {
                return;
            }
            page.loading = false;
            if (code !== "") {
                page.errorText = page.errorFor(code);
                return;
            }
            page.layouts = JSON.parse(page.backend.layoutsJson);
            page.items = page.layouts.map(l => ({
                        id: l.name,
                        text: l.description,
                        subtitle: l.name
                    }));
        }
    }

    WizardPicker {
        Layout.fillWidth: true
        Layout.fillHeight: true
        items: page.items
        loading: page.loading
        currentId: page.layout
        placeholder: qsTr("Search Layouts")
        onPicked: id => page.chosen(id, "")
    }
    RowLayout {
        Layout.fillWidth: true
        visible: page.variants.length > 0
        spacing: TelamonStyle.spacing
        TelamonLabel {
            text: qsTr("Variant")
        }
        TelamonComboBox {
            id: variantBox
            Layout.fillWidth: true
            Accessible.name: qsTr("Variant")
            model: [qsTr("Default")].concat(page.variants.map(v => v.description))
            currentIndex: {
                for (let i = 0; i < page.variants.length; ++i) {
                    if (page.variants[i].name === page.variant) {
                        return i + 1;
                    }
                }
                return 0;
            }
            onActivated: i => page.chosen(page.layout, i === 0 ? "" : page.variants[i - 1].name)
        }
    }
}

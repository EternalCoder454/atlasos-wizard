import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Telamon.Ui

// A filterable list: `items` is an array of {id, text, subtitle}; `currentId`
// is the chosen one; `picked(id)` fires when the user chooses.
ColumnLayout {
    id: control

    property var items: []
    property string currentId
    property string placeholder: qsTr("Search")
    property string emptyText: qsTr("Nothing matches.")
    property bool loading: false
    signal picked(string id)

    spacing: Kirigami.Units.smallSpacing
    Accessible.name: placeholder

    function _fill(): void {
        const f = filter.text.trim().toLowerCase();
        model.clear();
        let at = -1;
        for (const it of control.items) {
            if (f === "" || it.text.toLowerCase().indexOf(f) >= 0 || (it.subtitle ?? "").toLowerCase().indexOf(f) >= 0) {
                if (it.id === control.currentId) {
                    at = model.count;
                }
                model.append({ rid: it.id, text: it.text, subtitle: it.subtitle ?? "" });
            }
        }
        list.populating = true;
        if (at >= 0) {
            list.select(at);
            list.positionViewAtIndex(at, ListView.Center);
        } else {
            list.clearSelection();
        }
        list.populating = false;
    }
    onItemsChanged: _fill()
    onCurrentIdChanged: _fill()

    TelamonTextField {
        id: filter
        Layout.fillWidth: true
        placeholderText: control.placeholder
        clearable: true
        onTextChanged: control._fill()
        Accessible.name: control.placeholder
    }

    TelamonListView {
        id: list
        Layout.fillWidth: true
        Layout.fillHeight: true
        Layout.minimumHeight: Kirigami.Units.gridUnit * 8
        property bool populating: false
        subtitleRole: "subtitle"
        placeholderText: control.loading ? qsTr("Loading…") : control.emptyText
        Accessible.name: control.placeholder
        model: ListModel {
            id: model
        }
        onCurrentIndexChanged: {
            if (!populating && currentIndex >= 0 && currentIndex < model.count) {
                const id = model.get(currentIndex).rid;
                if (id !== control.currentId) {
                    control.picked(id);
                }
            }
        }
    }
}

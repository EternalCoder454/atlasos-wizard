import QtQuick
import QtQuick.Layouts
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

    spacing: TelamonStyle.spacing
    Accessible.name: placeholder

    function _fill(): void {
        const f = filter.text.trim().toLowerCase();
        rows.clear();
        let at = -1;
        for (const it of control.items) {
            if (f === "" || it.text.toLowerCase().indexOf(f) >= 0 || (it.subtitle ?? "").toLowerCase().indexOf(f) >= 0) {
                if (it.id === control.currentId) {
                    at = rows.count;
                }
                rows.append({
                    rid: it.id,
                    text: it.text,
                    subtitle: it.subtitle ?? ""
                });
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
        Layout.minimumHeight: TelamonStyle.rowHeight * 4
        property bool populating: false
        subtitleRole: "subtitle"
        Accessible.name: control.placeholder
        status: control.loading ? TelamonStatus.Loading : (rows.count === 0 ? (filter.text !== "" ? TelamonStatus.NoResults : TelamonStatus.Empty) : TelamonStatus.Ready)
        statusText: rows.count === 0 && !control.loading && filter.text !== "" ? control.emptyText : ""
        model: ListModel {
            id: rows
        }
        // The selection, not `currentIndex`: that starts at 0, so a click on
        // the first row changes nothing in it.
        onSelectedIndexesChanged: {
            if (!populating && selectedIndexes.length > 0 && selectedIndexes[0] < rows.count) {
                const id = rows.get(selectedIndexes[0]).rid;
                if (id !== control.currentId) {
                    control.picked(id);
                }
            }
        }
    }
}

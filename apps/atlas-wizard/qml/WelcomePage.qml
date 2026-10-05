import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

// Placeholder: the first page. The real pages come with the wizard's flow.
Item {
    id: page

    required property var backend

    signal startRequested

    ColumnLayout {
        anchors.centerIn: parent
        spacing: Kirigami.Units.largeSpacing

        Kirigami.Icon {
            Layout.alignment: Qt.AlignHCenter
            Layout.preferredWidth: Kirigami.Units.iconSizes.enormous
            Layout.preferredHeight: Kirigami.Units.iconSizes.enormous
            source: "atlasos"
            fallback: "distributor-logo"
        }

        AtlasLabel {
            Layout.alignment: Qt.AlignHCenter
            textStyle: AtlasLabel.Title
            text: qsTr("Welcome to AtlasOS")
        }

        PrimaryButton {
            Layout.alignment: Qt.AlignHCenter
            text: qsTr("Get Started")
            onClicked: page.startRequested()
        }
    }
}

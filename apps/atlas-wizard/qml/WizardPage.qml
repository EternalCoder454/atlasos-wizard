import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

// One page of the wizard. WizardOnboarding reads `title`, `canAdvance`,
// `skippable` and `hidden` from it. Children go in the body column.
Item {
    id: page

    property string stepId
    property string title
    property string subtitle
    property string errorText
    property bool canAdvance: true
    property bool skippable: false
    property bool hidden: false
    default property alias content: body.data

    ErrorText {
        id: errs
    }
    function errorFor(code: string): string {
        return errs.text(code);
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: Kirigami.Units.largeSpacing

        AtlasLabel {
            Layout.fillWidth: true
            textStyle: AtlasLabel.Title
            text: page.title
            wrapMode: Text.WordWrap
            Accessible.role: Accessible.Heading
        }
        AtlasLabel {
            Layout.fillWidth: true
            visible: page.subtitle !== ""
            text: page.subtitle
            wrapMode: Text.WordWrap
            color: AtlasStyle.textMuted
        }
        InfoBanner {
            Layout.fillWidth: true
            type: "error"
            text: page.errorText
            shown: page.errorText !== ""
        }
        ColumnLayout {
            id: body
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: Kirigami.Units.largeSpacing
        }
    }
}

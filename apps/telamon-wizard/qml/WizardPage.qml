import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import Telamon.Ui

// One page of the wizard, made by its WizardStep when the page is first
// needed. WizardOnboarding reads `title`, `canAdvance` and `skippable` from
// it (through the step). Children go in the body column. The page fills the
// card; when its content needs more room (a small screen, larger text) it
// scrolls instead of being cut off.
Item {
    id: page

    property string title
    property string subtitle
    property string errorText
    property bool canAdvance: true
    property bool skippable: false
    // Return in a field of the page: go on, as Next does.
    signal submitted
    default property alias content: body.data

    ErrorText {
        id: errs
    }
    function errorFor(code: string): string {
        return errs.text(code);
    }

    Flickable {
        id: flick
        anchors.fill: parent
        // The cards' ring and focus ring stay inside; nothing else overflows.
        clip: true
        contentWidth: width
        contentHeight: Math.max(height, column.implicitHeight)
        boundsBehavior: Flickable.StopAtBounds
        interactive: contentHeight > height
        QQC2.ScrollBar.vertical: TelamonScrollBar {}

        ColumnLayout {
            id: column
            width: flick.width
            height: flick.contentHeight
            spacing: TelamonStyle.spacingLarge

            TelamonLabel {
                Layout.fillWidth: true
                textStyle: TelamonLabel.Title
                text: page.title
                wrapMode: Text.WordWrap
            }
            TelamonLabel {
                Layout.fillWidth: true
                visible: page.subtitle !== ""
                text: page.subtitle
                wrapMode: Text.WordWrap
                color: TelamonStyle.textMuted
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
                spacing: TelamonStyle.spacingLarge
            }
        }
    }
}

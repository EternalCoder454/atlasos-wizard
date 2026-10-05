import QtQuick
import QtQuick.Layouts
import Atlas.Ui

WizardPage {
    id: page

    property bool working: false
    property bool done: false

    stepId: "finish"
    title: ""

    StatusHero {
        Layout.fillWidth: true
        Layout.fillHeight: true
        iconName: "emblem-ok-symbolic"
        busy: page.working
        headline: page.done ? qsTr("Starting your desktop…") : qsTr("You're All Set")
        subtitle: page.done ? qsTr("The sign-in screen will appear in a moment.") : qsTr("Your account is ready. Finish to go to the sign-in screen.")
    }
}

import QtQuick
import QtQuick.Layouts
import Telamon.Ui

WizardPage {
    id: page

    property bool working: false
    property bool done: false
    // Once done there is nothing left to press: the display manager restarts.
    canAdvance: !done

    title: ""

    Item {
        Layout.fillWidth: true
        Layout.fillHeight: true
        Layout.minimumHeight: hero.implicitHeight
        StatusHero {
            id: hero
            anchors.verticalCenter: parent.verticalCenter
            width: parent.width
            iconName: "emblem-ok-symbolic"
            busy: page.working
            headline: page.done ? qsTr("Starting your desktop…") : qsTr("You're All Set")
            subtitle: page.done ? qsTr("The sign-in screen will appear in a moment.") : qsTr("Your account is ready. Finish to go to the sign-in screen.")
        }
    }
}

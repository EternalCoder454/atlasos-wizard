import QtQuick
import QtQuick.Layouts
import Telamon.Ui

WizardPage {
    id: page

    required property var backend
    property string fullName
    property string userName
    property bool userNameEdited: false
    property bool autologin: false
    // The last answer of the worker, for the current text.
    property int generation: 0
    property int answered: -1
    property string nameCode
    property string fullNameCode
    property string passwordCode
    property int score: -1
    readonly property bool checked: answered === generation
    readonly property bool confirmed: pass.text === confirm.text
    signal edited

    title: qsTr("Create Your Account")
    subtitle: qsTr("This account is the administrator of the computer.")
    canAdvance: checked && fullNameCode === "" && nameCode === "" && userName !== "" && passwordCode === "" && pass.text !== "" && confirmed

    function takePassword(): string {
        return pass.text;
    }
    function clearPassword(): void {
        pass.text = "";
        confirm.text = "";
    }
    function _recheck(): void {
        generation++;
        backend.checkAccount(generation, fullName, userName, pass.text);
    }
    Connections {
        target: page.backend
        function onAccountChecked(gen: int, nameCode: string, fullCode: string, pwCode: string, score: int): void {
            if (gen !== page.generation) {
                return;
            }
            page.nameCode = nameCode;
            page.fullNameCode = fullCode;
            page.passwordCode = pwCode;
            page.score = score;
            page.answered = gen;
        }
    }
    // Any change of what the checks look at asks the worker again.
    onFullNameChanged: _recheck()
    onUserNameChanged: _recheck()
    Component.onCompleted: _recheck()

    TelamonTextField {
        id: full
        Layout.fillWidth: true
        placeholderText: qsTr("Full Name")
        text: page.fullName
        errorText: page.fullNameCode !== "" ? page.errorFor(page.fullNameCode) : ""
        Accessible.name: qsTr("Full Name")
        onTextEdited: {
            page.fullName = text;
            if (!page.userNameEdited) {
                page.userName = page.backend.deriveUserName(text);
            }
            page.edited();
        }
    }
    TelamonTextField {
        id: user
        Layout.fillWidth: true
        placeholderText: qsTr("User Name")
        text: page.userName
        errorText: page.userName !== "" && page.nameCode !== "" ? page.errorFor(page.nameCode) : ""
        Accessible.name: qsTr("User Name")
        onTextEdited: {
            page.userName = text;
            page.userNameEdited = true;
            page.edited();
        }
    }
    TelamonPasswordField {
        id: pass
        Layout.fillWidth: true
        placeholderText: qsTr("Password")
        errorText: text !== "" ? (page.checked ? page.errorFor(page.passwordCode) : "") : ""
        onTextChanged: page._recheck()
        Binding on errorText {
            when: pass.text === "" || page.passwordCode === ""
            value: ""
        }
    }
    TelamonPasswordStrength {
        Layout.fillWidth: true
        score: pass.text === "" ? -1 : page.score
    }
    TelamonPasswordField {
        id: confirm
        Layout.fillWidth: true
        placeholderText: qsTr("Confirm Password")
        errorText: text !== "" && !page.confirmed ? qsTr("The passwords don't match.") : ""
        onAccepted: if (page.canAdvance)
            page.submitted()
    }
    TelamonCheckBox {
        text: qsTr("Sign in automatically")
        checked: page.autologin
        onToggled: {
            page.autologin = checked;
            page.edited();
        }
    }
    Item {
        Layout.fillHeight: true
    }
}

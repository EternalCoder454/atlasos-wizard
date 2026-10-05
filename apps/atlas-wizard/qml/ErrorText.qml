import QtQuick

// Plain-English text for every error code a page can be given: wizard-core's
// validation codes and the helper's `<code>: <text>` codes. The English text
// the helper sends is for logs only.
QtObject {
    function text(code: string): string {
        switch (code) {
        case "name-empty":
            return qsTr("Enter a user name.");
        case "name-too-long":
            return qsTr("The user name can have at most 32 characters.");
        case "name-bad-start":
            return qsTr("The user name must start with a lowercase letter or an underscore.");
        case "name-bad-char":
            return qsTr("Use only lowercase letters, digits, underscores and hyphens.");
        case "name-reserved":
            return qsTr("That user name is reserved for the system.");
        case "name-exists":
            return qsTr("That user name is already taken.");
        case "name-unreadable":
            return qsTr("The list of existing users could not be read.");
        case "user-name-exists":
            return qsTr("That user name is already taken.");
        case "full-name-too-long":
            return qsTr("The name is too long.");
        case "full-name-bad-char":
            return qsTr("The name cannot contain colons, commas or equals signs.");
        case "full-name-control":
            return qsTr("The name cannot contain line breaks or control characters.");
        case "password-too-short":
            return qsTr("Use at least 8 characters.");
        case "password-too-long":
            return qsTr("The password is too long.");
        case "password-not-text":
            return qsTr("The password must be plain text.");
        case "password-has-nul":
            return qsTr("The password cannot contain a null character.");
        case "password-user-name":
            return qsTr("The password cannot contain your user name.");
        case "password-full-name":
            return qsTr("The password cannot contain your name.");
        case "password-quality-user":
            return qsTr("The password is too similar to your name.");
        case "password-quality-dictionary":
            return qsTr("The password is too common. Try something less guessable.");
        case "password-quality-simple":
            return qsTr("The password is too simple. Mix letters, digits and symbols.");
        case "password-quality-repetitive":
            return qsTr("The password repeats characters or follows a pattern.");
        case "password-quality-short":
            return qsTr("The password is too short.");
        case "password-quality-other":
            return qsTr("The password is not strong enough.");
        case "password-check-unavailable":
            return qsTr("The password could not be checked. Try again.");
        case "accounts-password-failed":
            return qsTr("The account was created but its password could not be set. Try again.");
        case "setup-done":
            return qsTr("Setup has already finished.");
        case "polkit-denied":
            return qsTr("This session is not allowed to do that.");
        case "not-setup-user":
            return qsTr("This session is not allowed to do that.");
        case "not-authorized":
            return qsTr("This session is not allowed to do that.");
        case "caller-unknown":
            return qsTr("The setup service could not identify this session. Try again.");
        case "no-sender":
            return qsTr("The setup service could not identify this session. Try again.");
        case "shutting-down":
            return qsTr("The setup service is shutting down. Try again.");
        case "choices-bad-value":
            return qsTr("One of your choices is not valid. Go back and check them.");
        case "settings-failed":
            return qsTr("Your settings could not be saved. Try again.");
        case "restart-failed":
            return qsTr("The sign-in screen could not be started. Try again.");
        case "verify-no-passwd-entry":
            return qsTr("The account could not be verified. Try again.");
        case "io":
            return qsTr("A file could not be written. Try again.");
        case "worker":
            return qsTr("Something went wrong inside the setup. Try again.");
        case "unreachable":
            return qsTr("The setup service did not answer. Try again.");
        case "timeout":
            return qsTr("This is taking too long. Try again, or skip this step.");
        case "failed":
            return qsTr("Something went wrong. Try again.");
        case "wifi-failed":
            return qsTr("Could not connect. Check the password and try again.");
        case "wifi-no-device":
            return qsTr("No Wi-Fi adapter was found.");
        case "orca-missing":
            return qsTr("The screen reader is not installed.");
        case "bad-argument":
            return qsTr("That choice is not valid.");
        case "list-unreadable":
            return qsTr("The list could not be loaded. Try again.");
        default:
            return qsTr("Something went wrong. Try again.");
        }
    }
}

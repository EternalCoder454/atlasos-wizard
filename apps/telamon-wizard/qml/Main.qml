pragma ComponentBehavior: Bound
import QtQuick
import Telamon.Ui

TelamonWindow {
    id: root

    // The Rust backend; telamon_app_run sets it.
    required property var backend

    property var answers: ({})
    property var startup: ({})
    property bool started: false
    property bool accountDone: false
    // Whose answer the Next button is waiting for: "language", "account", ...
    property string waitingFor: ""

    title: TelamonApp.name
    // Full screen, no close: the stand-in for TelamonWindow.kiosk (Telamon.Ui 1.5.0).
    x: 0
    y: 0
    width: Screen.width
    height: Screen.height
    visibility: Window.FullScreen
    visible: true
    LayoutMirroring.enabled: Qt.locale().textDirection === Qt.RightToLeft
    LayoutMirroring.childrenInherit: true

    // The window is the whole session: it closes only once setup is done.
    onClosing: close => {
        if (!root.backend.finished) {
            close.accepted = false;
        }
    }

    ErrorText {
        id: errors
    }

    function setAnswer(key: string, value: var): void {
        const a = Object.assign({}, root.answers);
        a[key] = value;
        root.answers = a;
        root.backend.saveAnswers(JSON.stringify(a));
    }
    function setAnswers(values: var): void {
        root.answers = Object.assign({}, root.answers, values);
        root.backend.saveAnswers(JSON.stringify(root.answers));
    }
    function indexOfStep(id: string): int {
        for (let i = 0; i < onboarding.pages.length; ++i) {
            if (onboarding.pages[i].stepId === id) {
                return i;
            }
        }
        return 0;
    }
    function begin(): void {
        const s = JSON.parse(root.backend.startupJson || "{}");
        const a = JSON.parse(root.backend.answersJson || "{}");
        root.startup = s;
        // Answers the user gave win; else what the installer or the system says.
        const d = {};
        d.language = a.language || s.language || "";
        d.keyboardLayout = a.keyboardLayout || s.keyboardLayout || "";
        d.keyboardVariant = a.keyboardLayout ? (a.keyboardVariant || "") : (s.keyboardVariant || "");
        d.timezone = a.timezone || s.timezone || "";
        d.hostname = a.hostname || "";
        root.answers = Object.assign({}, a, d);
        if (s.resumeAccount) {
            root.accountDone = true;
            root.answers = Object.assign({}, root.answers, {
                userName: s.resumeAccount
            });
        }
        root.backend.setTextScale(root.answers.textScale ?? 1);
        if (root.answers.screenReader) {
            root.backend.setOption("screenReader", true);
        }
        if (root.answers.highContrast && s.highContrastAvailable) {
            root.backend.setOption("highContrast", true);
        }
        let target = onboarding.pages.length > 0 ? indexOfStep(a.step || "welcome") : 0;
        if (s.resumeAccount && target <= indexOfStep("account")) {
            target = indexOfStep("account") + 1;
        }
        // Never rest on a page that is left out.
        while (target < onboarding.pages.length - 1 && onboarding.pages[target].hidden) {
            target++;
        }
        onboarding.currentIndex = target;
        root.started = true;
        root.enter(onboarding.pages[target]);
    }
    // The page's step makes the page and has it load its lists the first time
    // it shows.
    function enter(step: var): void {
        if (step) {
            step.enter();
        }
    }
    function stepFor(id: string): var {
        return onboarding.pages[indexOfStep(id)];
    }

    Component.onCompleted: {
        if (root.backend.ready) {
            begin();
        } else {
            root.backend.init();
        }
    }
    Connections {
        target: root.backend
        function onReadyChanged(): void {
            if (root.backend.ready && !root.started) {
                root.begin();
            }
        }
        function onCompleted(what: string, ok: bool, code: string, text: string): void {
            if (what === "screenReader" || what === "highContrast") {
                if (!ok) {
                    welcomeStep.setError(errors.text(code));
                }
                return;
            }
            if (what === "wifi") {
                wifiStep.item?.connectDone(ok, code);
                return;
            }
            const step = root.stepFor(what);
            onboarding.busy = false;
            root.waitingFor = "";
            if (!ok) {
                step.setError(errors.text(code));
                if (what === "finish") {
                    finishStep.item.working = false;
                }
                return;
            }
            step.setError("");
            if (what === "account") {
                const account = accountStep.item;
                account.clearPassword();
                root.accountDone = true;
                root.setAnswers({
                    userName: account.userName,
                    fullName: account.fullName
                });
                onboarding.next();
            } else if (what === "finish") {
                finishStep.item.working = false;
                finishStep.item.done = true;
            } else {
                onboarding.next();
            }
        }
    }

    Shortcut {
        sequence: "Meta+Alt+S"
        onActivated: root.toggleScreenReader(!(root.answers.screenReader ?? false))
    }
    function toggleScreenReader(on: bool): void {
        root.setAnswer("screenReader", on);
        root.backend.setOption("screenReader", on);
    }

    FirstLoginPage {
        anchors.fill: parent
        visible: root.backend.welcomeMode
    }

    TelamonLabel {
        anchors.centerIn: parent
        visible: !root.backend.welcomeMode && !root.started
        text: qsTr("Starting…")
    }

    WizardOnboarding {
        id: onboarding
        anchors.fill: parent
        visible: !root.backend.welcomeMode && root.started
        autoAdvance: false
        canGoBack: !(finishStep.item?.done ?? false)
        finishText: qsTr("Finish")
        onCurrentIndexChanged: {
            if (root.started) {
                root.setAnswer("step", onboarding.pages[currentIndex].stepId);
                root.enter(onboarding.pages[currentIndex]);
            }
        }
        onSkipped: index => {
            const p = onboarding.pages[index];
            p.setError("");
            if (p.stepId === "wifi") {
                root.setAnswer("wifiDone", true);
            }
        }
        onAdvanceRequested: index => {
            const p = onboarding.pages[index];
            const a = root.answers;
            p.setError("");
            const call = (what, x, y) => {
                root.waitingFor = what;
                onboarding.busy = true;
                root.backend.apply(what, x, y);
            };
            switch (p.stepId) {
            case "language":
                a.language ? call("language", a.language, "") : onboarding.next();
                break;
            case "keyboard":
                a.keyboardLayout ? call("keyboard", a.keyboardLayout, a.keyboardVariant ?? "") : onboarding.next();
                break;
            case "timezone":
                a.timezone ? call("timezone", a.timezone, "") : onboarding.next();
                break;
            case "hostname":
                // The page's own value: answers only hold the name once it is
                // edited, and sending "" for an untouched prefill failed.
                call("hostname", p.item.hostname, "");
                break;
            case "account":
                root.waitingFor = "account";
                onboarding.busy = true;
                root.backend.createAccount(p.item.userName, p.item.fullName, p.item.takePassword(), p.item.autologin);
                break;
            case "finish":
                root.waitingFor = "finish";
                onboarding.busy = true;
                p.item.working = true;
                root.backend.finishSetup();
                break;
            default:
                onboarding.next();
            }
        }

        WizardStep {
            id: welcomeStep
            stepId: "welcome"
            WelcomePage {
                screenReader: root.answers.screenReader ?? false
                largerText: (root.answers.textScale ?? 1) > 1
                highContrast: root.answers.highContrast ?? false
                highContrastAvailable: root.startup.highContrastAvailable ?? false
                onOptionToggled: (what, on) => {
                    if (what === "screenReader") {
                        root.toggleScreenReader(on);
                    } else if (what === "largerText") {
                        root.setAnswer("textScale", on ? 1.25 : 1.0);
                        root.backend.setTextScale(on ? 1.25 : 1.0);
                    } else {
                        root.setAnswer("highContrast", on);
                        root.backend.setOption("highContrast", on);
                    }
                }
            }
        }
        WizardStep {
            stepId: "language"
            hidden: root.startup.skipLanguage ?? false
            LanguagePage {
                backend: root.backend
                language: root.answers.language ?? ""
                canAdvance: root.waitingFor === ""
                onChosen: locale => root.setAnswer("language", locale)
            }
        }
        WizardStep {
            stepId: "keyboard"
            hidden: root.startup.skipKeyboard ?? false
            KeyboardPage {
                backend: root.backend
                layout: root.answers.keyboardLayout ?? ""
                variant: root.answers.keyboardVariant ?? ""
                onChosen: (layout, variant) => root.setAnswers({
                        keyboardLayout: layout,
                        keyboardVariant: variant
                    })
            }
        }
        WizardStep {
            id: wifiStep
            stepId: "wifi"
            hidden: root.startup.skipWifi ?? false
            WifiPage {
                backend: root.backend
            }
        }
        WizardStep {
            stepId: "timezone"
            TimeZonePage {
                backend: root.backend
                zone: root.answers.timezone ?? ""
                onChosen: id => root.setAnswer("timezone", id)
            }
        }
        WizardStep {
            id: accountStep
            stepId: "account"
            hidden: root.accountDone
            AccountPage {
                backend: root.backend
                fullName: root.answers.fullName ?? ""
                userName: root.answers.userName ?? ""
                userNameEdited: root.answers.userNameEdited ?? false
                autologin: root.answers.autologin ?? false
                onEdited: root.setAnswers({
                    fullName: fullName,
                    userName: userName,
                    userNameEdited: userNameEdited,
                    autologin: autologin
                })
            }
        }
        WizardStep {
            stepId: "hostname"
            hidden: !(root.startup.askHostname ?? false)
            HostnamePage {
                // A neutral name, the same as the image's DEFAULT_HOSTNAME. It was
                // "<user>-pc", which put the account name on the network.
                hostname: root.answers.hostname || "telamon"
                onEdited: root.setAnswer("hostname", hostname)
            }
        }
        WizardStep {
            stepId: "appearance"
            AppearancePage {
                look: root.answers.look ?? "light"
                accent: root.answers.accent ?? "#6858E2" // telamon-lint: allow-raw
                onEdited: root.setAnswers({
                    look: look,
                    accent: accent
                })
            }
        }
        WizardStep {
            stepId: "privacy"
            PrivacyPage {
                crashReports: root.answers.crashReports ?? false
                onToggled: on => root.setAnswer("crashReports", on)
            }
        }
        WizardStep {
            id: finishStep
            stepId: "finish"
            FinishPage {}
        }
    }
}

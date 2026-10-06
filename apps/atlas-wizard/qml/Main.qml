import QtQuick
import org.kde.kirigami as Kirigami
import Atlas.Ui

AtlasWindow {
    id: root

    // The Rust backend; atlas_app_run sets it.
    required property var backend

    property var answers: ({})
    property var startup: ({})
    property bool started: false
    property bool accountDone: false
    // Whose answer the Next button is waiting for: "language", "account", ...
    property string waitingFor: ""

    title: AtlasApp.name
    // Full screen, no close: the stand-in for AtlasWindow.kiosk (Atlas.Ui 1.5.0).
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
            root.answers = Object.assign({}, root.answers, { userName: s.resumeAccount });
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
    // Lists load the first time their page shows.
    function enter(page: var): void {
        if (page && page.load) {
            page.load();
        }
    }
    function pageFor(what: string): var {
        const id = what === "finish" ? "finish" : what;
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
                    welcome.errorText = errors.text(code);
                }
                return;
            }
            if (what === "wifi") {
                wifi.connectDone(ok, code);
                return;
            }
            const page = root.pageFor(what);
            onboarding.busy = false;
            root.waitingFor = "";
            if (!ok) {
                page.errorText = errors.text(code);
                if (what === "finish") {
                    finish.working = false;
                }
                return;
            }
            page.errorText = "";
            if (what === "account") {
                account.clearPassword();
                root.accountDone = true;
                root.setAnswers({ userName: account.userName, fullName: account.fullName });
                onboarding.next();
            } else if (what === "finish") {
                finish.working = false;
                finish.done = true;
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

    AtlasLabel {
        anchors.centerIn: parent
        visible: !root.backend.welcomeMode && !root.started
        text: qsTr("Starting…")
    }

    WizardOnboarding {
        id: onboarding
        anchors.fill: parent
        visible: !root.backend.welcomeMode && root.started
        autoAdvance: false
        canGoBack: !finish.done
        finishText: qsTr("Finish")
        onCurrentIndexChanged: {
            if (root.started) {
                root.setAnswer("step", onboarding.pages[currentIndex].stepId);
                root.enter(onboarding.pages[currentIndex]);
            }
        }
        onSkipped: index => {
            const p = onboarding.pages[index];
            p.errorText = "";
            if (p.stepId === "wifi") {
                root.setAnswer("wifiDone", true);
            }
        }
        onAdvanceRequested: index => {
            const p = onboarding.pages[index];
            const a = root.answers;
            p.errorText = "";
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
                call("hostname", p.hostname, "");
                break;
            case "account":
                root.waitingFor = "account";
                onboarding.busy = true;
                root.backend.createAccount(account.userName, account.fullName, account.takePassword(), account.autologin);
                break;
            case "finish":
                root.waitingFor = "finish";
                onboarding.busy = true;
                finish.working = true;
                root.backend.finishSetup();
                break;
            default:
                onboarding.next();
            }
        }

        WelcomePage {
            id: welcome
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
        LanguagePage {
            backend: root.backend
            hidden: root.startup.skipLanguage ?? false
            language: root.answers.language ?? ""
            canAdvance: !(root.waitingFor !== "")
            onChosen: locale => root.setAnswer("language", locale)
        }
        KeyboardPage {
            backend: root.backend
            hidden: root.startup.skipKeyboard ?? false
            layout: root.answers.keyboardLayout ?? ""
            variant: root.answers.keyboardVariant ?? ""
            onChosen: (layout, variant) => root.setAnswers({ keyboardLayout: layout, keyboardVariant: variant })
        }
        WifiPage {
            id: wifi
            backend: root.backend
            hidden: root.startup.skipWifi ?? false
        }
        TimeZonePage {
            backend: root.backend
            zone: root.answers.timezone ?? ""
            onChosen: id => root.setAnswer("timezone", id)
        }
        AccountPage {
            id: account
            backend: root.backend
            hidden: root.accountDone
            fullName: root.answers.fullName ?? ""
            userName: root.answers.userName ?? ""
            userNameEdited: root.answers.userNameEdited ?? false
            autologin: root.answers.autologin ?? false
            onEdited: root.setAnswers({ fullName: fullName, userName: userName, userNameEdited: userNameEdited, autologin: autologin })
        }
        HostnamePage {
            hidden: !(root.startup.askHostname ?? false)
            // A neutral name, the same as the image's DEFAULT_HOSTNAME. It was
            // "<user>-pc", which put the account name on the network.
            hostname: root.answers.hostname || "atlasos"
            onEdited: root.setAnswer("hostname", hostname)
        }
        AppearancePage {
            look: root.answers.look ?? "light"
            accent: root.answers.accent ?? "#6858E2" // atlas-lint: allow-raw
            onEdited: root.setAnswers({ look: look, accent: accent })
        }
        PrivacyPage {
            crashReports: root.answers.crashReports ?? false
            onToggled: on => root.setAnswer("crashReports", on)
        }
        FinishPage {
            id: finish
        }
    }
}

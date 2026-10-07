import QtQuick

// A step of WizardOnboarding. The page inside is made when the step is first
// shown, or a moment after the page before it, so a page nobody has reached
// costs nothing at start (the setup session must appear fast and fit a 4 GB
// machine). Once made it stays, with what the user typed. `hidden` is the
// step's, since it is known before the page exists; the rest is the page's,
// read through the step.
//
//   WizardStep {
//       stepId: "language"
//       hidden: startup.skipLanguage
//       LanguagePage { ... }
//   }
Item {
    id: step

    default property Component page
    property string stepId
    property bool hidden: false

    // The page itself, once made (null before).
    readonly property var item: loader.item
    readonly property string title: step.item ? step.item.title : ""
    readonly property bool canAdvance: step.item ? step.item.canAdvance : true
    readonly property bool skippable: step.item ? step.item.skippable : false

    property bool _wanted: false

    // Makes the page now; a page that is there already stays.
    function ensure(): void {
        step._wanted = true;
    }
    // Shows `text` as the page's error; a page not made yet has none to show.
    function setError(text: string): void {
        if (step.item) {
            step.item.errorText = text;
        }
    }
    // Loads the page's lists, if it has any (when the user arrives).
    function enter(): void {
        step.ensure();
        if (step.item && step.item.load) {
            step.item.load();
        }
    }

    onVisibleChanged: if (step.visible)
        step.ensure()

    Loader {
        id: loader
        anchors.fill: parent
        active: step._wanted
        sourceComponent: step.page
    }
}

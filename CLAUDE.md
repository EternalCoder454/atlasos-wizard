# AtlasOS Wizard

AtlasOS's first-run setup: Rust + CXX-Qt + Qt 6.11 Quick + Kirigami on Atlas
Framework (`v1.4.0`), for AtlasOS, a Fedora Kinoite 44 bootc image (repo
`~/Documents/Projects/AtlasOS/AtlasOS`, read-only from here). Read
`docs/DESIGN.md` first: it fixes the programs, the helper's methods, the
polkit rules, the state file and every recovery path. Change it only together
with the code that changes what it says. The plan and checklist are the Atlas
Notes notes "AtlasOS/Wizard/Plan" and "AtlasOS/Wizard/Roadmap".

The stack, build and look follow Atlas Monitor
(`~/Documents/Projects/AtlasOS/AtlasOS Monitor`); the root helper follows Atlas
Updater's (`~/Documents/Projects/AtlasOS/Atlas Updater`,
`crates/atlas-update-engine`). The framework is `~/Documents/Atlas Framework`
(read-only from here; its reference is `docs/reference/`).

## Hard rules

- **Build and test inside the dev container**, never on the host:
  `scripts/dev.sh <command>`. Use a separate target dir per agent or task
  (`CARGO_TARGET_DIR=/src/target/<name> scripts/dev.sh ...`).
- **Never run anything privileged on the host**: the helper, `prepare`, the
  fallback, `useradd`, AccountsService calls. They run in the container
  against a private system bus and a throwaway root, and for real in the
  AtlasOS VM (the "AtlasOS" session runs it).
- **Never run the GUI on the user's display.** `QT_QPA_PLATFORM=offscreen`, or
  `xvfb-run -a -s "-screen 0 1920x1080x24"` inside `dbus-run-session`, in the
  container.
- **The helper's surface is fixed** (DESIGN.md, The helper): four methods,
  none takes a path, command, argv or unit name. Adding one, or a polkit
  grant, is a design change: DESIGN.md first, and a security review.
- **Nothing secret is ever logged or stored**: passwords travel as bytes, are
  zeroed after use, and never reach the journal, the state file, answers.json
  or a panic message. Tests check.
- **Every write that matters is atomic** (temp file, fsync, rename, directory
  fsync), and every step that can be cut by a power loss has a row in
  DESIGN.md's recovery table and a test.
- **The GUI thread never blocks**: D-Bus and file work on a worker, results
  posted with `qt_thread().queue`.
- **Atlas.Ui is the installed `atlas-ui` package.** Use its controls (never
  QQC2 or Kirigami buttons); what it lacks is asked of the framework. A local
  stand-in is named `Wizard<Name>` so it never clashes (`check-app-names.sh`).
- Commit only the paths you own (`git commit -- <paths>`), as
  `EternalHell <77252745+EternalCoder454@users.noreply.github.com>`. Don't
  push; the repository is not on GitHub yet.
- MIT. Wording follows KDE: Title Case buttons and titles, US spelling,
  `qsTr()` for every string a user reads.

## Commands

| Task | Command (from the repo root on the host) |
|---|---|
| Format | `scripts/dev.sh cargo fmt --all --check` |
| Lint | `scripts/dev.sh cargo clippy --workspace --all-targets --locked -- -D warnings` |
| Tests | `scripts/dev.sh cargo test --workspace --locked` |
| App build | `scripts/dev.sh bash -c 'cmake -S apps/atlas-wizard -B build/dev -G Ninja && cmake --build build/dev'` |
| Smoke run | `scripts/dev.sh dbus-run-session -- env QT_QPA_PLATFORM=offscreen ATLAS_WIZARD_DEMO=1 build/dev/atlas-wizard` |
| Atlas checks | `~/Documents/Atlas\ Framework/tools/lint-app.sh . && ~/Documents/Atlas\ Framework/tools/check-app-names.sh .` (host, read-only scripts) |
| RPM | `scripts/dev.sh packaging/build-rpm.sh /src/out` |

`scripts/dev.sh` builds `localhost/atlas-wizard-dev:44` on first use from
`ATLAS_LOCAL_RPMS=<dir>`: atlas-framework v1.4.0's RPMs (atlas-ui and
atlas-symbols-fonts), built from the tag with the framework's
`packaging/build-rpm.sh`. Delete the image after changing the spec's
BuildRequires or to take a newer atlas-ui.

`ATLAS_WIZARD_DEMO=1` runs the GUI with no helper and no system services
(every call answers from canned data), for screenshots and layout work.

## Moving the atlas-framework pin

Change `tag` in `Cargo.toml`, then `scripts/dev.sh cargo update -p atlas-framework-ui -p atlas-framework-system`.
Move the same tag in `.github/workflows/ci.yml` and, when the app uses
something new in Atlas.Ui, `ui:` in `apps/atlas-wizard/src/lib.rs` and
`atlas-ui >=` in the spec (Requires and BuildRequires) together.

# Telamon Wizard

Telamon OS's first-run setup: Rust + CXX-Qt + Qt 6.11 Quick + Kirigami on the
Telamon framework (`v2.0.0`), for Telamon OS, a Fedora Kinoite 44 bootc image (repo
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
  Telamon OS VM (the "AtlasOS" session runs it).
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
- **Telamon.Ui is the installed `telamon-ui` package.** Use its controls (never
  QQC2 or Kirigami buttons); what it lacks is asked of the framework. A local
  stand-in is named `Wizard<Name>` so it never clashes (`check-app-names.sh`).
- Commit only the paths you own (`git commit -- <paths>`), as
  `EternalHell <77252745+EternalCoder454@users.noreply.github.com>`. Don't
  push; the repository is not on GitHub yet.
- **The old names (Atlas Wizard 0.1.x) still work for one release**: the old
  units, programs, done marker, state and installer.ini paths, the kernel
  command line and the setup autologin's removal (DESIGN.md, "The names until
  0.2.0"). Do not remove them before the image and the installer have moved.
- MIT. Wording follows KDE: Title Case buttons and titles, US spelling,
  `qsTr()` for every string a user reads.

## Commands

| Task | Command (from the repo root on the host) |
|---|---|
| Format | `scripts/dev.sh cargo fmt --all --check` |
| Lint | `scripts/dev.sh cargo clippy --workspace --all-targets --locked -- -D warnings` |
| Tests | `scripts/dev.sh cargo test --workspace --locked` |
| App build | `scripts/dev.sh bash -c 'cmake -S apps/telamon-wizard -B build/dev -G Ninja && cmake --build build/dev'` |
| Smoke run | `scripts/dev.sh dbus-run-session -- env QT_QPA_PLATFORM=offscreen TELAMON_WIZARD_DEMO=1 build/dev/telamon-wizard` |
| Telamon checks | `<framework v2.0.0>/tools/lint-app.sh . && <framework v2.0.0>/tools/check-app-names.sh .` (read-only scripts; the dev image has the framework at `$TELAMON_FRAMEWORK`) |
| RPM | `scripts/dev.sh packaging/build-rpm.sh /src/out` |

`scripts/dev.sh` builds `localhost/telamon-wizard-dev:44` from `ci/Containerfile`
(the one list of packages; CI runs in the same file's `ci` target, published
as `ghcr.io/eternalcoder454/telamon-wizard-dev` by `dev-image.yml`). It builds
telamon-ui and telamon-symbols-fonts from the atlas-framework tag in `Cargo.toml`
(needs network), and rebuilds by itself when the Containerfile, the spec's
BuildRequires or that tag change (`ci/image-tag.sh`).
`TELAMON_FRAMEWORK_REF=<tag or branch>` builds against another framework ref.
`TELAMON_LOCAL_RPMS` is gone. CI (`.github/workflows/ci.yml`) runs the Format,
Lint, Tests and RPM rows above plus qmllint, shellcheck and
`desktop-file-validate`, with mold as the linker (`RUSTFLAGS` there only).

`TELAMON_WIZARD_DEMO=1` runs the GUI with no helper and no system services
(every call answers from canned data), for screenshots and layout work.

## Moving the atlas-framework pin

Change `tag` in `Cargo.toml`, then `scripts/dev.sh cargo update -p telamon-framework-ui -p telamon-framework-system`.
CI and the dev image follow the tag by themselves (`ci/framework-ref.sh`).
When the app uses
something new in Telamon.Ui, `ui:` in `apps/telamon-wizard/src/lib.rs` and
`telamon-ui >=` in the spec (Requires and BuildRequires) together.

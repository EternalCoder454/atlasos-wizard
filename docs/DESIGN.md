# AtlasOS Wizard: design

AtlasOS's first-run setup. It runs on first boot, before any user account
exists, as its own locked system user, creates the account, and hands over to
the login screen. It replaces Fedora's plasma-setup (which AtlasOS rebuilt
with a style patch) and the `atlasos-fingerprint-setup` autostart.

App ID `net.eterneon.atlas.wizard`, binary `atlas-wizard`, repository
`EternalCoder454/atlasos-wizard`, version 0.1.0, MIT. Built on Atlas
Framework `v1.4.0` (`ui: "1.4.0"`). The plan and checklist are the Atlas Notes
notes "AtlasOS/Wizard/Plan" and "AtlasOS/Wizard/Roadmap". This file is the
contract: change it together with the code that changes what it says.

## Layout

```
Cargo.toml                      workspace
crates/wizard-core/             no Qt, no D-Bus: validation, installer.ini, state
                                file, boot decisions, per-user settings, password
                                hashing (libxcrypt) and quality (libpwquality)
crates/wizard-helper/           atlas-wizard-helper: the root D-Bus helper
crates/wizard-boot/             atlas-wizard-boot: prepare (before the display
                                manager) and the text-mode fallback
apps/atlas-wizard/              the GUI: CXX-Qt backend (src/), QML (qml/), C++ (cpp/)
data/                           everything installed that is not a binary:
  systemd/ dbus-1/ polkit-1/ sysusers.d/ tmpfiles.d/ wayland-sessions/
  autostart/ libexec/ dnf/
packaging/atlas-wizard.spec     the RPM; build-rpm.sh builds it in fedora:44
scripts/dev.sh                  runs a command in the dev container
tests/                          integration tests (private system bus, mocks)
docs/                           this file, the image hand-over
```

## Programs

| Program | Runs as | What |
|---|---|---|
| `/usr/libexec/atlas-wizard-boot prepare` | root, `atlas-wizard-boot.service`, every boot before the display manager | Reads the state and the system, decides (table below), writes or removes the setup autologin, cleans up and locks `atlas-setup` once setup is done. Fast: a few stats when done. |
| `/usr/libexec/atlas-wizard-boot fallback` | root, `atlas-wizard-fallback.service` on tty1 | Text-mode account creation, when the GUI cannot run. |
| `/usr/libexec/atlas-wizard-session` | `atlas-setup`, the plasmalogin autologin session `atlas-wizard` | Starts `kwin_wayland` with only the wizard, the screen reader's bus, the on-screen keyboard. Counts failed starts this boot in `/run/atlas-setup/session-failures`; at 3 it calls the helper's `GiveUp`. |
| `/usr/bin/atlas-wizard` | `atlas-setup` | The setup pages. |
| `/usr/bin/atlas-wizard --welcome` | the signed-in user, XDG autostart | First-login extras: fingerprint, PIN. |
| `/usr/libexec/atlas-wizard-helper` | root, D-Bus activated (`atlas-wizard-helper.service`, `Type=dbus`), exits after 30 s idle | The only privileged code the GUI reaches. |

## First boot

1. `atlas-wizard-boot.service`: `Type=oneshot`,
   `Before=display-manager.service plasmalogin.service`,
   `After=systemd-sysusers.service systemd-tmpfiles-setup.service systemd-user-sessions.service`,
   `ConditionKernelCommandLine=!rd.live.image`, `WantedBy=multi-user.target`.
   It runs at every boot (not only the first), so cleanup is re-checked.
2. When setup is needed it writes `/etc/plasmalogin.conf.d/99-atlas-wizard.conf`:

   ```ini
   [Autologin]
   User=atlas-setup
   Session=atlas-wizard
   Relogin=true
   ```

3. plasmalogin signs `atlas-setup` into `atlas-wizard.desktop`
   (`/usr/share/wayland-sessions/`, `NoDisplay=true`, `Hidden` from the
   session list), which runs `atlas-wizard-session`.
4. The wizard's pages. System-wide choices apply at once through localed,
   timedated, hostnamed and NetworkManager (polkit rule below). The answers
   so far (never the password) are kept in `/run/atlas-setup/answers.json`,
   so a crash of the session comes back on the same page.
5. Account page Next: helper `CreateAccount`. The account exists from then on.
6. Finish: helper `Finish`; the finish screen's button: helper `EndSetup`,
   which restarts `display-manager.service`. The login screen shows with the
   new account selected; when the user ticked "Sign in automatically", the
   autologin drop-in `/etc/plasmalogin.conf.d/50-atlas-autologin.conf` signs
   them in instead. (Typing the password once at the login screen unlocks the
   wallet; that is why there is no one-time autologin.)
7. First login: `atlas-wizard --welcome` (autostart) offers fingerprint
   enrolment and a PIN, each only when it applies.

### The setup session

Minimal on purpose (decided 2026-10-05): it must appear fast on a cold boot
and fit a 4 GB machine. `kwin_wayland` runs the wizard alone; no plasmashell,
kded, wallet, notifications, lock screen or portals. at-spi starts on demand
for Orca; plasma-keyboard is KWin's input method. The wizard shows network
and battery itself. If a GPU misbehaves with KWin alone in the VM tests, the
fallback is the same session file running a full Plasma session.

The setup user `atlas-setup` (sysusers, `u atlas-setup - "AtlasOS Setup" /run/atlas-setup /bin/sh`,
password `!*`) has its home on `/run` (tmpfiles, mode 0750, owned by it), with
the wizard's own `kdeglobals` and `kxkbrc` there. Its settings never reach
the new account except through `Finish`.

## The helper

Bus name `net.eterneon.atlas.WizardHelper`, object `/net/eterneon/atlas/WizardHelper`,
interface `net.eterneon.atlas.WizardHelper1`. Every method:

- authorizes the caller with `atlas_framework_system::polkit::check` against
  its own action (non-interactive);
- checks the caller's uid is `atlas-setup`'s (from the bus, not an argument);
- refuses with `net.eterneon.atlas.Error.SetupDone` once `/etc/atlasos/setup-done`
  exists;
- validates every argument again with wizard-core (never trusts the GUI);
- takes no path, command, argv or unit name.

| Method | Polkit action | Does |
|---|---|---|
| `CreateAccount(s name, s full_name, ay password, b autologin) -> u uid` | `net.eterneon.atlas.wizard.create-account` | AccountsService `CreateUser(name, full_name, 1)` (administrator: wheel), then `SetPassword(yescrypt hash, "")` with the hash made in the helper. State stages `creating`, `created`, `password-set`, `verified` are written before and after each step. Verifies the passwd entry, the shadow hash, wheel, and the home owned by the uid. One account per first run: a second call fails unless the state names a half-made account, which is deleted first (only that uid, only when its home holds nothing but skel). The password buffer is zeroed. |
| `Finish(a{sv} choices)` | `net.eterneon.atlas.wizard.finish` | Writes the new account's settings as that user (below), the autologin drop-in if asked, then the done markers, removes the setup autologin, locks `atlas-setup`. Idempotent: a repeat after a crash finishes the remaining steps. |
| `EndSetup()` | `net.eterneon.atlas.wizard.finish` | Restarts `display-manager.service` (systemd D-Bus, fixed unit). |
| `GiveUp()` | `net.eterneon.atlas.wizard.fallback` | Records it in the state, removes the setup autologin, starts `atlas-wizard-fallback.service`. |

`choices` keys (anything else is refused): `look` (`light` / `dark`),
`accent` (`#rrggbb` from the offered list), `text_scale` (`1.0`, `1.25`,
`1.5`), `high_contrast` (b), `screen_reader` (b), `crash_reports` (b),
`keyboard` (`layout`, `variant`, as validated XKB names). Global Themes are
`org.atlasos.desktop` (light) and `org.atlasos.dark.desktop` (dark).

**Settings for the new account** are written by a child that has dropped to
the account's uid and gid (setresgid, setgroups, setresuid) before it opens
anything, with `HOME` and `XDG_*` pointing into the home and nothing else in
the environment: root never writes into a user-owned tree. Files: the look
and accent through `plasma-apply-lookandfeel --apply <id>` and
`plasma-apply-colorscheme --accent-color <hex>` (fixed argv,
`QT_QPA_PLATFORM=offscreen`), `kdeglobals` font size for larger text,
`kaccessrc` `[ScreenReader] Enabled`, `kxkbrc`, and
`~/.config/atlas/crash-reporting.toml` through the framework's
`crash::Settings::save_to`. Each write is temp file + rename.

## Polkit

`/usr/share/polkit-1/actions/net.eterneon.atlas.wizard.policy`: the three
actions above, every default `no`. `/usr/share/polkit-1/rules.d/50-atlas-wizard.rules`
returns YES only when `subject.user == "atlas-setup" && subject.local &&
subject.active`, for our three actions and these stock ones:

- `org.freedesktop.locale1.set-locale`, `org.freedesktop.locale1.set-keyboard`
- `org.freedesktop.timedate1.set-timezone`, `org.freedesktop.timedate1.set-ntp`
- `org.freedesktop.hostname1.set-static-hostname`, `org.freedesktop.hostname1.set-hostname`
- `org.freedesktop.NetworkManager.settings.modify.system`,
  `org.freedesktop.NetworkManager.network-control`,
  `org.freedesktop.NetworkManager.enable-disable-wifi`

plasma-setup's other grants (its KAuth actions, display scaling, temporary
autologin) are not carried over. After setup `atlas-setup` can have no
session (below), so the rule can never match again.

## Done markers

- `/etc/atlasos/setup-done`, ours: `[Setup]` `Version=1`, `Finished=<RFC 3339>`,
  `Wizard=<version>`. Written temp + fsync + rename + directory fsync.
- `/etc/plasma-setup-done`, also written, so the image's `health-lib` and
  `pin-setup` keep working and a rollback to an image that still has
  plasma-setup never runs it again.
- An existing `/etc/plasma-setup-done` (a machine set up before this wizard)
  counts as done.

## State and recovery

`/var/lib/atlas-wizard/state.json` (root, 0644, no secrets), written atomically:

```json
{"format": 1, "boots": 2, "gave_up": false,
 "account": {"name": "ada", "uid": 1000, "stage": "verified"},
 "finish": "markers"}
```

Readers accept a missing or higher `format` (unknown keys kept as-is when
rewriting is not needed); a file that cannot be parsed is renamed to
`state.json.bad-<time>` and treated as empty, with a journal warning.

`prepare` decides in this order (a pure function, `wizard_core::boot::decide`,
with a test per row):

| Found | Does |
|---|---|
| a done marker | cleanup: remove the setup autologin drop-in, lock `atlas-setup`, write the missing marker; normal login screen |
| `atlas.wizard=skip` and a usable human account | mark done, cleanup |
| a usable human account (uid 1000–60000, a password hash, a login shell) not in the state | mark done, cleanup (Anaconda, kickstart) |
| account `verified`, Finish not done, `boots` < 3 | setup autologin; the wizard resumes after the Account page |
| account `verified`, `boots` ≥ 3 | finish with defaults (markers, cleanup); login screen |
| `gave_up`, `boots` ≥ 3 with no account, or `atlas.wizard=fallback` | start `atlas-wizard-fallback.service` (text mode); no setup autologin |
| otherwise | setup autologin, `boots` + 1 |

Details of the table (`wizard_core::boot`):

- "Finish not done" means `finish` is unset; a verified account whose Finish
  had begun (and no marker) is finished with defaults at once.
- `atlas.wizard=fallback` only applies when there is no verified account: with
  one, the account exists and the machine is finished with defaults instead.
- "Not in the state" means its name and uid differ from the state's account.
- A state account that is not usable (no shadow hash, missing) counts as
  half-made, whatever its stage says. An unknown stage (from a newer wizard)
  is kept as it is and counts as verified when `accounts::verify` passes,
  half-made otherwise.
- installer.ini values of an impossible shape count as unset (the page shows).

A half-made account (`creating`, `created`, `password-set`) is resolved by the
helper at the next `CreateAccount` (deleted, made again), and by the fallback
the same way.

**Locking `atlas-setup`:** account expired (`chage -E 0`, which pam_unix's
account check refuses even for autologin), shell `/usr/sbin/nologin`, the
setup autologin drop-in removed, `/run/atlas-setup` emptied, and its logind
user terminated. `prepare` checks all of it at every boot and fixes what is
missing.

**The fallback** (`atlas-wizard-fallback.service`, `Conflicts=display-manager.service`,
`TTYPath=/dev/tty1`, `StandardInput=tty`): asks for the full name, user name
and password twice on the console, with the same wizard-core validation, and
creates the account with `useradd -m -U -G wheel -c <name> <user>` and
`chpasswd -e` (the hash on stdin), fixed argv. It is the one path that does
not use AccountsService, so a broken AccountsService still ends in an
account. Then it writes the markers, cleans up and starts the display manager.
No Qt, GPU or network needed.

**Kernel command line, for support:** `atlas.wizard=fallback` forces the text
mode; `atlas.wizard=skip` marks setup done when an account already exists.

## Pages

Order: Welcome, Language, Keyboard, Wi-Fi, Time Zone, Account, Hostname,
Appearance, Privacy, Finish. First login: Fingerprint, PIN.

- **installer.ini** (`/etc/atlasos/installer.ini`, `[Installer]`, `Version=1`,
  written by Atlas Installer): Language is skipped when `Language` is
  non-empty; Keyboard when `KeyboardLayout` is non-empty; Wi-Fi when
  `Network=true` and NetworkManager reports full connectivity when the page
  would show. No file, an unreadable file, or another `Version` with these
  keys missing: every page shows. A higher `Version` is read the same way
  (readers accept higher versions).
- **Accessibility from the first screen**: Screen Reader, Larger Text and High
  Contrast on the Welcome page, and Meta+Alt+S turns the screen reader on
  anywhere. They change the wizard at once (Orca; the setup user's
  `kdeglobals` font and colour scheme, which Kirigami follows live) and are
  carried to the new account.
- **Account**: full name, user name (derived from the full name until edited),
  password and confirmation with a strength meter, "Sign in automatically"
  (off). Rules (decided 2026-10-05): user name `^[a-z_][a-z0-9_-]{0,31}$`, not
  a name in passwd or group, not reserved; full name at most 255 bytes, no
  `:`, `,`, `=`, newline or control characters; password at least 8
  characters, not the user name or full name, and libpwquality's check
  (dictionary included) passes.
- **Hostname**: only when the static hostname is unset, `localhost*` or
  `fedora`, prefilled `<user>-pc`.
- **Appearance**: AtlasOS Light / Dark pictured as desktops, and accent
  swatches (violet first).
- **Privacy**: "Send crash reports" off by default, with what a report holds.

Look: as the patched plasma-setup (welcome with the AtlasOS logo, a card with
a shadow, big bold titles, step dots, the step forward in the accent colour),
built from Atlas.Ui: `AtlasWindow` full screen with no close, `AtlasOnboarding`,
`AtlasTextField`, `AtlasPasswordField`, `AtlasComboBox`, `AtlasSwitch`,
`Section`/`SectionRow`, `PrimaryButton`/`SecondaryButton`, `StatusHero`.
What Atlas.Ui lacks is asked of the framework; until then a local copy has a
`Wizard` prefix so it never clashes with an Atlas.Ui type. Atlas.Ui 1.5.0
will have them (framework ROADMAP item 42), so each stand-in mirrors the
1.5.0 API exactly and moving to it is a rename:

| Stand-in (0.1.0, on 1.4.0) | Atlas.Ui 1.5.0 | API to mirror |
|---|---|---|
| `WizardOnboarding` | `AtlasOnboarding` additions | `nextText`, `finishText`, `backText` (empty = built-in); `busy` (Next shows a spinner and ignores input, Back and Skip disabled, `Accessible.description` "Busy"); `autoAdvance` (default true) and `advanceRequested(int index)` on every Next, the app calls `next()` itself when false; `canGoBack` (default true; false hides Back, Alt+Left does nothing); `stepStyle: Column \| Dots` (current dot wider in accent, past accent 45 %, future text 20 %, width animation off under reduced motion) |
| `WizardPasswordStrength` | `AtlasPasswordStrength` | `score` 0–4, -1 = nothing typed (empty bar, no label); `text` (empty = "Very Weak" … "Strong"); accessible value "<label>, <score> of 4" |
| `WizardChoiceCard` | `AtlasChoiceCard` | AbstractButton, checkable; `source`, `text`, `checked`, `aspectRatio` (1.6); check circle, checked ring, hover ring |
| `WizardAccentPicker` | `AtlasAccentPicker` | `model` (colours or `{color, name}`), `currentIndex`, `currentColor` (read-only), `activated(int)`; arrows, Home/End; accessible name = name or "Accent color N" |
| `onClosing: close.accepted = false` + `visibility: Window.FullScreen` | `AtlasWindow.kiosk` | full screen, no close, close requests refused |

## Threading and errors

The GUI thread never blocks: every D-Bus call and file read runs on a worker
and posts back with `qt_thread().queue`. Every D-Bus call has a timeout (25 s,
120 s for `CreateAccount`, which can wait for AccountsService to start).
Every error reaches the page in plain words with a way on (Try Again, Skip,
Back). Logs: `journalctl -t atlas-wizard`, `-t atlas-wizard-helper`,
`-t atlas-wizard-boot`; each decision and step is logged, a password never is
(tests check the logs and state for the test password).

## Budgets

Measured in the P phase against plasma-setup's full-Plasma session as the
baseline: wizard window within 1.5 s of the setup session starting on a cold
boot; wizard RSS ≤ 140 MB; the whole setup session PSS ≤ 350 MB; idle CPU
< 1 %; the helper ≤ 10 MB and gone when idle. Lists (locales, layouts, time
zones) load on a worker on first view.

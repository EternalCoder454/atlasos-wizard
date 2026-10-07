# Telamon Wizard: design

Telamon OS's first-run setup. It runs on first boot, before any user account
exists, as its own locked system user, creates the account, and hands over to
the login screen. It replaces Fedora's plasma-setup (which Telamon OS rebuilt
with a style patch) and the `atlasos-fingerprint-setup` autostart.

App ID `net.eterneon.telamon.wizard`, binary `telamon-wizard`, repository
`EternalCoder454/atlasos-wizard`, version 0.2.0, MIT. Built on
Telamon.Ui `v2.0.0` (`ui: "2.0.0"`). The plan and checklist are the Atlas Notes
notes "AtlasOS/Wizard/Plan" and "AtlasOS/Wizard/Roadmap". This file is the
contract: change it together with the code that changes what it says.

## Layout

```
Cargo.toml                      workspace
crates/wizard-core/             no Qt, no D-Bus: validation, installer.ini, state
                                file, boot decisions, per-user settings, password
                                hashing (libxcrypt) and quality (libpwquality)
crates/wizard-helper/           telamon-wizard-helper: the root D-Bus helper
crates/wizard-boot/             telamon-wizard-boot: prepare (before the display
                                manager) and the text-mode fallback
apps/telamon-wizard/              the GUI: CXX-Qt backend (src/), QML (qml/), C++ (cpp/)
data/                           everything installed that is not a binary:
  systemd/ dbus-1/ polkit-1/ sysusers.d/ tmpfiles.d/ wayland-sessions/
  autostart/ libexec/ dnf/
packaging/telamon-wizard.spec     the RPM; build-rpm.sh builds it in fedora:44
scripts/dev.sh                  runs a command in the dev container
tests/                          integration tests (private system bus, mocks)
docs/                           this file, the image hand-over
```

## Programs

| Program | Runs as | What |
|---|---|---|
| `/usr/libexec/telamon-wizard-boot prepare` | root, `telamon-wizard-boot.service`, every boot before the display manager | Reads the state and the system, decides (table below), writes or removes the setup autologin, cleans up and locks `telamon-setup` once setup is done. Fast: a few stats when done. |
| `/usr/libexec/telamon-wizard-boot fallback` | root, `telamon-wizard-fallback.service` on tty1 | Text-mode account creation, when the GUI cannot run. |
| `/usr/libexec/telamon-wizard-session` | `telamon-setup`, the plasmalogin autologin session `telamon-wizard` | Starts `kwin_wayland` with only the wizard, the screen reader's bus, the on-screen keyboard. Counts the starts this boot that did not end with setup done in `/run/telamon-setup/session-failures` (a wizard that exits 0 without finishing counts too, the count is reset only once a done marker exists, a start that cannot write the count gives up at once, and a failed start waits 2 s before plasmalogin's relogin) and failed `GiveUp` calls in `giveup-tries` (after 10 it logs and asks logind to reboot, the stock `org.freedesktop.login1.reboot` allowed to an active local session, then sleeps so plasmalogin stops respawning it; the boot count then reaches the boot fallback row). At 3 it calls the helper's `GiveUp`. Both counts are files of `telamon-setup`, so that user can reset or raise them: availability only (a crash loop that never gives up, or giving up early); the root-kept `boots` count still sends the next boot to the fallback. If that fails: on `SetupDone` (busctl prints the message, `setup-done: ...`) it calls `EndSetup` (clean-up again, display manager restart); on any other failure it logs with `logger -t telamon-wizard-session`, sleeps 5 s after the first failure, 15 s after the second and 30 s after each later one (about 5 minutes over the 10 tries) and exits non-zero, so a restart loop cannot spin and a slow-starting helper is waited for. |
| `/usr/bin/telamon-wizard` | `telamon-setup` | The setup pages. |
| `/usr/bin/telamon-wizard --welcome` | the signed-in user, XDG autostart | First-login extras: fingerprint, PIN. |
| `/usr/libexec/telamon-wizard-helper` | root, D-Bus activated (`telamon-wizard-helper.service`, `Type=dbus`), exits after 30 s idle | The only privileged code the GUI reaches. |

## First boot

1. `telamon-wizard-boot.service`: `Type=oneshot`,
   `Before=display-manager.service plasmalogin.service`,
   `After=systemd-sysusers.service systemd-tmpfiles-setup.service systemd-user-sessions.service`,
   `ConditionKernelCommandLine=!rd.live.image`, `WantedBy=multi-user.target`.
   It runs at every boot (not only the first), so cleanup is re-checked.
   Its sandbox and the fallback unit's leave out `NoNewPrivileges` on
   purpose, as the helper's does: chage, usermod, useradd and chpasswd run in
   their own SELinux domains, and it blocks the transition. Both allow only
   `AF_UNIX` sockets and no writable and executable memory. The Telamon OS VM
   test confirms these under SELinux enforcing, and sets a
   `CapabilityBoundingSet` from what that run needs.
2. When setup is needed it writes `/etc/plasmalogin.conf.d/99-telamon-wizard.conf`:

   ```ini
   [Autologin]
   User=telamon-setup
   Session=telamon-wizard
   Relogin=true
   ```

3. plasmalogin signs `telamon-setup` into `telamon-wizard.desktop`
   (`/usr/share/wayland-sessions/`, `NoDisplay=true`, `Hidden` from the
   session list), which runs `telamon-wizard-session`.
4. The wizard's pages. System-wide choices apply at once through localed,
   timedated, hostnamed and NetworkManager (polkit rule below). The answers
   so far (never the password) are kept in `/run/telamon-setup/answers.json`,
   so a crash of the session comes back on the same page.
5. Account page Next: helper `CreateAccount`. The account exists from then on.
6. Finish: helper `Finish`; the finish screen's button: helper `EndSetup`,
   which restarts `display-manager.service`. The login screen shows with the
   new account selected; when the user ticked "Sign in automatically", the
   autologin drop-in `/etc/plasmalogin.conf.d/50-telamon-autologin.conf` signs
   them in instead. (Typing the password once at the login screen unlocks the
   wallet; that is why there is no one-time autologin.)
7. First login: `telamon-wizard --welcome` (autostart) offers fingerprint
   enrolment and a PIN, each only when it applies.

### The setup session

Minimal on purpose (decided 2026-10-05): it must appear fast on a cold boot
and fit a 4 GB machine. `kwin_wayland` runs the wizard alone; no plasmashell,
kded, wallet, notifications, lock screen or portals. at-spi starts on demand
for Orca; plasma-keyboard is KWin's input method. The wizard shows network
and battery itself. If a GPU misbehaves with KWin alone in the VM tests, the
fallback is the same session file running a full Plasma session.

The setup user `telamon-setup` (sysusers, `u telamon-setup - "Telamon Setup" /run/telamon-setup /bin/sh`,
password `!*`) has its home on `/run` (tmpfiles, mode 0750, owned by it), with
the wizard's own `kdeglobals` and `kxkbrc` there. Its settings never reach
the new account except through `Finish`.

## The helper

Bus name `net.eterneon.telamon.WizardHelper`, object `/net/eterneon/telamon/WizardHelper`,
interface `net.eterneon.telamon.WizardHelper1`. Every method:

- checks the caller's uid is `telamon-setup`'s (`GetConnectionUnixUser` on the
  sender, from the bus, not an argument) before anything else: a call from
  another uid does no work, asks polkit nothing and does not reset the idle
  timer; refusals are logged at most once a second, with a count of the rest;
- then authorizes the caller with `telamon_framework_system::polkit::check`
  against its own action (non-interactive);
- refuses with `net.eterneon.telamon.Error.SetupDone` once a done marker
  exists (`CreateAccount`, `Finish` and `GiveUp`; `EndSetup` runs after the
  markers and instead needs the state's `finish` to be `done`);
- validates every argument again with wizard-core (never trusts the GUI);
- takes no path, command, argv or unit name.

The bus policy (`system.d/net.eterneon.telamon.WizardHelper.conf`) lets only
root and `telamon-setup` send to the helper; the default stays deny. The process
turns `PR_SET_DUMPABLE` off at start and the unit has `LimitCORE=0` and a
capability allow-list (see the unit; to be verified in the VM).

Errors are `net.eterneon.telamon.Error.{NotAuthorized,SetupDone,Invalid,AccountsService,Failed}`,
with the message `<code>: <English text>`; the GUI splits on the first `: `
and shows its own translated text for the code. `gave-up` (`Invalid`) is the refusal of `CreateAccount` and `Finish` after `GiveUp`.

| Method | Polkit action | Does |
|---|---|---|
| `CreateAccount(s name, s full_name, ay password, b autologin) -> u uid` | `net.eterneon.telamon.wizard.create-account` | AccountsService `CreateUser(name, full_name, 1)` (administrator: wheel), then `SetPassword(yescrypt hash, "")` with the hash made in the helper. State stages `creating`, `created`, `password-set`, `verified` are written before and after each step. Verifies the passwd entry, the shadow hash, wheel, and the home owned by the uid. One account per first run: a second call fails unless the state names a half-made account, which is deleted first (only that uid, only when its home holds nothing but skel; a `creating` stage, which has no uid yet, is deleted by name only when its uid is 1000..=60000, its shadow hash is absent, locked or empty and its home is skel-only, else `half-made-account-unclear`). Refused with `Invalid` `gave-up` ("Setup moved to text mode.") once `GiveUp` has run. A uid outside 1000..=60000 from `CreateUser` is refused (`accounts-bad-uid`) and the state stays `creating`. The helper zeroes its own copies of the password; it cannot zero zbus's message buffers, which is why core dumps are disabled (`PR_SET_DUMPABLE`, `LimitCORE=0`). |
| `Finish(a{sv} choices)` | `net.eterneon.telamon.wizard.finish` | Writes the new account's settings as that user (below), the autologin drop-in if asked, then the done markers, removes the setup autologin, locks `telamon-setup`. Refused with `Invalid` `gave-up` once `GiveUp` has run, unless it resumes `finish` = `markers`. Idempotent: a repeat after a crash finishes the remaining steps (state `finish`: `settings`, `autologin`, `markers`, `done`; once the markers exist only the clean-up is redone, and its failures are logged, not returned, since `prepare` redoes it at the next boot). |
| `EndSetup()` | `net.eterneon.telamon.wizard.finish` | Verifies the clean-up (the setup autologin drop-in is absent, `telamon-setup`'s shadow expire field is 0 or a past day, its shell is `/usr/sbin/nologin`), redoing it once if not and failing with `cleanup-incomplete` without restarting if it still is not; then restarts `display-manager.service` (systemd D-Bus, fixed unit). When the markers exist but `finish` is still `markers` (a cut, or GiveUp's retry) and the state's account is `verified` and passes `accounts::verify` (else `not-finished`, clean-up left to `prepare`), it first runs Finish's tail (remove the setup autologin, lock `telamon-setup`, `finish` = `done`). Repeatable. |
| `GiveUp()` | `net.eterneon.telamon.wizard.fallback` | Records it in the state, removes the setup autologin, starts `telamon-wizard-fallback.service`. Not refused when the state's account is `verified` or `finish` is set: the fallback then finishes without asking when passwd lists the account (refusing would strand the machine when the wizard crashes after the account is made). Done markers, or markers that cannot be checked, give `SetupDone`. |

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
`~/.config/telamon/crash-reporting.toml` through the framework's
`crash::Settings::save_to` (and `~/.config/atlas/crash-reporting.toml`, the same
file, where Atlas.Ui 1.x apps read it until they have moved). Each write is temp file + rename.

## Polkit

`/usr/share/polkit-1/actions/net.eterneon.telamon.wizard.policy`: the three
actions above, every default `no`. `/usr/share/polkit-1/rules.d/50-telamon-wizard.rules`
returns YES only when `subject.user == "telamon-setup" && subject.local &&
subject.active` and none of `/etc/telamon/setup-done`,
`/etc/atlasos/setup-done` (Atlas Wizard's) and `/etc/plasma-setup-done` exists (checked with
`polkit.spawn(["/usr/bin/test", "-d", parent, "-a", "-x", parent, "-a", "!", "-e", path])` with
parent `/etc/telamon`, `/etc/atlasos` or `/etc`, which tmpfiles.d creates; a marker, a missing
or unsearchable parent (checked with access(2); that only proves the parent can be searched, a stat failure on the marker file itself still reads as "no marker", and the helper's fail-closed marker check is the backstop), or a spawn that fails, gives NO). The
helper likewise counts a marker it cannot stat as done, for our three actions and these stock ones. The exception is
`net.eterneon.telamon.wizard.finish`: `Finish` writes the markers first and
`EndSetup` uses the same action after them, so it is not subject to the marker
check; the helper's own state checks bound it (`SetupDone` unless `finish` is
`markers` or `done`). `tests/helper/polkit-rules.test.mjs` tests the rule with
a stub `polkit`.

- `org.freedesktop.locale1.set-locale`, `org.freedesktop.locale1.set-keyboard`
- `org.freedesktop.timedate1.set-timezone`, `org.freedesktop.timedate1.set-ntp`
- `org.freedesktop.hostname1.set-static-hostname`, `org.freedesktop.hostname1.set-hostname`
- `org.freedesktop.NetworkManager.settings.modify.system`,
  `org.freedesktop.NetworkManager.network-control`,
  `org.freedesktop.NetworkManager.enable-disable-wifi`

plasma-setup's other grants (its KAuth actions, display scaling, temporary
autologin) are not carried over. After setup `telamon-setup` can have no
session (below), so the rule can never match again, and the markers close it
even if the clean-up failed.

## Done markers

- `/etc/telamon/setup-done`, ours: `[Setup]` `Version=1`, `Finished=<RFC 3339>`,
  `Wizard=<version>`. Written temp + fsync + rename + directory fsync.
- `/etc/atlasos/setup-done`, Atlas Wizard's marker (0.1.x), same format. Also
  written for one release, so the image's `health-lib` and a rollback to an image
  with Atlas Wizard keep working. A machine that has only this one (set up by
  Atlas Wizard) is done, and the first boot adds `/etc/telamon/setup-done`, with
  its `Finished=` time.
- `/etc/plasma-setup-done`, also written, so the image's `health-lib` and
  `pin-setup` keep working and a rollback to an image that still has
  plasma-setup never runs it again.
- An existing `/etc/plasma-setup-done` (a machine set up before this wizard)
  counts as done.
- A marker that cannot be checked (stat fails: EACCES, SELinux, I/O error)
  counts as there, in `prepare`, the fallback and the helper alike
  (`wizard_core::markers`), so a stat failure never reopens setup on a
  finished machine.

## State and recovery

`/var/lib/telamon-wizard/state.json` (root, 0644, no secrets), written atomically:

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
| a done marker | cleanup: remove the setup autologin drop-in, lock `telamon-setup`, write the missing marker; normal login screen |
| `telamon.wizard=skip` and a usable human account | mark done, cleanup |
| a usable human account (uid 1000–60000, a password hash, a login shell) not in the state | mark done, cleanup (Anaconda, kickstart) |
| account `verified`, Finish not done, `boots` < 3 | setup autologin; the wizard resumes after the Account page |
| account `verified`, `boots` ≥ 3 or `gave_up` | finish with defaults (markers, cleanup); login screen |
| `gave_up`, `boots` ≥ 3 with no account, or `telamon.wizard=fallback` | start `telamon-wizard-fallback.service` (text mode); no setup autologin |
| otherwise | setup autologin, `boots` + 1 |
| account made (`verified`), then the wizard crashes 3 times in one boot (a later boot with `gave_up` set and the fallback not finished also takes finish-with-defaults, whatever `boots` is) | the session script calls `GiveUp` (allowed); the fallback starts, sees the verified account and finishes without asking when passwd lists it, or when passwd or shadow cannot be read (logged; asking might make a second account); a readable passwd without it (`/etc` reset) clears the note and asks; `finish` alone proves nothing; login screen |

Details of the table (`wizard_core::boot`):

- "Finish not done" means `finish` is unset; a verified account whose Finish
  had begun (and no marker) is finished with defaults at once.
- `telamon.wizard=fallback` only applies when there is no verified account: with
  one, the account exists and the machine is finished with defaults at once
  (whatever `boots` is).
- "Not in the state" means its name and uid differ from the state's account.
- A state account that is not usable (no shadow hash, missing) counts as
  half-made, whatever its stage says. An unknown stage (from a newer wizard)
  is kept as it is and counts as verified when `accounts::verify` passes,
  half-made otherwise.
- installer.ini values of an impossible shape count as unset (the page shows).

A half-made account (`creating`, `created`, `password-set`) is resolved by the
helper at the next `CreateAccount` (deleted, made again), and by the fallback
the same way, with the same bar: the uid in passwd must be a human one and
match the state's, and the home hold only what `/etc/skel` put there. At
`creating` the uid was never recorded (0), so the state vouches for the name
alone: the account must also have no usable password hash, as `useradd`
leaves it, so an account of that name made some other way is never deleted.
A `verified` account that passwd no longer lists (/etc reset, `userdel` by
hand) is a stale note everywhere: the helper and the fallback forget it and
set up again, and `prepare` runs the wizard again; one that cannot be checked (passwd or shadow
unreadable) still counts as made.

**Locking `telamon-setup`:** account expired (`chage -E 0`, which pam_unix's
account check refuses even for autologin), shell `/usr/sbin/nologin`, the
setup autologin drop-in removed, `/run/telamon-setup` emptied, and its logind
user terminated. `prepare` checks all of it at every boot and fixes what is
missing, and `EndSetup` checks the first three before restarting the display
manager. When the wizard must run again (a cut after the lock but before the
markers, or markers removed by hand), `prepare` first undoes the lock
(`chage -E -1`, shell `/bin/sh`, each only if needed) so the setup autologin
can log in.

**The fallback** (`telamon-wizard-fallback.service`, `Conflicts=display-manager.service`,
`TTYPath=/dev/tty1`, `StandardInput=tty`): asks for the full name, user name
and password twice on the console, with the same wizard-core validation, and
creates the account with `useradd -m -U -G wheel -c <name> <user>` and
`chpasswd -e` (the hash on stdin), fixed argv (`--` before the user name). It is the one path that does
not use AccountsService, so a broken AccountsService still ends in an
account. Then it writes the markers, cleans up and starts the display manager.
No Qt, GPU or network needed. The password is typed into it, so it turns
`PR_SET_DUMPABLE` off and its unit has `LimitCORE=0`; the line buffer is
reserved once (never moved) and wiped after each line, and a terminal whose
echo cannot be turned off is not asked for a password. It is never given up
on (`StartLimitIntervalSec=0`): while it runs there is no account and the
login screen is stopped, so it restarts after a failure, backing off from
10 s to a minute.

**Kernel command line, for support:** `telamon.wizard=fallback` forces the text
mode; `telamon.wizard=skip` marks setup done when an account already exists.
`atlas.wizard=` (its name until 0.2.0) is read as well; the last of the two wins.

## The names until 0.2.0 (Atlas Wizard)

Telamon Setup was Atlas Wizard up to 0.1.x. The image and installed systems name
the old things, so for one release both work, and a machine Atlas Wizard set up
is never set up again:

| Old name | Now | What keeps the old one working |
|---|---|---|
| package `atlas-wizard`, programs `atlas-wizard`, `/usr/libexec/atlas-wizard-{boot,helper,session}` | `telamon-wizard`, `telamon-wizard-*` | `Obsoletes:` and `Provides: atlas-wizard`; links |
| units `atlas-wizard-{boot,fallback,helper}.service` | `telamon-wizard-*.service` | each old name is a link to the new unit (an alias), so the enable link of an installed system and `systemctl enable|start` reach it; the preset enables the new name |
| `/etc/atlasos/setup-done` | `/etc/telamon/setup-done` | both are written and both count as done (also in the polkit rule and the session script) |
| `/var/lib/atlas-wizard/state.json` | `/var/lib/telamon-wizard/state.json` | moved once, atomically, when the new one is missing |
| `/etc/atlasos/installer.ini` | `/etc/telamon/installer.ini` | read when the new one does not exist |
| `atlas.wizard=skip\|fallback` on the kernel command line | `telamon.wizard=` | both are read |
| setup user `atlas-setup`, session `atlas-wizard`, `/run/atlas-setup` | `telamon-setup`, `telamon-wizard`, `/run/telamon-setup` | `prepare` removes `99-atlas-wizard.conf` wherever it finds it and locks `atlas-setup` if the machine has it and it is open; the name stays reserved for the new account |
| `net.eterneon.atlas.WizardHelper`, the `net.eterneon.atlas.wizard.*` polkit actions | `net.eterneon.telamon.WizardHelper`, `net.eterneon.telamon.wizard.*` | nothing but this program uses them: renamed outright |
| `net.eterneon.atlas.wizard.desktop`, `/etc/dnf/protected.d/atlas-wizard.conf` | `net.eterneon.telamon.wizard.desktop`, `telamon-wizard.conf` | the old files stay (hidden desktop file, `NoDisplay=true`; the old protected file names both packages) |
| `~/.config/atlas/crash-reporting.toml` | `~/.config/telamon/crash-reporting.toml` | the choice is written to both, for the apps that have not moved |
| `~/.config/atlas-wizardrc` | `telamon-wizardrc` | copied once by the framework |

What the image still has to change is listed in the pull request that renamed
this app (Look and feel IDs `org.atlasos.*`, the icon `atlasos`, the colour scheme
`AtlasOSHighContrast`, the checks in `apps.sh`, `health-lib`, `vmctl.py`).

## Pages

Order: Welcome, Language, Keyboard, Wi-Fi, Time Zone, Account, Hostname,
Appearance, Privacy, Finish. First login: Fingerprint, PIN.

- **installer.ini** (`/etc/telamon/installer.ini`, or Atlas Installer's
  `/etc/atlasos/installer.ini` when there is none; `[Installer]`, `Version=1`,
  written by the installer): Language is skipped when `Language` is
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
  `:`, `,`, `=`, newline, control characters or invisible format characters
  that disguise it (bidi controls, zero-width space, BOM, tags; the joiners
  ZWJ and ZWNJ are allowed); password at least 8
  characters, not the user name or full name, and libpwquality's check
  (dictionary included) passes.
- **Hostname**: only when the static hostname is unset, `localhost*` or
  `fedora`, prefilled `telamon` (the image's `DEFAULT_HOSTNAME`, which still says `atlasos`
until the image moves; nothing
  personal goes on the network unless the user types it).
- **Appearance**: Telamon OS Light / Dark pictured as desktops (drawn, in the
  chosen accent), and accent swatches (violet first) with the colour's name.
- **Privacy**: "Send crash reports" off by default, with what a report holds.

Look: as the patched plasma-setup (welcome with the Telamon OS logo, a card with
a shadow, big bold titles, step dots, the step forward in the accent colour),
built from Telamon.Ui: `TelamonWindow` full screen with no close, `TelamonOnboarding`,
`TelamonTextField`, `TelamonPasswordField`, `TelamonComboBox`, `TelamonSwitch`,
`Section`/`SectionRow`, `PrimaryButton`/`SecondaryButton`, `StatusHero`.
What Telamon.Ui lacks is asked of the framework; until then a local copy has a
`Wizard` prefix so it never clashes with a Telamon.Ui type. The password
strength bar and the accent picker moved to `TelamonPasswordStrength` and
`TelamonAccentPicker` (2.0.0) and their stand-ins are gone. What is local:

| Local type | Why | What it is |
|---|---|---|
| `WizardOnboarding` | `TelamonOnboarding` has no hidden pages | The card, the step dots, Back / Skip / Next. `finishText`; `busy` (Next shows a spinner and ignores input, Back and Skip disabled); `autoAdvance` and `advanceRequested(int index)` on every Next, the app calls `next()` itself when false; `canGoBack`; a step with `hidden: true` is left out of the dots and of Back and Next; a page's `submitted` signal (Return in a field) is Next. A page fades in by a card-coloured cover fading out (the page is drawn once). |
| `WizardStep` | start-up cost | Holds a page as a `Component` and makes it the first time it shows, or 400 ms after the page before it (so Next never waits). Once made it stays with what the user typed. `hidden` is the step's (known before the page exists); `title`, `canAdvance` and `skippable` are read from the page; `setError()`, `enter()`, `item`. |
| `WizardPage` | the page frame | Title, subtitle, error banner, body column. It scrolls when its content needs more than the card gives (a small screen, larger text). |
| `WizardChoiceCard` | `TelamonChoiceCard` only takes an image | Its look and its `checked` binding rule, with the picture declared inside (`default property preview`). |
| `WizardThemePreview` | the Appearance picture | A 16:10 picture of the Telamon OS desktop in Light or Dark drawn with shapes (wallpaper, the menu bar's islands, a Telamon.Ui window, the dock), in the colours TelamonStyle derives from the TelamonLight and TelamonDark schemes and the chosen accent, every edge snapped to a device pixel. No image, so it is sharp at any scale. |
| `WizardLogo` | no `telamon` icon in a container or minimal session | The Telamon mark drawn with shapes (the welcome page). It is not looked up in the icon theme. |
| `WizardPicker` | filterable list | Search field and `TelamonListView` with `TelamonStatus` (loading, no results). It picks by the list's selection, not `currentIndex` (which starts at 0, so the first row could not be chosen). |
| `onClosing: close.accepted = false` + `visibility: Window.FullScreen` | `TelamonWindow.kiosk` | full screen, no close, close requests refused |

## Threading and errors

The GUI thread never blocks: every D-Bus call and file read runs on a worker
and posts back with `qt_thread().queue`. Every D-Bus call has a timeout: 25 s, and
for the helper's long calls longer than the helper's own limit, so the
helper's error arrives before the GUI gives up on a call still working
(`CreateAccount` 130 s against 120 s, as it can wait for AccountsService to
start; `Finish` 130 s against 120 s; `EndSetup` 75 s).
Every error reaches the page in plain words with a way on (Try Again, Skip,
Back). Logs: `journalctl -t telamon-wizard`, `-t telamon-wizard-helper`,
`-t telamon-wizard-boot`; each decision and step is logged, a password never is
(tests check the logs and state for the test password).

## Budgets

Measured in the P phase against plasma-setup's full-Plasma session as the
baseline: wizard window within 1.5 s of the setup session starting on a cold
boot; wizard RSS ≤ 140 MB; the whole setup session PSS ≤ 350 MB; idle CPU
< 1 %; the helper ≤ 10 MB and gone when idle. Lists (locales, layouts, time
zones) load on a worker on first view; a page is made when first shown
(`WizardStep`). `scripts/bench.sh` measures start-up, memory and CPU in demo
mode; `scripts/screenshots.sh` takes every page and state in both colour
schemes at 1x, 1.7x and 2x (headless, in the dev container).

## Demo mode and start-up probes

- `TELAMON_WIZARD_DEMO=1` runs the GUI on canned data. In it, the user name
  `fail` makes `CreateAccount` fail (any other name waits 3 s first), the
  Wi-Fi password `wrong` fails, and `TELAMON_WIZARD_ANSWERS=<file>` names the
  answers file (to jump to a page for screenshots). The variable is read only
  in demo mode; otherwise the path is fixed at `/run/telamon-setup/answers.json`.
- The reads at start-up (NetworkManager connectivity, hostnamed's static
  name) time out after 5 s, not 25 s: a missing service must not hold the
  first page. The page then shows as if the answer were "no".


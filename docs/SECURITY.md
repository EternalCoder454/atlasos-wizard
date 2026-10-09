# Setup: security

The threat model of Telamon Setup (the first-run wizard): what it protects,
who it defends against, the rule each entry point follows, and the test that
keeps the rule true. `docs/DESIGN.md` is the contract (programs, helper
methods, polkit, state, recovery); this file says why it is built the way it
is, and what is left. Change it together with the code it describes: some of
the tests named below read the units, policy and rule files and the source,
and fail when a rule here is skipped.

Report a vulnerability privately to the maintainer through GitHub's "Report a
vulnerability" on the repository (Security tab), not in a public issue.

## What Setup is, security-wise

It runs on the first boot, before any account exists, and it **creates the
first account as root**, with the password the person types. Three pieces
hold privilege:

| Piece | Runs as | Reached through |
|---|---|---|
| `telamon-wizard-boot prepare` | root, every boot before the display manager | nothing: it reads the state, the files and the kernel command line |
| `telamon-wizard-boot fallback` | root, on tty1 | the physical console: text-mode account creation when the GUI cannot run |
| `telamon-wizard-helper` | root, D-Bus activated | the system bus, only from the user `telamon-setup`, then polkit |

The GUI (`telamon-wizard`) is **not trusted**: it runs as the locked system
user `telamon-setup` (password `!*`, home on tmpfs `/run/telamon-setup`),
which is expired, set to `nologin` and emptied when setup is done and at
every boot after. After the account exists, the GUI's first-login extras
(`--welcome`) run as that account, with no privilege.

The helper's surface is four methods (`CreateAccount`, `Finish`, `EndSetup`,
`GiveUp`); none takes a path, a command, an argv or a unit name.

It handles secrets: the new account's password (as D-Bus bytes, a yescrypt
hash, a terminal line) and a Wi-Fi password (handed to NetworkManager).

## Who we defend against

| Attacker | What they can reach | Defended? |
|---|---|---|
| **The GUI process, compromised** (a Qt, QML or library bug, or a hostile Wi-Fi name or locale string that gets it to misbehave) | the four helper methods, as `telamon-setup`, while setup is not done | Yes: see 1 to 3. It can create the one account once, with a name and password it chooses (that is the job), and nothing else |
| **Another local user or service** | the system bus | Yes: the bus policy lets only root and `telamon-setup` send to the helper; the caller's uid is checked from the bus before polkit is asked |
| **A machine that is already set up** | the helper, the setup user | Yes: the done markers close the polkit rule and the helper (fail closed: a marker that cannot be checked counts as there); the setup user is locked at every boot |
| **Someone at the boot loader** (kernel command line) | `telamon.wizard=skip\|fallback` | Partly, by design: `skip` only marks setup done when a usable human account already exists; it cannot create or promote an account. Anyone who can edit the command line owns the machine anyway (GRUB is locked only on encrypted installs) |
| **Files other users can write** | `/run/telamon-setup` (the setup user's), `answers.json`, the new home | Yes: root never reads a file the setup user can write; settings are written into the new home by a child that has dropped to the account's uid |
| **A hostile string typed by the person** (user name, full name, hostname, Wi-Fi password) | argv of `useradd`, the passwd line, INI files, D-Bus calls | Yes: see 2, 4 |
| **The network** | nothing: the wizard has no network code; NetworkManager's own connectivity check is the only traffic | Yes |
| **The supply chain** | crates.io, the pinned `atlas-framework`, CI actions | Partly (see 7) |

Out of scope, because it is not Setup's to stop: physical access to the
machine during setup (the person at the keyboard creates the account, by
design); a malicious or compromised image; bugs in AccountsService, polkit,
NetworkManager, systemd, `shadow-utils`, `libxcrypt` or `libpwquality`.

## 1. The helper: who may call it

- **Bus policy** (`data/dbus-1/system.d`): only root and `telamon-setup` send to the
  helper; no default or group context. The real bus test enforces it.
- Every method, in this order: the caller's uid from the bus
  (`GetConnectionUnixUser` on the sender, never an argument), then polkit
  (non-interactive; the subject is the unique bus name), then the done
  markers (fail closed), then the state machine (`SetupDone`, `gave-up`,
  half-made account), then argument validation with `wizard-core`. A call from
  another uid does no work and does not reset the idle timer.
- **Polkit** (`data/polkit-1`): the three actions default to `no`. The rule
  returns `YES` for `telamon-setup`'s active local session only while no
  done marker exists, for a fixed list of ten actions (locale, keyboard, time
  zone, hostname, NetworkManager system connections and control, and the
  wizard's own). A rule that cannot check the marker answers `NO`.
  `timedate1.set-ntp` and `NetworkManager.enable-disable-wifi` were granted and
  never used; they were removed.
  *Tests:* `tests/data` (policy and rule files), `tests/helper/bus.rs`,
  `tests/helper/polkit-rules.test.mjs` (the rule against a stub polkit).
- **Size and shape:** a 20 MB password, user name or full name, 300 `choices`
  keys, a 4 MB key and variants nested 5 deep or more are refused and the
  helper survives (`MAX_VARIANT_NEST`). *Tests:* `service::props::deeply_nested_and_huge_values`
  (overflowed the stack before the fix), `hostile_arguments_are_refused_and_the_helper_survives`.
  Huge messages are still parsed by zbus before authorization (only
  `telamon-setup` and root can send them).
- **Process:** `PR_SET_DUMPABLE` 0 first thing; `LimitCORE=0`; the setup session
  script runs `ulimit -c 0` before KWin and the GUI start (the GUI holds the
  typed password in memory it cannot wipe). *Tests:* `tests/data`.

## 2. Creating the account

- **User name** `^[a-z_][a-z0-9_-]{0,31}$`, not in passwd or group, not in
  `RESERVED_NAMES` (root, the setup users, `systemd-*`, and the accounts
  packages create: `sshd`, `dbus`, `polkitd`, `sys`, `sudo`, `www-data`,
  `pipewire`, ...). **Full name** at most 255 bytes, none of `:` `,` `=`,
  control characters, or bidi and invisible characters that disguise it.
  *Tests:* `validate::props` (never panics; accepted names have the shape; the
  reserved accounts are refused), `tests/container/real-tools.sh` (everything
  `wizard-core` accepts, the real `useradd` takes and stores unchanged).
- **Argument injection:** the fallback runs `useradd -m -U -G wheel -c <full name> -- <user>` and
  `chpasswd -e` (the hash on stdin), argv only, environment cleared, a
  timeout. `--` stops option parsing for names like `-x` and `--root=/`.
  *Test:* `real-tools.sh` (127 checks against the real shadow-utils).
- **Only the first admin is in `wheel`:** AccountsService `CreateUser(.., 1)`
  or `-G wheel`; one account per first run.
- **No existing-user takeover:** `CreateUser` fails on an existing name; the
  uid it returns must be in 1000..=60000. A half-made account from a crash is
  deleted (`userdel -r`) only when the state's uid equals passwd's, is in range,
  the home holds only `/etc/skel`'s files and belongs to that uid, and, for the
  `creating` stage (which has no uid yet), the password hash is unusable.
- **No empty-password window:** `useradd` leaves `!!`, AccountsService `!`; the
  yescrypt hash is set afterwards.
- **The home is private:** `useradd` and AccountsService give 0755 (0777 under a loose
  umask: confirmed with the real `useradd`). The helper and the fallback chmod it
  `go-rwx` through an fd checked to be that directory (not a symlink, owned by the
  uid) (`accounts::secure_home`), and `verify` refuses a home others can write.
  *Tests:* `the_new_home_is_private_whatever_the_system_gave` (failed before the
  fix), `finish_refuses_a_home_others_can_write`, `real-tools.sh`.

## 3. Passwords

- They travel as bytes (D-Bus `ay`, the console), are hashed with yescrypt
  (`$y$`, libxcrypt `crypt_rn`, default cost; the FFI checks buffer sizes,
  NUL, NULL returns and zeroes its data), and are zeroed after use. The hash
  goes to AccountsService or to `chpasswd -e` on stdin.
- **Nothing secret is logged or stored:** not in argv, the environment, the
  journal, `state.json`, `answers.json`, an error or a panic message
  (no `unwrap` or `expect` in production code). The journal holds codes and
  the user name; not the full name, the Wi-Fi name or any hash. *Tests:*
  the canary tests (`tests/boot/fallback.rs` over a pty: no password in the
  terminal output, stderr, state, command log or any file of the root;
  `no_password_in_any_error_or_debug`), `tests/data::the_gui_logs_no_password_ssid_or_full_name`.
- **The console:** echo is switched off *before* the prompt is written and
  restored on every exit path, signals included; a terminal whose echo cannot
  be switched off is never asked for a password; the line buffer is reserved
  once and wiped after each line.
- Password rules: at least 8 characters, not the user name or full name,
  libpwquality (dictionary included).
- The Wi-Fi password is borrowed into the NetworkManager call, not copied.
  *Test:* `the_wifi_passphrase_is_borrowed_into_the_call_not_copied` (static).

## 4. Values that go to other services and files

- **Hostname, time zone, locale, keyboard** go from the GUI to hostnamed, timedated and
  localed over D-Bus under polkit; the helper never sets them. The GUI checks
  them (`hostname_ok` refuses `localhost`, `zone_id_ok` refuses `a//b`, `a/`, `-x`;
  XKB names must start with a letter or digit) and the helper validates the
  keyboard again in `Finish`. *Tests:* `data::props`, `choices::props`,
  `system::tests::hostile_strings_never_reach_the_bus`.
- **Settings for the new account** (`Finish`) are written by a child that has
  dropped to the account's uid and gid (`setgroups`, `setresgid`, `setresuid`,
  `NO_NEW_PRIVS`, environment cleared to a fixed list, stdin capped at 16 KiB,
  60 s), writing under its home with `O_NOFOLLOW` and temp + rename. Programs
  and arguments are fixed (theme ids and `#rrggbb` come from static lists).
  The autologin drop-in re-validates the name. An INI key that could forge a
  section header is refused (`safefs::ini_set`).
- **Root reads no file the setup user can write.** `installer.ini` is read by the
  GUI only (64 KiB cap); `answers.json` (0600, tmpfs, no password field) is
  never read by root; `state.json` (root, 1 MiB cap, nesting-depth limited) and the
  done markers are written atomically in root-owned directories.

## 5. Boot, recovery and the setup user

- `prepare` always exits 0 and decides by a pure function
  (`wizard_core::boot::decide`, a test per row). `telamon.wizard=skip` marks
  setup done only with an existing human account (`boot::props`).
- Locking `telamon-setup` is checked at every boot and before `EndSetup` restarts
  the display manager: expired (`chage -E 0`), shell `nologin`, the setup
  autologin drop-in removed, `/run/telamon-setup` emptied without following
  links.
- **Test hooks cannot ship:** every `TELAMON_WIZARD_TEST_*` variable is behind the
  `test-root` feature, which fails to compile in a release build; the package
  build refuses a program that contains the string (`scripts/check-hardening.sh
  --forbid-string`). The environment variables read in production are pinned by a
  test (`ENV_READS`). The GUI reads `TELAMON_WIZARD_DEMO` and
  `TELAMON_WIZARD_ANSWERS` (it has no privilege and no helper in demo mode).

## 6. Units and data files

The boot, fallback and helper units carry: `PrivateTmp`, `ProtectKernelTunables`
/`Modules`/`Logs`, `ProtectClock`, `ProtectHostname`, `ProtectControlGroups`,
`LockPersonality`, `RestrictRealtime`, `RestrictSUIDSGID`,
`SystemCallArchitectures=native`, `RestrictAddressFamilies=AF_UNIX`,
`LimitCORE=0`, `ProtectProc=invisible`, `UMask=0022`; boot and fallback also
`ProtectSystem=strict` with fixed `ReadWritePaths`, no writable-executable memory
and `RestrictNamespaces`; the helper's capabilities are limited to `CHOWN`,
`DAC_OVERRIDE`, `FOWNER`, `SETUID`, `SETGID` and `KILL`. *Tests:* `tests/data`
parses the units, the D-Bus policy, the polkit actions and rule, sysusers and
tmpfiles and fails when a key goes. Not set until a VM run proves it:
`NoNewPrivileges` (the design notes say it blocks the SELinux transition of
`useradd`/`chpasswd`; `real-tools.sh` shows these tools work under
`setpriv --no-new-privs`, so only SELinux could break it), a capability bounding
set for boot and fallback, `SystemCallFilter`.

## 7. Build hardening and supply chain

- **Programs:** `scripts/check-hardening.sh` reads the built GUI, helper and boot
  program back with `readelf` (position-independent, `GNU_RELRO` and `BIND_NOW`,
  no executable stack, no `RPATH`, no text relocations, stack protectors in the
  C++) and runs in the RPM's `%check`, so a package that lost them does not
  build; CI tests the script itself with programs built with and without each
  flag (`scripts/test-check-hardening.sh`). `overflow-checks` is on in the
  release profile: root code panics on an overflow instead of wrapping. Intel
  CET `IBT` is reported, not required (rustc has no stable switch).
- **Dependencies:** `Cargo.lock` is committed, every build is `--locked`.
  `deny.toml` (advisories incl. unmaintained and yanked, licences, bans, sources:
  crates.io and the pinned `atlas-framework` git tag only) and `cargo-audit`
  run on every dependency change and every Monday
  (`.github/workflows/security.yml`).
- **CI:** every action pinned by commit, `permissions: contents: read` at the
  top, `packages: write` only in the job that publishes the dev image on `main`,
  `persist-credentials: false`, cache written from `main` only, no
  `pull_request_target`, untrusted values reach shell through `env:`. The dev
  image checks that the `atlas-framework` tag it builds is still the commit
  `Cargo.lock` locks (`ci/framework-sha.sh`): a tag can be moved.

## Tests

| What | How |
|---|---|
| Unit, helper bus and boot tests | `scripts/dev.sh cargo test --workspace --locked` |
| Property tests (20000 cases in CI) | `PROPTEST_CASES=20000 cargo test --workspace --locked -- props` |
| Shipped files, units, policy, logs | `tests/data` (part of `cargo test`) |
| Polkit rule | `node --test tests/helper/polkit-rules.test.mjs` |
| Real shadow-utils | `tests/container/real-tools.sh` (inside the dev container only) |
| Hardening | `scripts/test-check-hardening.sh`, the RPM's `%check` |

## Left

| Item | Why it stays | What would close it |
|---|---|---|
| `NoNewPrivileges`, a capability bounding set for boot and fallback, `SystemCallFilter`, `PrivateDevices`, `ProtectSystem=strict` for the helper | Need an SELinux-enforcing VM to prove they do not break `useradd`/`chpasswd` transitions | A VM run, one key at a time |
| The GUI does not set `PR_SET_DUMPABLE` | A non-dumpable process may hide `/proc/<pid>/exe` from KWin (same uid); unverified | Try it in the VM; `ulimit -c 0` covers dumps meanwhile |
| Copies of the password in zbus buffers, QString and QML cannot be wiped; no `mlock` | Qt and zbus own them; Fedora's swap is zram | none |
| Huge D-Bus messages are parsed before authorization; no `MemoryMax` | Only `telamon-setup` and root can send; the Qt settings child shares the cgroup | VM sizing |
| Extra groups beyond `wheel` are not checked; the fallback does not re-check the uid range after `useradd` | Images that add default groups; a misconfigured `login.defs` only | Check `id <user>` in the VM |
| `password::verify` accepts any hash setting | Used in tests only | Remove or restrict |
| The GUI binary reads the demo variables in production | No privilege; compiling them out is a product decision | A build feature |
| Wi-Fi password stored system-wide by NetworkManager | By design (`settings.modify.system`) | A per-user connection |
| First login (`--welcome`) fingerprint and PIN are not implemented yet | Nothing to audit | When built: fprintd over D-Bus (not `fprintd-enroll` as root), the PIN on stdin |

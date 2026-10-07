# AtlasOS Wizard

AtlasOS Setup (`atlas-wizard`): the first-run wizard of AtlasOS. It asks for the
language, keyboard, network, time zone and the first user, then hands over to
the desktop; started again at the first login (`--welcome`) it offers a
fingerprint and a PIN. Rust + CXX-Qt + Qt 6.11 Quick + Kirigami on
[Atlas Framework](https://github.com/EternalCoder454/atlas-framework).

The design (programs, the root helper, polkit rules, the state file, recovery)
is in [docs/DESIGN.md](docs/DESIGN.md). Rules for working on it are in
[CLAUDE.md](CLAUDE.md).

## Build

Everything builds inside the dev container (`scripts/dev.sh`; the first run
builds the image, telamon-ui included, from the atlas-framework tag in
`Cargo.toml`, so it needs network).

```sh
scripts/dev.sh cargo fmt --all --check
scripts/dev.sh cargo clippy --workspace --all-targets --locked -- -D warnings
scripts/dev.sh cargo test --workspace --locked
scripts/dev.sh bash -c 'cmake -S apps/atlas-wizard -B build/dev -G Ninja && cmake --build build/dev'
scripts/dev.sh dbus-run-session -- env QT_QPA_PLATFORM=offscreen ATLAS_WIZARD_DEMO=1 build/dev/atlas-wizard
scripts/dev.sh packaging/build-rpm.sh /src/out
```

`ATLAS_WIZARD_DEMO=1` runs the GUI with no helper and no system services.
The RPM is built from the commit at HEAD; `ATLAS_RPM_WORKTREE=1` builds the
working tree instead, uncommitted edits included, for local testing.

MIT licensed.

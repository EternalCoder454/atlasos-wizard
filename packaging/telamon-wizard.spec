# Telamon Setup (telamon-wizard), the first-run wizard of Telamon OS.

# No debuginfo subpackage: the Rust flags below keep symbols (debuginfo=2,
# strip=none) and the binary is shipped as built.
%global debug_package %{nil}

Name:           telamon-wizard
Version:        0.2.2
Release:        1%{?dist}
Summary:        Telamon Setup, the first-run wizard of Telamon OS
License:        MIT
URL:            https://github.com/EternalCoder454/atlasos-wizard
# Atlas Wizard until 0.2.0: an upgrade replaces it, and what asks for the old
# package name (the image's checks) is still met.
Obsoletes:      atlas-wizard < 0.2.0
Provides:       atlas-wizard = %{version}-%{release}
Source0:        telamon-wizard-%{version}.tar.gz

BuildRequires:  cargo
BuildRequires:  rust
# %%build_rustflags
BuildRequires:  rust-srpm-macros
BuildRequires:  gcc
BuildRequires:  gcc-c++
BuildRequires:  cmake
BuildRequires:  ninja-build
BuildRequires:  corrosion
# Cargo fetches the atlas-framework crates from GitHub.
BuildRequires:  git-core
BuildRequires:  desktop-file-utils
# %%{_unitdir}, %%{_presetdir}, %%{_sysusersdir}, %%{_tmpfilesdir} and the scriptlet macros
BuildRequires:  systemd-rpm-macros
BuildRequires:  cmake(Qt6Core)
BuildRequires:  cmake(Qt6Gui)
BuildRequires:  cmake(Qt6Qml)
BuildRequires:  cmake(Qt6Quick)
BuildRequires:  cmake(Qt6QuickControls2)
BuildRequires:  cmake(Qt6Widgets)
BuildRequires:  cmake(Qt6QmlTools)
BuildRequires:  qt6-qtbase-devel
BuildRequires:  cmake(KF6CoreAddons)
BuildRequires:  cmake(KF6DBusAddons)
BuildRequires:  cmake(KF6WindowSystem)
# QML modules qmlcachegen resolves at build time (not linked). telamon-ui comes
# from atlas-framework, which is in no repository: install its RPMs first
# (build-rpm.sh does, given TELAMON_LOCAL_RPMS).
BuildRequires:  kf6-kirigami-devel
# crypt(3) hashes and password strength checks.
BuildRequires:  libxcrypt-devel
BuildRequires:  libpwquality-devel
BuildRequires:  telamon-ui >= 2.0.0

Requires:       kf6-kirigami
# Telamon.Ui, the shared look (atlas-framework); the spec's BuildRequires and src/lib.rs's ui: say the same
Requires:       telamon-ui >= 2.0.0
Requires:       kf6-qqc2-desktop-style
Requires:       qt6-qtdeclarative
# the app icon and Breeze's icons are SVG
Requires:       qt6-qtsvg
Requires:       accountsservice
Requires:       kwin
Requires:       plasma-login-manager
Requires:       polkit
Requires:       shadow-utils
Requires:       cracklib-dicts
# the helper and boot program talk to systemd, logind and the system bus
Requires:       systemd
Requires:       dbus-common
# useradd, chage, usermod and chpasswd are shadow-utils (above); nologin
# (the shell the lock step gives the setup user, which starts with /bin/sh
# for its session) and the session script's logger are util-linux; the
# fallback unit runs chvt (kbd)
Requires:       util-linux
Requires:       kbd
# The telamon-setup user comes from sysusers.d/telamon-wizard.conf: rpm creates it
# before the files are installed (rpm's sysusers generator does it; no scriptlet here).
# Atlas Wizard's atlas-setup is not made any more; a machine it set up keeps the
# locked account, and the boot program locks it if it was left open.
%{?systemd_requires}

%description
Telamon Setup runs the first time Telamon OS starts: it asks for the language,
keyboard, network, time zone and the first user, then hands over to the
desktop. Started again at the first login, it offers a fingerprint and a PIN.

%prep
%autosetup -n telamon-wizard-%{version}

%build
# NETWORK: cargo (Corrosion runs it with --locked) fetches crates.io and the
# pinned atlas-framework crates during %%build. That works in podman and with `rpmbuild`
# on a networked machine, not in an offline mock/Koji build.
# CARGO_HOME from the environment keeps a crate cache between builds
# (CLAUDE.md mounts one); otherwise a fresh one in the build dir.
export CARGO_HOME=${CARGO_HOME:-%{_builddir}/cargo-home}
# Fedora's Rust flags (hardening, build-id, ...), also used by Corrosion's cargo.
# The remaps keep build paths (panic locations, assert file names) out of the
# package, as atlas-framework's DESIGN.md asks of apps using its crates.
# HOST_CXXFLAGS reaches only the C++ that cargo's build scripts compile
# (cc-rs reads HOST_ when not cross-compiling; CMake ignores it), which
# otherwise gets no flags from here. (cc-rs then ignores a plain CXXFLAGS,
# which is only for CMake.)
# CFLAGS and CXXFLAGS are Fedora's plus the same remap for the C++ CMake
# builds (%%cmake keeps them when set). Without it the build dir, a random
# mktemp one, goes into the debug info and so into the linker's build ID,
# and two builds of one commit differ. These flags split on spaces, so
# _topdir must have none (build-rpm.sh's hasn't).
export RUSTFLAGS="%{build_rustflags} --remap-path-prefix=$PWD=. --remap-path-prefix=$CARGO_HOME=cargo"
export HOST_CXXFLAGS="-ffile-prefix-map=$PWD=. -ffile-prefix-map=$CARGO_HOME=cargo"
export CFLAGS="%{build_cflags} -ffile-prefix-map=$PWD=."
export CXXFLAGS="%{build_cxxflags} -ffile-prefix-map=$PWD=."
export CARGO_PROFILE_RELEASE_STRIP=none
# (checked with rpmspec --eval: %%cmake honours _vpath_srcdir, not __cmake_source_dir)
%global _vpath_srcdir apps/telamon-wizard
%cmake -G Ninja -DCMAKE_BUILD_TYPE=Release
%cmake_build
# The root helper and the boot program, from the same workspace and lock file.
# Never the test-root feature: no TELAMON_WIZARD_TEST_* variable is read.
export CARGO_TARGET_DIR=%{_builddir}/cargo-target
cargo build --release --locked -p wizard-helper --bin telamon-wizard-helper
cargo build --release --locked -p wizard-boot --bin telamon-wizard-boot

%install
%cmake_install
# Telamon OS needs its setup: dnf refuses to remove it. (The old file, which
# the image checks for, names both packages.)
install -Dpm0644 apps/telamon-wizard/data/dnf/protected.d/telamon-wizard.conf \
    %{buildroot}%{_sysconfdir}/dnf/protected.d/telamon-wizard.conf
install -Dpm0644 apps/telamon-wizard/data/dnf/protected.d/atlas-wizard.conf \
    %{buildroot}%{_sysconfdir}/dnf/protected.d/atlas-wizard.conf

install -Dpm0755 %{_builddir}/cargo-target/release/telamon-wizard-helper %{buildroot}%{_libexecdir}/telamon-wizard-helper
install -Dpm0755 %{_builddir}/cargo-target/release/telamon-wizard-boot %{buildroot}%{_libexecdir}/telamon-wizard-boot
install -Dpm0755 data/libexec/telamon-wizard-session %{buildroot}%{_libexecdir}/telamon-wizard-session

# What it was called until 0.2.0 (Atlas Wizard), for one release: the image's
# presets and checks, the enable links of installed systems and the first-login
# autostart name the old programs and units. Links, so there is one program and
# one unit of each.
ln -s telamon-wizard %{buildroot}%{_bindir}/atlas-wizard
for p in boot helper session; do
    ln -s telamon-wizard-$p %{buildroot}%{_libexecdir}/atlas-wizard-$p
done
install -Dpm0644 apps/telamon-wizard/data/legacy/net.eterneon.atlas.wizard.desktop \
    %{buildroot}%{_datadir}/applications/net.eterneon.atlas.wizard.desktop

for u in telamon-wizard-boot telamon-wizard-fallback telamon-wizard-helper; do
    install -Dpm0644 data/systemd/$u.service %{buildroot}%{_unitdir}/$u.service
done
# The old unit names are aliases of the new ones (a link in the unit
# directory is how systemd names an alias): the enable link of an installed
# system (multi-user.target.wants/atlas-wizard-boot.service) and a
# `systemctl enable|start atlas-wizard-boot.service` reach the new unit.
for u in boot fallback helper; do
    ln -s telamon-wizard-$u.service %{buildroot}%{_unitdir}/atlas-wizard-$u.service
done
# Only the boot unit is enabled; the helper is D-Bus activated and the
# fallback is started by `prepare` or the helper's GiveUp.
install -d %{buildroot}%{_presetdir}
echo 'enable telamon-wizard-boot.service' > %{buildroot}%{_presetdir}/50-telamon-wizard.preset
chmod 0644 %{buildroot}%{_presetdir}/50-telamon-wizard.preset

install -Dpm0644 data/dbus-1/system.d/net.eterneon.telamon.WizardHelper.conf \
    %{buildroot}%{_datadir}/dbus-1/system.d/net.eterneon.telamon.WizardHelper.conf
install -Dpm0644 data/dbus-1/system-services/net.eterneon.telamon.WizardHelper.service \
    %{buildroot}%{_datadir}/dbus-1/system-services/net.eterneon.telamon.WizardHelper.service
install -Dpm0644 data/polkit-1/actions/net.eterneon.telamon.wizard.policy \
    %{buildroot}%{_datadir}/polkit-1/actions/net.eterneon.telamon.wizard.policy
install -Dpm0644 data/polkit-1/rules.d/50-telamon-wizard.rules \
    %{buildroot}%{_datadir}/polkit-1/rules.d/50-telamon-wizard.rules
install -Dpm0644 data/sysusers.d/telamon-wizard.conf %{buildroot}%{_sysusersdir}/telamon-wizard.conf
install -Dpm0644 data/tmpfiles.d/telamon-wizard.conf %{buildroot}%{_tmpfilesdir}/telamon-wizard.conf
install -Dpm0644 data/wayland-sessions/telamon-wizard.desktop %{buildroot}%{_datadir}/wayland-sessions/telamon-wizard.desktop
install -dm0755 %{buildroot}%{_sharedstatedir}/telamon-wizard

%check
# No path into the build tree (checked as well as set: see %%build).
# grep: 0 = found, 1 = not found, anything else (no binary) fails too.
rc=0
grep -qF "%{_builddir}" %{buildroot}%{_bindir}/telamon-wizard || rc=$?
if [ "$rc" != 1 ]; then
    echo "telamon-wizard holds the build path %{_builddir} (grep status $rc)" >&2
    exit 1
fi
# The test-root feature must not be in the shipped programs.
for b in telamon-wizard-helper telamon-wizard-boot; do
    rc=0
    grep -qF TELAMON_WIZARD_TEST %{buildroot}%{_libexecdir}/$b || rc=$?
    if [ "$rc" != 1 ]; then
        echo "$b holds TELAMON_WIZARD_TEST (grep status $rc)" >&2
        exit 1
    fi
done
desktop-file-validate %{buildroot}%{_datadir}/applications/net.eterneon.telamon.wizard.desktop
desktop-file-validate %{buildroot}%{_datadir}/applications/net.eterneon.atlas.wizard.desktop
# The old names reach the same programs and units.
[ "$(readlink %{buildroot}%{_bindir}/atlas-wizard)" = telamon-wizard ]
for u in boot fallback helper; do
    [ "$(readlink %{buildroot}%{_unitdir}/atlas-wizard-$u.service)" = telamon-wizard-$u.service ]
done
for p in boot helper session; do
    [ "$(readlink %{buildroot}%{_libexecdir}/atlas-wizard-$p)" = telamon-wizard-$p ]
done
# The session file is not validated by desktop-file-validate: DesktopNames is
# a display-manager key it rejects. Check the keys the login manager needs.
for k in '^Type=Application$' '^Name=' '^Exec=/' '^DesktopNames='; do
    grep -q "$k" %{buildroot}%{_datadir}/wayland-sessions/telamon-wizard.desktop || {
        echo "wayland session file lacks $k" >&2
        exit 1
    }
done

%post
# /var/lib/telamon-wizard, /etc/telamon, /etc/atlasos and /run/telamon-setup
# now, not at next boot.
%tmpfiles_create telamon-wizard.conf
%systemd_post telamon-wizard-boot.service

%preun
%systemd_preun telamon-wizard-boot.service

%postun
%systemd_postun telamon-wizard-boot.service

%files
%license LICENSE
%{_bindir}/telamon-wizard
%{_datadir}/applications/net.eterneon.telamon.wizard.desktop
%config(noreplace) %{_sysconfdir}/dnf/protected.d/telamon-wizard.conf
%config(noreplace) %{_sysconfdir}/dnf/protected.d/atlas-wizard.conf
%{_libexecdir}/telamon-wizard-helper
%{_libexecdir}/telamon-wizard-boot
%{_libexecdir}/telamon-wizard-session
%{_unitdir}/telamon-wizard-boot.service
%{_unitdir}/telamon-wizard-fallback.service
%{_unitdir}/telamon-wizard-helper.service
%{_presetdir}/50-telamon-wizard.preset
%{_datadir}/dbus-1/system.d/net.eterneon.telamon.WizardHelper.conf
%{_datadir}/dbus-1/system-services/net.eterneon.telamon.WizardHelper.service
%{_datadir}/polkit-1/actions/net.eterneon.telamon.wizard.policy
%{_datadir}/polkit-1/rules.d/50-telamon-wizard.rules
%{_sysusersdir}/telamon-wizard.conf
%{_tmpfilesdir}/telamon-wizard.conf
%{_datadir}/wayland-sessions/telamon-wizard.desktop
%dir %{_sharedstatedir}/telamon-wizard
# Until 0.2.0's names, for one release (data/legacy, and links)
%{_bindir}/atlas-wizard
%{_libexecdir}/atlas-wizard-helper
%{_libexecdir}/atlas-wizard-boot
%{_libexecdir}/atlas-wizard-session
%{_unitdir}/atlas-wizard-boot.service
%{_unitdir}/atlas-wizard-fallback.service
%{_unitdir}/atlas-wizard-helper.service
%{_datadir}/applications/net.eterneon.atlas.wizard.desktop
# Not owned, not even as %%ghost: the done markers /etc/telamon/setup-done and
# /etc/atlasos/setup-done are the machine's, and rpm deletes a package's ghost
# files when it is removed. /etc/telamon and /etc/atlasos are made by tmpfiles.d.

%changelog
* Thu Oct 08 2026 EternalHell <77252745+EternalCoder454@users.noreply.github.com> - 0.2.2-1
- Fix: icons drawn over dialogs, popups and menus. With Qt Quick's software renderer a Kirigami.Icon was
  painted again over what sat in front of it whenever a repaint touched a part of it; the app's icons are
  layers on that renderer now.

* Wed Oct 07 2026 EternalHell <77252745+EternalCoder454@users.noreply.github.com> - 0.2.1-1
- The Light and Dark examples are drawn miniatures of the Telamon OS desktop
  that follow the accent colour, with a check mark and focus ring.
- The first row of a list (Wi-Fi, language, keyboard, time zone) can be
  chosen; it couldn't before.
- Pages are made when first shown: the first frame comes about a fifth sooner
  and idle memory is lower.
- The terminal setup turns echo off before it asks for a password.
- Smaller fixes: the welcome mark never shows blank, Finish greys out once
  pressed, one main button on Wi-Fi, pages scroll on small screens.

* Wed Oct 07 2026 EternalHell <77252745+EternalCoder454@users.noreply.github.com> - 0.2.0-1
- Renamed to Telamon Setup (telamon-wizard, net.eterneon.telamon.wizard), on
  Telamon.Ui 2.0.0; replaces atlas-wizard
- A machine Atlas Wizard set up stays set up: its done markers count, and
  /etc/telamon/setup-done is added; the state in /var/lib/atlas-wizard moves
  to /var/lib/telamon-wizard
- The setup user is telamon-setup and the session telamon-wizard; the old
  unit, program, desktop file and package names keep working for this release
- The computer name is prefilled "telamon"

* Tue Oct 06 2026 Atlas <atlas@eterneon.net> - 0.1.1-1
- Next after Account no longer goes back to Wi-Fi or Language
- The computer name is prefilled "atlasos", not "<user>-pc"

* Fri Oct 02 2026 Atlas <atlas@eterneon.net> - 0.1.0-1
- First package

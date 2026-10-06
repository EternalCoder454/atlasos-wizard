# AtlasOS Setup (atlas-wizard), the first-run wizard of AtlasOS.

# No debuginfo subpackage: the Rust flags below keep symbols (debuginfo=2,
# strip=none) and the binary is shipped as built.
%global debug_package %{nil}

Name:           atlas-wizard
Version:        0.1.1
Release:        1%{?dist}
Summary:        AtlasOS Setup, the first-run wizard of AtlasOS
License:        MIT
URL:            https://github.com/EternalCoder454/atlasos-wizard
Source0:        atlas-wizard-%{version}.tar.gz

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
# QML modules qmlcachegen resolves at build time (not linked). atlas-ui comes
# from atlas-framework, which is in no repository: install its RPMs first
# (build-rpm.sh does, given ATLAS_LOCAL_RPMS).
BuildRequires:  kf6-kirigami-devel
# crypt(3) hashes and password strength checks.
BuildRequires:  libxcrypt-devel
BuildRequires:  libpwquality-devel
BuildRequires:  atlas-ui >= 1.4.0

Requires:       kf6-kirigami
# Atlas.Ui, the shared look (atlas-framework); the spec's BuildRequires and src/lib.rs's ui: say the same
Requires:       atlas-ui >= 1.4.0
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
# The atlas-setup user comes from sysusers.d/atlas-wizard.conf: rpm creates it
# before the files are installed (rpm's sysusers generator does it; no scriptlet here).
%{?systemd_requires}

%description
AtlasOS Setup runs the first time AtlasOS starts: it asks for the language,
keyboard, network, time zone and the first user, then hands over to the
desktop. Started again at the first login, it offers a fingerprint and a PIN.

%prep
%autosetup -n atlas-wizard-%{version}

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
%global _vpath_srcdir apps/atlas-wizard
%cmake -G Ninja -DCMAKE_BUILD_TYPE=Release
%cmake_build
# The root helper and the boot program, from the same workspace and lock file.
# Never the test-root feature: no ATLAS_WIZARD_TEST_* variable is read.
export CARGO_TARGET_DIR=%{_builddir}/cargo-target
cargo build --release --locked -p wizard-helper --bin atlas-wizard-helper
cargo build --release --locked -p wizard-boot --bin atlas-wizard-boot

%install
%cmake_install
# AtlasOS needs its setup: dnf refuses to remove it.
install -Dpm0644 apps/atlas-wizard/data/dnf/protected.d/atlas-wizard.conf \
    %{buildroot}%{_sysconfdir}/dnf/protected.d/atlas-wizard.conf

install -Dpm0755 %{_builddir}/cargo-target/release/atlas-wizard-helper %{buildroot}%{_libexecdir}/atlas-wizard-helper
install -Dpm0755 %{_builddir}/cargo-target/release/atlas-wizard-boot %{buildroot}%{_libexecdir}/atlas-wizard-boot
install -Dpm0755 data/libexec/atlas-wizard-session %{buildroot}%{_libexecdir}/atlas-wizard-session

for u in atlas-wizard-boot atlas-wizard-fallback atlas-wizard-helper; do
    install -Dpm0644 data/systemd/$u.service %{buildroot}%{_unitdir}/$u.service
done
# Only the boot unit is enabled; the helper is D-Bus activated and the
# fallback is started by `prepare` or the helper's GiveUp.
install -d %{buildroot}%{_presetdir}
echo 'enable atlas-wizard-boot.service' > %{buildroot}%{_presetdir}/50-atlas-wizard.preset
chmod 0644 %{buildroot}%{_presetdir}/50-atlas-wizard.preset

install -Dpm0644 data/dbus-1/system.d/net.eterneon.atlas.WizardHelper.conf \
    %{buildroot}%{_datadir}/dbus-1/system.d/net.eterneon.atlas.WizardHelper.conf
install -Dpm0644 data/dbus-1/system-services/net.eterneon.atlas.WizardHelper.service \
    %{buildroot}%{_datadir}/dbus-1/system-services/net.eterneon.atlas.WizardHelper.service
install -Dpm0644 data/polkit-1/actions/net.eterneon.atlas.wizard.policy \
    %{buildroot}%{_datadir}/polkit-1/actions/net.eterneon.atlas.wizard.policy
install -Dpm0644 data/polkit-1/rules.d/50-atlas-wizard.rules \
    %{buildroot}%{_datadir}/polkit-1/rules.d/50-atlas-wizard.rules
install -Dpm0644 data/sysusers.d/atlas-wizard.conf %{buildroot}%{_sysusersdir}/atlas-wizard.conf
install -Dpm0644 data/tmpfiles.d/atlas-wizard.conf %{buildroot}%{_tmpfilesdir}/atlas-wizard.conf
install -Dpm0644 data/wayland-sessions/atlas-wizard.desktop %{buildroot}%{_datadir}/wayland-sessions/atlas-wizard.desktop
install -dm0755 %{buildroot}%{_sharedstatedir}/atlas-wizard

%check
# No path into the build tree (checked as well as set: see %%build).
# grep: 0 = found, 1 = not found, anything else (no binary) fails too.
rc=0
grep -qF "%{_builddir}" %{buildroot}%{_bindir}/atlas-wizard || rc=$?
if [ "$rc" != 1 ]; then
    echo "atlas-wizard holds the build path %{_builddir} (grep status $rc)" >&2
    exit 1
fi
# The test-root feature must not be in the shipped programs.
for b in atlas-wizard-helper atlas-wizard-boot; do
    rc=0
    grep -qF ATLAS_WIZARD_TEST %{buildroot}%{_libexecdir}/$b || rc=$?
    if [ "$rc" != 1 ]; then
        echo "$b holds ATLAS_WIZARD_TEST (grep status $rc)" >&2
        exit 1
    fi
done
desktop-file-validate %{buildroot}%{_datadir}/applications/net.eterneon.atlas.wizard.desktop
# The session file is not validated by desktop-file-validate: DesktopNames is
# a display-manager key it rejects. Check the keys the login manager needs.
for k in '^Type=Application$' '^Name=' '^Exec=/' '^DesktopNames='; do
    grep -q "$k" %{buildroot}%{_datadir}/wayland-sessions/atlas-wizard.desktop || {
        echo "wayland session file lacks $k" >&2
        exit 1
    }
done

%post
# /var/lib/atlas-wizard, /etc/atlasos and /run/atlas-setup now, not at next boot.
%tmpfiles_create atlas-wizard.conf
%systemd_post atlas-wizard-boot.service

%preun
%systemd_preun atlas-wizard-boot.service

%postun
%systemd_postun atlas-wizard-boot.service

%files
%license LICENSE
%{_bindir}/atlas-wizard
%{_datadir}/applications/net.eterneon.atlas.wizard.desktop
%config(noreplace) %{_sysconfdir}/dnf/protected.d/atlas-wizard.conf
%{_libexecdir}/atlas-wizard-helper
%{_libexecdir}/atlas-wizard-boot
%{_libexecdir}/atlas-wizard-session
%{_unitdir}/atlas-wizard-boot.service
%{_unitdir}/atlas-wizard-fallback.service
%{_unitdir}/atlas-wizard-helper.service
%{_presetdir}/50-atlas-wizard.preset
%{_datadir}/dbus-1/system.d/net.eterneon.atlas.WizardHelper.conf
%{_datadir}/dbus-1/system-services/net.eterneon.atlas.WizardHelper.service
%{_datadir}/polkit-1/actions/net.eterneon.atlas.wizard.policy
%{_datadir}/polkit-1/rules.d/50-atlas-wizard.rules
%{_sysusersdir}/atlas-wizard.conf
%{_tmpfilesdir}/atlas-wizard.conf
%{_datadir}/wayland-sessions/atlas-wizard.desktop
%dir %{_sharedstatedir}/atlas-wizard
# Not owned, not even as %%ghost: the done marker /etc/atlasos/setup-done is the
# machine's, and rpm deletes a package's ghost files when it is removed.
# /etc/atlasos is made by tmpfiles.d.

%changelog
* Tue Oct 06 2026 Atlas <atlas@eterneon.net> - 0.1.1-1
- Next after Account no longer goes back to Wi-Fi or Language
- The computer name is prefilled "atlasos", not "<user>-pc"

* Fri Oct 02 2026 Atlas <atlas@eterneon.net> - 0.1.0-1
- First package

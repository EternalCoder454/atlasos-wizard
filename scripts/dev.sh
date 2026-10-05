#!/bin/bash
# Run a command in the fedora:44 build container, with the repo at /src and
# the cargo and dnf caches in named podman volumes (the dnf one is this app's own).
#   scripts/dev.sh <command...>     e.g. scripts/dev.sh cargo test --workspace
#   scripts/dev.sh                  an interactive shell
# The first run installs the build dependencies from the spec (cached after).
# They include atlas-ui, which no repository has: that run needs
# ATLAS_LOCAL_RPMS=<dir> holding atlas-framework's RPMs (atlas-ui and
# atlas-symbols-fonts): the out dir of its packaging/build-rpm.sh, one version
# only. An image made before atlas-ui was needed lacks it: delete the image.
# Set CARGO_TARGET_DIR to /src/target/<name> to keep one target dir per task.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
image=localhost/atlas-wizard-dev:44

if ! podman image exists "$image"; then
    rpms=${ATLAS_LOCAL_RPMS:?the dev image needs atlas-framework RPMs: set ATLAS_LOCAL_RPMS=<dir>}
    rpms=$(cd "$rpms" && pwd)
    ctr=$(podman run -d --security-opt label=disable -v "$repo/packaging":/packaging:ro \
        -v "$rpms":/atlas-rpms:ro \
        -v atlas-wizard-dnf:/var/cache/libdnf5 \
        registry.fedoraproject.org/fedora:44 sleep infinity)
    trap 'podman rm -f "$ctr" >/dev/null' EXIT
    podman exec "$ctr" bash -c '
        echo keepcache=True >>/etc/dnf/dnf.conf
        dnf -y install dnf5-plugins rpm-build clippy rustfmt xorg-x11-server-Xvfb \
            dbus-daemon qt6-qtbase-gui kf6-qqc2-desktop-style breeze-icon-theme \
            ImageMagick xdotool \
            accountsservice python3-dbusmock polkit shadow-utils passwd util-linux systemd \
            kwin plasma-workspace libxcrypt-devel libpwquality-devel cracklib-dicts orca \
            at-spi2-core \
            /atlas-rpms/atlas-ui-[0-9]*.rpm /atlas-rpms/atlas-symbols-fonts-[0-9]*.rpm &&
        dnf -y builddep /packaging/atlas-wizard.spec' >&2
    podman commit "$ctr" "$image" >/dev/null
    podman rm -f "$ctr" >/dev/null
    trap - EXIT
fi

tty=()
[ -t 0 ] && tty=(-it)
exec podman run --rm --security-opt label=disable "${tty[@]}" \
    -v "$repo":/src -w /src \
    -v atlas-cargo:/root/.cargo/registry \
    -v atlas-cargo-git:/root/.cargo/git \
    -e CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/src/target/dev}" \
    "$image" "${@:-bash}"

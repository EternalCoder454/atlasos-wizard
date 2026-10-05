#!/bin/bash
# Build the AtlasOS Setup RPM inside a fedora:44 container, as root.
#   packaging/build-rpm.sh <out dir> [rpmbuild options]
# The binary RPM (no source, no debuginfo) is copied to <out dir>.
# Cargo needs network access.
# ATLAS_LOCAL_RPMS=<dir> installs the RPMs in <dir> first: atlas-framework's
# (atlas-ui), which the app builds against and no repository has. Optional
# when atlas-ui is installed already (the dev image: scripts/dev.sh
# packaging/build-rpm.sh /src/out). CARGO_HOME from the environment keeps the
# crate cache. The source is the commit at HEAD (git archive);
# ATLAS_RPM_WORKTREE=1 takes the working tree instead, for local testing.
set -euo pipefail

main() {
    out=${1:?usage: build-rpm.sh <out dir> [rpmbuild options]}
    shift

    here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
    src=$(dirname "$here")
    top=$(mktemp -d)
    trap 'rm -rf "$top"' EXIT
    mkdir -p "$top"/{SOURCES,BUILD,RPMS,SRPMS,SPECS}
    # The spec and the sources both come from HEAD (git archive below), so the
    # RPM is exactly that commit; ATLAS_RPM_WORKTREE=1 takes both from the
    # working tree. safe.directory: the checkout belongs to another uid in a
    # container; no fsmonitor hook from its config runs.
    spec=$top/SPECS/atlas-wizard.spec
    git=(git -c safe.directory="$src" -c core.fsmonitor=false -C "$src")
    if [ -n "${ATLAS_RPM_WORKTREE:-}" ]; then
        cp "$here/atlas-wizard.spec" "$spec"
    else
        if ! commit=$("${git[@]}" rev-parse --verify HEAD); then
            echo "build-rpm.sh: $src is not a usable git checkout (ATLAS_RPM_WORKTREE=1 builds the working tree)" >&2
            return 1
        fi
        "${git[@]}" show HEAD:packaging/atlas-wizard.spec >"$spec"
    fi
    version=$(awk '/^Version:/ {print $2; exit}' "$spec")

    dnf -y install rpm-build dnf5-plugins tar gzip >&2
    if [ -n "${ATLAS_LOCAL_RPMS:-}" ]; then
        # Atlas.Ui and its fonts, not the gallery. dnf brings their
        # dependencies; rpm then puts these exact files in place even when a
        # build of the same version is installed already.
        local_rpms=("$ATLAS_LOCAL_RPMS"/atlas-ui-[0-9]*.rpm "$ATLAS_LOCAL_RPMS"/atlas-symbols-fonts-[0-9]*.rpm)
        dnf -y install "${local_rpms[@]}" >&2
        rpm -U --replacepkgs --replacefiles "${local_rpms[@]}" >&2
    fi
    dnf -y builddep "$spec" >&2

    tarball=$top/SOURCES/atlas-wizard-$version.tar.gz
    if [ -n "${ATLAS_RPM_WORKTREE:-}" ]; then
        # Local testing only: the working tree as it is, uncommitted edits
        # included.
        echo "build-rpm.sh: building from the working tree, not a commit" >&2
        tar -C "$src" \
            --exclude=./.git --exclude=./.claude --exclude=./target --exclude=./out --exclude=./build \
            --transform "s,^\./,atlas-wizard-$version/," \
            -czf "$tarball" .
    else
        if [ -n "$("${git[@]}" status --porcelain --untracked-files=no)" ]; then
            echo "build-rpm.sh: uncommitted changes are left out; building $commit" >&2
        fi
        "${git[@]}" archive --format=tar.gz --prefix="atlas-wizard-$version/" -o "$tarball" HEAD
    fi

    rpmbuild -bb "$@" --define "_topdir $top" "$spec"

    mkdir -p "$out"
    find "$top/RPMS" -name '*.rpm' ! -name '*.src.rpm' ! -name '*debuginfo*' ! -name '*debugsource*' \
        -exec cp -v {} "$out"/ \;
}

main "$@"
exit $?

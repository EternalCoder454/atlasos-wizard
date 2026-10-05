#!/bin/bash
# check-image-secrets.sh <image>: fail if the dev image could carry a secret.
# GHCR makes an image pushed from this public repo public at once, so
# dev-image.yml runs this before it logs in and pushes. Runs with podman
# (CONTAINER_ENGINE to change it); fixes from atlasos-notepad 88fb5e7. Checks the build
# history (commands and build args), the environment, every layer as pushed
# (files a later step deleted too), and the files a secret would land in: no
# repository or .git copied in, root's home holds only the skeleton files and
# an empty .ssh, and no token- or key-shaped string in the places a build
# writes to.
set -euo pipefail
image=${1:?usage: check-image-secrets.sh <image>}
engine=${CONTAINER_ENGINE:-podman}
fail=0
bad() { echo "::error title=Secret check::$*"; fail=1; }

# Token shapes (GitHub, GitLab, npm, AWS long-term and temporary, Google API,
# Slack, private keys; keep in step with scan-layers.py); strict lengths, so the framework's redaction tests and
# fixture templates do not match.
tokens='gh[pousr]_[A-Za-z0-9]{36}|github_pat_[A-Za-z0-9_]{60,}|glpat-[A-Za-z0-9_-]{20}|npm_[A-Za-z0-9]{36}|(AKIA|ASIA)[0-9A-Z]{16}|AIza[0-9A-Za-z_-]{35}|xox[abeprs]-[0-9A-Za-z-]{10,}|xapp-[0-9A-Za-z-]{10,}|-----BEGIN [A-Z ]*PRIVATE KEY-----'
names='(TOKEN|SECRET|PASSW(OR)?D|PRIVATE|CREDENTIAL|API_?KEY|AUTH)'

# Matches are reported by kind (or the keyword alone), never printed: the log
# of this public repo is public, and GitHub masks only the secrets it knows.
# An engine error fails.
if ! history=$("$engine" history --no-trunc --format '{{.CreatedBy}}' "$image"); then
    bad "cannot read the image history"
elif grep -qE -e "$tokens" <<<"$history"; then
    bad "the image history holds a token-shaped string"
elif found=$(grep -oiE "${names}[A-Z0-9_]*=" <<<"$history" | grep -oiE "$names" | sort -u | tr '\n' ' ') && [ -n "$found" ]; then
    bad "the image history sets a secret-named variable (keyword: $found)"
fi

if ! inspect=$("$engine" image inspect "$image"); then
    bad "cannot inspect the image"
elif grep -qE -e "$tokens" <<<"$inspect"; then
    bad "the image config (env, labels, annotations) holds a token-shaped string"
elif ! env=$("$engine" image inspect --format '{{range .Config.Env}}{{println .}}{{end}}' "$image"); then
    bad "cannot read the image environment"
elif found=$(cut -d= -f1 <<<"$env" | grep -iE "$names" | tr '\n' ' ') && [ -n "$found" ]; then
    bad "the image environment has a secret-named variable: $found"
fi

# Every file of every layer as it is pushed, files a later step deleted too
# (they are still in the published layer); scan-layers.py prints the layer,
# path and kind only. Compressed payloads (RPM contents) stay unseen there;
# the RPMs' installed files are checked below.
set +e
"$engine" save --format docker-archive "$image" 2>/dev/null |
    python3 "$(dirname "$0")/scan-layers.py" >&2
st=("${PIPESTATUS[@]}")
set -e
if [ "${st[1]}" -eq 1 ]; then
    bad "an image layer holds a token-shaped string (paths above)"
elif [ "${st[1]}" -ne 0 ] || [ "${st[0]}" -ne 0 ]; then
    bad "cannot scan the image layers (save exit ${st[0]}, scan exit ${st[1]})"
fi

# The work paths may exist as empty directories (BuildKit leaves the mount
# points of RUN --mount behind); anything else there fails: a file in them, or
# a file or symlink in their place. Token shapes are looked for (binary files
# too) where a build writes, in /atlas-rpms if a build leaves it behind, and
# in every file of the atlas-* RPMs, the only packages not from Fedora, which
# must be installed. The scanner first checks its tools and its pattern, so a
# broken tool cannot pass in silence. File names only, never the matching line.
# shellcheck disable=SC2016 # expanded inside the container
if ! "$engine" run --rm --pull=never --network none --security-opt label=disable "$image" bash -c '
    rc=0
    command -v find grep xargs rpm >/dev/null || { echo "scanner tools missing"; exit 1; }
    printf "ghp_%036d\n" 0 | LC_ALL=C grep -qaE -- "$1" || { echo "scanner self-test failed"; exit 1; }
    for d in /src /workspace /github; do
        { [ -e "$d" ] || [ -L "$d" ]; } || continue
        f=$(find -H "$d" ! -type d 2>/dev/null | head -3)
        [ -n "$f" ] && { echo "files at or under $d: $f"; rc=1; }
    done
    git_dirs=$(find / -xdev -name .git -not -path "/proc/*" 2>/dev/null | head -5)
    [ -n "$git_dirs" ] && { echo "git directories: $git_dirs"; rc=1; }
    extra=$(find /root -mindepth 1 -maxdepth 1 \
        ! -name .bash_logout ! -name .bash_profile ! -name .bashrc \
        ! -name .cshrc ! -name .tcshrc ! -name .ssh ! -name .cache 2>/dev/null)
    [ -n "$extra" ] && { echo "unexpected in /root: $extra"; rc=1; }
    [ -n "$(ls -A /root/.ssh 2>/dev/null)" ] && { echo "/root/.ssh is not empty"; rc=1; }
    hits=$(LC_ALL=C grep -ralE -- "$1" /root /home /etc /opt /usr/local /tmp /var/tmp \
        /var/lib /var/log /var/cache /srv /mnt /media /atlas-rpms 2>/dev/null | head -5)
    [ -n "$hits" ] && { echo "token-shaped strings in: $hits"; rc=1; }
    if ! rpm -q atlas-ui atlas-symbols-fonts >/dev/null; then
        echo "atlas-ui or atlas-symbols-fonts is not installed"; rc=1
    fi
    if ! pkgs=$(rpm -qa --qf "%{NAME}\n" "atlas-*"); then
        echo "cannot list the atlas-* packages"; rc=1
    fi
    if [ -n "$pkgs" ]; then
        # Binaries too (-a): these are libraries and fonts.
        mapfile -t names <<<"$pkgs"
        if ! files=$(rpm -ql "${names[@]}"); then
            echo "cannot list the atlas-* package files"; rc=1
        fi
        hits=$(while IFS= read -r f; do
            [ -f "$f" ] && ! [ -L "$f" ] && printf "%s\0" "$f"
        done <<<"$files" | LC_ALL=C xargs -0r grep -alE -- "$1" 2>/dev/null | head -5)
        [ -n "$hits" ] && { echo "token-shaped strings in: $hits"; rc=1; }
    fi
    exit $rc' _ "$tokens" >&2; then
    bad "the image files hold something that must not be published (above)"
fi

[ "$fail" = 0 ] && echo "secret check passed: $image"
exit "$fail"

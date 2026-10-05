#!/bin/bash
# check-image-secrets.sh <image>: fail if the dev image could carry a secret.
# GHCR makes an image pushed from this public repo public at once, so
# dev-image.yml runs this before it logs in and pushes. Checks the build
# history (commands and build args), the environment, and the files a secret
# would land in: no repository or .git copied in, root's home holds only the
# skeleton files and an empty .ssh, and no token- or key-shaped string in the
# places a build writes to.
set -euo pipefail
image=${1:?usage: check-image-secrets.sh <image>}
fail=0
bad() { echo "::error title=Secret check::$*"; fail=1; }

# Token shapes (GitHub, AWS, Slack, private keys); strict lengths, so the
# framework's redaction tests and fixture templates do not match.
tokens='gh[pousr]_[A-Za-z0-9]{36}|github_pat_[A-Za-z0-9_]{60,}|AKIA[0-9A-Z]{16}|xox[abpr]-[0-9A-Za-z-]{10,}|-----BEGIN [A-Z ]*PRIVATE KEY-----'
names='(TOKEN|SECRET|PASSW(OR)?D|PRIVATE|CREDENTIAL|API_?KEY|AUTH)'

if podman history --no-trunc --format '{{.CreatedBy}}' "$image" | grep -E -i -e "$tokens" -e "${names}[A-Z_]*=" >&2; then
    bad "the image history holds a token or a secret-named variable (above)"
fi

if podman image inspect --format '{{range .Config.Env}}{{println .}}{{end}}' "$image" |
    cut -d= -f1 | grep -E -i "$names" >&2; then
    bad "the image environment has a secret-named variable (above)"
fi

# shellcheck disable=SC2016 # expanded inside the container
if ! podman run --rm --network none --security-opt label=disable "$image" bash -c '
    rc=0
    for d in /src /workspace /github; do
        [ -e "$d" ] && { echo "present: $d"; rc=1; }
    done
    git_dirs=$(find / -xdev -name .git -not -path "/proc/*" 2>/dev/null | head -5)
    [ -n "$git_dirs" ] && { echo "git directories: $git_dirs"; rc=1; }
    extra=$(find /root -mindepth 1 -maxdepth 1 \
        ! -name .bash_logout ! -name .bash_profile ! -name .bashrc \
        ! -name .cshrc ! -name .tcshrc ! -name .ssh ! -name .cache 2>/dev/null)
    [ -n "$extra" ] && { echo "unexpected in /root: $extra"; rc=1; }
    [ -n "$(ls -A /root/.ssh 2>/dev/null)" ] && { echo "/root/.ssh is not empty"; rc=1; }
    hits=$(grep -rIlE -- "$1" /root /home /etc /opt /usr/local /tmp /var/tmp /atlas-rpms 2>/dev/null | head -5)
    [ -n "$hits" ] && { echo "token-shaped strings in: $hits"; rc=1; }
    exit $rc' _ "$tokens" >&2; then
    bad "the image files hold something that must not be published (above)"
fi

[ "$fail" = 0 ] && echo "secret check passed: $image"
exit "$fail"

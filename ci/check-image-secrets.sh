#!/bin/bash
# check-image-secrets.sh <image>: fail if the dev image could carry a secret.
# GHCR makes an image pushed from this public repo public at once, so
# dev-image.yml runs this before it logs in and pushes. Runs with podman
# (CONTAINER_ENGINE to change it); fixes from atlasos-notepad 88fb5e7. Checks the build
# history (commands and build args), the environment, every layer as pushed
# (files a later step deleted too), and the files a secret would land in: no
# repository or .git copied in, no credential file anywhere, root's home holds
# only the skeleton files and an empty .ssh, and no token- or key-shaped
# string in the places a build writes to.
#
# Not detected (known limits): secrets with no fixed shape (an AWS secret
# access key alone, a password in a URL or a config line), token shapes too
# generic to tell from Fedora's own data (Twilio, Discord, Telegram, JWTs), an
# AWS key ID glued to capitals or digits, and anything compressed or in
# UTF-16 (RPM payloads, archives).
set -euo pipefail
image=${1:?usage: check-image-secrets.sh <image>}
engine=${CONTAINER_ENGINE:-podman}
here=$(dirname "$0")
fail=0
bad() { echo "::error title=Secret check::$*"; fail=1; }

# Token shapes (GitHub, GitLab, npm, PyPI, Docker Hub, Hugging Face,
# Anthropic/OpenAI, Stripe, SendGrid, Vault, DigitalOcean, Shopify, age,
# Google API and OAuth, Slack, AWS long-term and temporary, registry auth,
# private keys). Keep in step with scan-layers.py, which has the same list
# plus base64-wrapped keys. Strict lengths, so the framework's redaction
# tests and fixture templates do not match; a key only with its encoded body
# after the header (on the same line, or across real or escaped line breaks:
# \n, \\n, \u000a in JSON), and an
# AWS key ID only standing alone, because Fedora files carry bare key headers
# (ImageMagick's mime.xml) and long capital runs. Every grep uses -z, so a
# key's lines are one record.
tokens='gh[pousr]_[A-Za-z0-9]{36}|github_pat_[A-Za-z0-9_]{60,}|glpat-[A-Za-z0-9_-]{20}|npm_[A-Za-z0-9]{36}|pypi-AgEI[A-Za-z0-9_-]{50,}|dckr_pat_[A-Za-z0-9_-]{27,}|hf_[A-Za-z0-9]{34}|sk-ant-[A-Za-z0-9_-]{32,}|sk-(proj|svcacct|admin)-[A-Za-z0-9_-]{40,}|sk-[A-Za-z0-9]{48}|[sr]k_live_[0-9A-Za-z]{24,}|SG\.[A-Za-z0-9_-]{22}\.[A-Za-z0-9_-]{43}|hv[sb]\.[A-Za-z0-9_-]{24,}|do[por]_v1_[a-f0-9]{64}|shp(at|ca|pa|ss)_[a-fA-F0-9]{32}|AGE-SECRET-KEY-1[0-9A-Z]{58}|AIza[0-9A-Za-z_-]{35}|ya29\.[0-9A-Za-z_-]{30,}|xox[abeprs]-[0-9A-Za-z-]{10,}|xapp-[0-9A-Za-z-]{10,}|hooks\.slack\.com/services/T[A-Z0-9]{8,}/B[A-Z0-9]{8,}/[A-Za-z0-9]{24}|(^|[^A-Z0-9])(AKIA|ASIA)[0-9A-Z]{16}([^A-Z0-9]|$)|"(auth|identitytoken)"[[:space:]]*:[[:space:]]*"[A-Za-z0-9+/=._-]{16,}"|-----BEGIN [A-Z ]{0,20}PRIVATE KEY( BLOCK)?-----([[:space:]]|(\\)+[nr]|(\\)+u000[aAdD]){0,64}([A-Za-z-]{1,40}: [^\\[:cntrl:]]{0,200}([[:space:]]|(\\)+[nr]|(\\)+u000[aAdD]){1,64}){0,8}[A-Za-z0-9+/]{20}(([[:space:]]|(\\)+[nr]|(\\)+u000[aAdD]){0,64}[A-Za-z0-9+/]{4}){5}'
names='(TOKEN|SECRET|PASSW(OR)?D|PRIVATE|CREDENTIAL|API_?KEY|AUTH)'

# has_token <<<text: true for a match, and for a grep that failed (which is
# reported), so an error never passes as clean.
has_token() {
    local s=0
    LC_ALL=C grep -qazE -e "$tokens" || s=$?
    if [ "$s" -ge 2 ]; then
        bad "the token search failed (grep exit $s)"
        return 0
    fi
    return "$s"
}

# keywords <match regex> <<<text: the secret keywords in what the regex
# matches, space-separated (empty for none); fails on a grep error.
keywords() {
    local s=0 m
    m=$(LC_ALL=C grep -oiaE -e "$1") || s=$?
    [ "$s" -le 1 ] || return 1
    [ -n "$m" ] || return 0
    LC_ALL=C grep -oiE -e "${names%)}|KEY)" <<<"$m" | sort -u | tr '\n' ' '
}

# The patterns themselves: shapes that must match, and Fedora's bare header
# that must not.
body=$(printf 'A%.0s' {1..64})
for t in "$(printf 'ghp_%036d' 0)" \
    "$(printf -- '-----BEGIN PRIVATE KEY-----\n%s\n' "$body")" \
    "$(printf -- '  -----BEGIN RSA PRIVATE KEY-----  \n  Proc-Type: 4,ENCRYPTED\n\n  %s\n' "$body")" \
    "{\"private_key\": \"-----BEGIN PRIVATE KEY-----\\n$body\\n\"}" \
    "{\"labels\": {\"k\": \"-----BEGIN PRIVATE KEY-----\\\\n$body\"}}" \
    "KEY=-----BEGIN RSA PRIVATE KEY-----$body-----END RSA PRIVATE KEY-----" \
    "x AKIAABCDEFGHIJKLMNOP y"; do
    has_token <<<"$t" || bad "the token patterns failed their self-test"
done
! has_token <<<'<match value="-----BEGIN PGP PRIVATE KEY BLOCK-----"/>' ||
    bad "the token patterns matched a bare key header in their self-test"

# Matches are reported by kind (or the keyword alone), never printed: the log
# of this public repo is public, and GitHub masks only the secrets it knows.
# An engine error fails.
if ! history=$("$engine" history --no-trunc --format '{{.CreatedBy}}' "$image"); then
    bad "cannot read the image history"
elif has_token <<<"$history"; then
    bad "the image history holds a token-shaped string"
elif ! found=$(keywords "${names}[A-Z0-9_]*=" <<<"$history"); then
    bad "the search for secret-named variables in the history failed"
elif [ -n "$found" ]; then
    bad "the image history sets a secret-named variable (keyword: $found)"
fi

# The inspect JSON escapes line breaks (\n), the environment as printed has
# them real; both are searched.
if ! inspect=$("$engine" image inspect "$image"); then
    bad "cannot inspect the image"
elif has_token <<<"$inspect"; then
    bad "the image config (env, labels, annotations) holds a token-shaped string"
elif ! env=$("$engine" image inspect --format '{{range .Config.Env}}{{println .}}{{end}}' "$image"); then
    bad "cannot read the image environment"
elif has_token <<<"$env"; then
    bad "the image environment holds a token-shaped string"
elif ! found=$(cut -d= -f1 <<<"$env" | keywords "${names%)}|KEY)"); then
    bad "the search for secret-named variables in the environment failed"
elif [ -n "$found" ]; then
    bad "the image environment has a secret-named variable (keyword: $found)"
fi

# The one known key in Fedora's files (gnutls' FIPS self-test keys) passes the
# layer scan only in the file the RPM database names, byte for byte.
allow=()
if digests=$("$engine" run --rm --pull=never --network none --security-opt label=disable "$image" \
    rpm -q --qf '[%{FILEDIGESTS} %{FILENAMES}\n]' gnutls 2>/dev/null); then
    while read -r digest path; do
        if [[ $path =~ ^/usr/lib64/libgnutls\.so\.[0-9.]+$ && $digest =~ ^[0-9a-f]{64}$ ]]; then
            allow+=(--allow-sha256 "$digest")
        fi
    done <<<"$digests"
fi

# Every file of every layer as it is pushed, files a later step deleted too
# (they are still in the published layer); scan-layers.py prints the layer,
# path and kind only. Compressed payloads (RPM contents) stay unseen there;
# the RPMs' installed files are checked below.
set +e
"$engine" save --format docker-archive "$image" 2>/dev/null |
    python3 -I -B "$here/scan-layers.py" "${allow[@]}" >&2
st=("${PIPESTATUS[@]}")
set -e
if [ "${st[1]}" -eq 1 ]; then
    bad "an image layer holds a token-shaped string (paths above)"
elif [ "${st[1]}" -ne 0 ] || [ "${st[0]}" -ne 0 ]; then
    bad "cannot scan the image layers (save exit ${st[0]}, scan exit ${st[1]})"
fi

# The work paths may exist as empty directories (BuildKit leaves the mount
# points of RUN --mount behind); anything else there fails: a file in them, or
# a file or symlink in their place. Credential files fail by name anywhere.
# Token shapes are looked for (binary files too) where a build writes, in
# /telamon-rpms if a build leaves it behind, and in every file of the telamon-*
# RPMs, the only packages not from Fedora, which must be installed. The
# scanner first checks its tools and its pattern, so a broken tool cannot pass
# in silence, and a grep error fails. File names only, never the matching
# line; the output is made safe for the log on the way out (no control
# characters, nothing that starts a workflow command).
set +e
# shellcheck disable=SC2016 # expanded inside the container
"$engine" run --rm --pull=never --network none --security-opt label=disable "$image" bash -c '
    tokens=$1
    rc=0
    for t in find grep rpm; do
        command -v "$t" >/dev/null || { echo "scanner tool missing: $t"; exit 1; }
    done
    printf "ghp_%036d\n" 0 | LC_ALL=C grep -qazE -e "$tokens" || { echo "scanner self-test failed"; exit 1; }
    # outside every searched path
    errs=$(mktemp -p /run) || { echo "scanner: no temporary file"; exit 1; }
    # found <what> <grep option> <path>...: report files holding a token
    found() {
        local what=$1 opt=$2 s=0 hits
        shift 2
        hits=$(LC_ALL=C grep -lzE "$opt" -e "$tokens" -- "$@" 2>"$errs") || s=$?
        [ "$s" -ge 2 ] && { echo "$what: the search failed: $(head -3 "$errs")"; rc=1; }
        [ -n "$hits" ] && { echo "$what: token-shaped strings in: $(head -5 <<<"$hits")"; rc=1; }
    }
    for d in /src /workspace /github; do
        { [ -e "$d" ] || [ -L "$d" ]; } || continue
        f=$(find -H "$d" ! -type d 2>/dev/null | head -3)
        [ -n "$f" ] && { echo "files at or under $d: $f"; rc=1; }
    done
    git_dirs=$(find / -xdev -name .git -not -path "/proc/*" 2>/dev/null | head -5)
    [ -n "$git_dirs" ] && { echo "git directories: $git_dirs"; rc=1; }
    creds=$(find / -xdev \( -name .git-credentials -o -name .netrc -o -name _netrc \
        -o -name .pypirc -o -name .npmrc -o -name .yarnrc.yml -o -name .dockercfg \
        -o -path "*/.docker/config.json" -o -path "*/containers/auth.json" \
        -o -path "*/.aws/credentials" -o -path "*/.config/gh/hosts.yml" \
        -o -path "*/.cargo/credentials*" -o -path "*/.kube/config" \) \
        -not -path "/proc/*" 2>/dev/null | head -5)
    [ -n "$creds" ] && { echo "credential files: $creds"; rc=1; }
    extra=$(find /root -mindepth 1 -maxdepth 1 \
        ! -name .bash_logout ! -name .bash_profile ! -name .bashrc \
        ! -name .cshrc ! -name .tcshrc ! -name .ssh ! -name .cache 2>/dev/null)
    [ -n "$extra" ] && { echo "unexpected in /root: $extra"; rc=1; }
    [ -n "$(ls -A /root/.ssh 2>/dev/null)" ] && { echo "/root/.ssh is not empty"; rc=1; }
    dirs=()
    for d in /root /home /etc /opt /usr/local /tmp /var/tmp /var/lib /var/log \
        /var/cache /srv /mnt /media /telamon-rpms; do
        [ -e "$d" ] && dirs+=("$d")
    done
    found "build paths" -ra "${dirs[@]}"
    if ! rpm -q telamon-ui telamon-symbols-fonts >/dev/null; then
        echo "telamon-ui or telamon-symbols-fonts is not installed"; rc=1
    fi
    if ! pkgs=$(rpm -qa --qf "%{NAME}\n" "telamon-*"); then
        echo "cannot list the telamon-* packages"; rc=1
    fi
    if [ -n "$pkgs" ]; then
        mapfile -t names <<<"$pkgs"
        if ! files=$(rpm -ql "${names[@]}"); then
            echo "cannot list the telamon-* package files"; rc=1
        fi
        regular=()
        while IFS= read -r f; do
            [ -f "$f" ] && ! [ -L "$f" ] && regular+=("$f")
        done <<<"$files"
        # Binaries too (-a): these are libraries and fonts.
        [ "${#regular[@]}" -gt 0 ] && found "telamon-* package files" -a "${regular[@]}"
    fi
    exit $rc' _ "$tokens" 2>&1 |
    LC_ALL=C sed -e 's/[^[:print:]]/?/g' -e 's/::/: :/g' -e 's/##\[/# #[/g' >&2
st=("${PIPESTATUS[@]}")
set -e
if [ "${st[0]}" -ne 0 ] || [ "${st[1]}" -ne 0 ]; then
    bad "the image files hold something that must not be published (above)"
fi

[ "$fail" = 0 ] && echo "secret check passed: $image"
exit "$fail"

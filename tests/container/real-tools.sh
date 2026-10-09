#!/usr/bin/env bash
# The real shadow-utils against wizard-core, in a throw-away root.
#
# What the fallback runs (`useradd -m -U -G wheel -c <full name> -- <user>`,
# `chpasswd -e` with a hash from wizard-core, `userdel -r`, and the lock
# commands) is run for real, with the shadow-utils `-P PREFIX` option so that
# only files below a directory made here are touched, and the result is
# checked: passwd, shadow and group, the yescrypt hash, wheel, the uid range,
# the home's mode (before and after wizard-core's `secure_home`), that `--`
# stops option parsing for names such as `-x` and `--root=/`, that hostile
# comments are refused, and that everything wizard-core accepts as a user name
# or full name, useradd accepts and stores unchanged.
#
# Run it ONLY in an unprivileged container (it refuses to run anywhere else),
# from the repository root:
#
#   export TMPDIR=$HOME/.cache/podman-tmp
#   podman run --rm --ulimit core=0 --security-opt label=disable \
#       --security-opt no-new-privileges -v "$PWD":/src -w /src \
#       -e CARGO_TARGET_DIR=/src/target/container \
#       localhost/telamon-wizard-dev:44 tests/container/real-tools.sh
#
# Exit 0 when every check passes, or when the tools cannot run here (it says
# SKIP and why); non-zero on a failed check. TELAMON_REAL_TOOLS_HELPER names a
# built `real-tools-helper` (`cargo build --example real-tools-helper -p
# wizard-core`) to skip the build.
set -euo pipefail

skip() {
    echo "SKIP: $*"
    exit 0
}

# Never on a host: this runs useradd, chpasswd, userdel and chage as root.
[ "$(id -u)" = 0 ] || skip "not root (it must run as root inside a container)"
[ -e /run/.containerenv ] || [ -e /.dockerenv ] || skip "not inside a container; it refuses to run useradd outside one"
for t in useradd userdel chpasswd chage usermod awk; do
    command -v "$t" >/dev/null || skip "$t is not installed"
done
useradd --help 2>&1 | grep -q -- '--prefix' || skip "this useradd has no --prefix"

helper=${TELAMON_REAL_TOOLS_HELPER:-}
if [ -z "$helper" ]; then
    command -v cargo >/dev/null || skip "no cargo to build the helper (set TELAMON_REAL_TOOLS_HELPER)"
    cargo build --quiet --locked -p wizard-core --example real-tools-helper
    helper=${CARGO_TARGET_DIR:-target}/debug/examples/real-tools-helper
fi
[ -x "$helper" ] || skip "no helper at $helper"

work=$(mktemp -d /tmp/telamon-real-tools.XXXXXX)
trap 'rm -rf "$work"' EXIT
checks=0
fail() {
    echo "FAIL: $*" >&2
    exit 1
}
ok() { checks=$((checks + 1)); }
eq() { # eq WHAT WANT GOT
    [ "$2" = "$3" ] || fail "$1: wanted [$2], got [$3]"
    ok
}

# A fresh root: root, the setup user, a wheel group, a skeleton, and this
# image's login.defs with HOME_MODE kept ("with") or taken out ("without": the
# home then gets 0755 from UMASK 022, as on a system that lacks HOME_MODE).
new_root() {
    r=$(mktemp -d "$work/root.XXXXXX")
    mkdir -p "$r/etc/skel" "$r/home" "$r/var/spool/mail"
    case ${1:-with} in
    without) grep -v '^HOME_MODE' /etc/login.defs >"$r/etc/login.defs" ;;
    *) cp /etc/login.defs "$r/etc/login.defs" ;;
    esac
    printf 'root:x:0:0:root:/root:/bin/bash\ntelamon-setup:x:970:970:Telamon Setup:/run/telamon-setup:/bin/sh\n' >"$r/etc/passwd"
    printf 'root:!:19000::::::\ntelamon-setup:!*:19000::::::\n' >"$r/etc/shadow"
    printf 'root:x:0:\nwheel:x:10:\ntelamon-setup:x:970:\n' >"$r/etc/group"
    printf 'root:::\nwheel:::\ntelamon-setup:::\n' >"$r/etc/gshadow"
    printf '# skeleton\n' >"$r/etc/skel/.bashrc"
    echo "$r"
}
passwd_field() { # passwd_field ROOT NAME N
    awk -F: -v n="$2" -v f="$3" '$1 == n { print $f }' "$1/etc/passwd"
}
shadow_field() {
    awk -F: -v n="$2" -v f="$3" '$1 == n { print $f }' "$1/etc/shadow"
}
mode_of() { stat -c '%a' "$1"; }
sum_files() { (cd "$1" && find etc -type f | sort | xargs sha256sum; find home -mindepth 1 | sort); }

PW='CANARY-real-tools-Plum-Orbit-4711'
hash=$(printf '%s\n' "$PW" | "$helper" hash)
# shellcheck disable=SC2016 # the single quotes keep the dollar signs of the prefix
case $hash in '$y$'*) ok ;; *) fail "wizard-core's hash is not yescrypt: $hash" ;; esac

# ---- the happy path: what the fallback does, in order -----------------------
for variant in with without; do
r=$(new_root "$variant")
useradd -P "$r" -m -U -G wheel -c 'Ada Lovelace' -- ada 2>/dev/null
uid=$(passwd_field "$r" ada 3)
eq "uid in the human range" yes "$([ "$uid" -ge 1000 ] && [ "$uid" -le 60000 ] && echo yes || echo no)"
eq "passwd has seven fields" 7 "$(awk -F: '$1 == "ada" { print NF }' "$r/etc/passwd")"
eq "the comment is stored unchanged" 'Ada Lovelace' "$(passwd_field "$r" ada 5)"
eq "the shadow entry is locked before the password is set" '!' "$(shadow_field "$r" ada 2)"
eq "wheel has the user" ada "$(awk -F: '$1 == "wheel" { print $4 }' "$r/etc/group")"
eq "the user's own group exists, alone" "ada:x:$uid:" "$(grep '^ada:' "$r/etc/group")"
eq "no other group lists the user" 1 "$(grep -c '[:,]ada\(,\|$\)' "$r/etc/group")"
eq "the setup user is untouched" 'telamon-setup:x:970:970:Telamon Setup:/run/telamon-setup:/bin/sh' "$(grep '^telamon-setup:' "$r/etc/passwd")"

printf 'ada:%s\n' "$hash" | chpasswd -P "$r" -e
eq "shadow holds the exact hash" "$hash" "$(shadow_field "$r" ada 2)"
# shellcheck disable=SC2016 # the dollar signs are the hash prefix
case $(shadow_field "$r" ada 2) in '$y$'*) ok ;; *) fail "shadow has no yescrypt hash" ;; esac
eq "the hash is accepted by wizard-core's own reading" "ada $uid" "$("$helper" humans "$r" | grep '^ada ')"
if grep -rqF -e "$PW" "$r"; then fail "the password is on disk under the root"; fi
ok

if [ "$variant" = with ]; then
    eq "HOME_MODE 0700 gives a private home" 700 "$(mode_of "$r/home/ada")"
else
    eq "without HOME_MODE useradd leaves the home open (0755)" 755 "$(mode_of "$r/home/ada")"
    eq "wizard-core's secure_home reports a change" changed "$("$helper" secure-home "$r" ada "$uid")"
fi
chmod 0755 "$r/home/ada"
eq "wizard-core's secure_home closes it again" changed "$("$helper" secure-home "$r" ada "$uid")"
eq "the home is private" 700 "$(mode_of "$r/home/ada")"
eq "and a second run changes nothing" unchanged "$("$helper" secure-home "$r" ada "$uid")"
chmod 0777 "$r/home/ada"
out=$("$helper" verify "$r" ada "$uid" || true)
eq "verify refuses a home others can write" verify-home-writable "$out"
"$helper" secure-home "$r" ada "$uid" >/dev/null
"$helper" verify "$r" ada "$uid" || fail "verify refuses the finished account"
ok
if "$helper" verify "$r" ada $((uid + 1)) >/dev/null; then fail "verify accepted another uid"; fi
ok

# the lock the helper and `prepare` apply to the setup user
chage -P "$r" -E 0 telamon-setup
usermod -P "$r" -s /usr/sbin/nologin telamon-setup
eq "setup user expired" 0 "$(shadow_field "$r" telamon-setup 8)"
eq "setup user shell" /usr/sbin/nologin "$(passwd_field "$r" telamon-setup 7)"
eq "setup user has no human account" ada "$("$helper" humans "$r" | awk '{ print $1 }')"

# removing the half-made account
userdel -r -P "$r" -- ada 2>/dev/null
eq "userdel removed passwd" "" "$(passwd_field "$r" ada 1)"
eq "userdel removed shadow" "" "$(shadow_field "$r" ada 1)"
eq "userdel removed the group" 0 "$(grep -c '^ada:' "$r/etc/group")"
eq "userdel took the user out of wheel" "" "$(awk -F: '$1 == "wheel" { print $4 }' "$r/etc/group")"
eq "userdel removed the home" "" "$(ls "$r/home")"
done

# ---- option parsing: `--` ends the options ---------------------------------
r=$(new_root)
before=$(sum_files "$r")
for bad in -x --root=/ --help -r -P /; do
    if useradd -P "$r" -m -U -G wheel -c 'x' -- "$bad" 2>/dev/null; then fail "useradd took the name $bad"; fi
    ok
    if "$helper" user-name "$bad" 2>/dev/null; then fail "wizard-core accepted the name $bad"; fi
    ok
done
eq "nothing changed" "$before" "$(sum_files "$r")"
# a comment that looks like an option is a comment (it is the value of -c)
useradd -P "$r" -m -U -G wheel -c '-rf /' -- eve 2>/dev/null
eq "an option-like comment is stored as text" '-rf /' "$(passwd_field "$r" eve 5)"
eq "and there are still seven fields" 7 "$(awk -F: '$1 == "eve" { print NF }' "$r/etc/passwd")"

# ---- hostile comments: useradd refuses what the validator also refuses ------
for bad in 'a:b' $'a\nb' $'a\nroot:x:0:0::/root:/bin/bash'; do
    if useradd -P "$r" -c "$bad" -- mal 2>/dev/null; then fail "useradd took the comment [$bad]"; fi
    ok
    if "$helper" full-name "$bad" >/dev/null 2>&1; then fail "wizard-core accepted the comment [$bad]"; fi
    ok
done
eq "no stray account" "" "$(passwd_field "$r" mal 1)"
eq "root is still alone at uid 0" 1 "$(awk -F: '$3 == 0' "$r/etc/passwd" | wc -l)"

# ---- the validators against the real tool ----------------------------------
# everything wizard-core accepts, useradd accepts and stores as given
i=0
accepted=0
# shellcheck disable=SC2016 # the candidates hold shell metacharacters on purpose
while IFS= read -r -d '' text; do
    i=$((i + 1))
    if shown=$("$helper" full-name "$text" 2>/dev/null); then
        accepted=$((accepted + 1))
        name="u$i"
        useradd -P "$r" -m -U -G wheel -c "$shown" -- "$name" 2>/dev/null || fail "useradd refused the full name [$text] that wizard-core accepts"
        eq "stored unchanged [$text]" "$shown" "$(passwd_field "$r" "$name" 5)"
        eq "seven fields [$text]" 7 "$(awk -F: -v n="$name" '$1 == n { print NF }' "$r/etc/passwd")"
        userdel -r -P "$r" -- "$name" 2>/dev/null
    fi
done < <(printf '%s\0' \
    'Ada Lovelace' '  padded  ' $'Zo\xc3\xab \xc3\x9cnal' $'\xe5\xbc\xa0\xe4\xbc\x9f' 'O'\''Brien' '"quoted"' 'a b  c' '-rf /' '--root=/' \
    'x;y|z&w' '$(id)' '`id`' 'a\b' 'a/b' $'\xc3\x89mile (the "great")' $'\xc3\x9cn\xc3\xafc\xc3\xb6d\xc3\xa9 N\xc3\xa4m\xc3\xa9' \
    "$(printf '\xc3\xa9%.0s' $(seq 1 127))" "$(printf 'a%.0s' $(seq 1 255))" \
    $'ZWNJ\xe2\x80\x8cjoiner' 'tab	inside' 'a,b' 'a=b' 'a:b')
[ "$accepted" -ge 10 ] || fail "only $accepted of the candidate comments were accepted: the list is broken"
ok

for name in ada _a a-b a_b a1 "$(printf 'a%.0s' $(seq 1 32))" _ z9; do
    "$helper" user-name "$name" || fail "wizard-core refuses the plain name $name"
    useradd -P "$r" -m -U -G wheel -c 'T' -- "$name" 2>/dev/null || fail "useradd refuses the name $name that wizard-core accepts"
    eq "name stored [$name]" "$name" "$(passwd_field "$r" "$name" 1)"
    userdel -r -P "$r" -- "$name" 2>/dev/null
done

# ---- no_new_privs: the units leave NoNewPrivileges off for SELinux's sake ---
# (DESIGN.md), not because the tools need setuid or capabilities gained on
# exec: with the flag set every command above still works here. What SELinux
# does to domain transitions under it is the VM's to say.
if command -v setpriv >/dev/null; then
    r=$(new_root with)
    nnp() { setpriv --no-new-privs "$@"; }
    nnp useradd -P "$r" -m -U -G wheel -c 'Nnp Test' -- nnp 2>/dev/null
    printf 'nnp:%s\n' "$hash" | nnp chpasswd -P "$r" -e
    nnp chage -P "$r" -E 0 telamon-setup
    nnp usermod -P "$r" -s /usr/sbin/nologin telamon-setup
    eq "chpasswd under no_new_privs" "$hash" "$(shadow_field "$r" nnp 2)"
    eq "chage under no_new_privs" 0 "$(shadow_field "$r" telamon-setup 8)"
    nnp userdel -r -P "$r" -- nnp 2>/dev/null
    eq "userdel under no_new_privs" "" "$(passwd_field "$r" nnp 1)"
fi

echo "OK: $checks checks against the real useradd, chpasswd, chage, usermod and userdel"

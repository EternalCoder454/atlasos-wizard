#!/bin/bash
# Every page and state that matters, in the light and dark scheme, at the
# given scales, in parallel. Inside the dev container:
#   scripts/dev.sh scripts/screenshots.sh OUTDIR [SCALES...]   (default: 1 1.7 2)
# One PNG per state: OUTDIR/<name>-<scheme>-<scale>x.png. Set ONLY=<regex> to
# take the states whose name matches. Uses scripts/screenshot.sh.
set -uo pipefail
outdir=${1:?usage: screenshots.sh OUTDIR [SCALES...]}
shift
scales=${*:-1 1.7 2}
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

# name|step|actions|answers|env (see screenshot-states.txt)
states=$(cat "$here/screenshot-states.txt")
jobs=()
while IFS='|' read -r name step actions answers extra; do
    [ -z "$name" ] && continue
    [[ "$name" == \#* ]] && continue
    [ -n "${ONLY:-}" ] && ! [[ "$name" =~ $ONLY ]] && continue
    for scheme in light dark; do
        for scale in $scales; do
            jobs+=("$name|$step|$scheme|$scale|$actions|$answers|$extra")
        done
    done
done <<<"$states"

run() {
    IFS='|' read -r name step scheme scale actions answers extra <<<"$1"
    f=$2/$name-$scheme-${scale}x.png
    read -ra envs <<<"$extra"
    for _ in 1 2 3; do
        env "${envs[@]}" "$3/screenshot.sh" "$f" "$step" "$scheme" "$scale" "$actions" "$answers" >/dev/null 2>"$f.log" && [ -s "$f" ] && { rm -f "$f.log"; return; }
        sleep 1
    done
    echo "FAILED $f" >&2
}
export -f run
mkdir -p "$outdir"
printf '%s\n' "${jobs[@]}" | xargs -d '\n' -P "${JOBS:-8}" -I{} bash -c 'run "$@"' _ {} "$outdir" "$here"
echo "${#jobs[@]} shots in $outdir"

#!/bin/bash
# Startup, memory and CPU of the setup in demo mode, headless, inside the dev
# container (llvmpipe, so absolute numbers are only comparable run to run):
#   scripts/dev.sh scripts/bench.sh [RUNS]      (default 7; prints the medians)
# Per run: time from launch to the first painted frame and to the first page
# (the card), RSS and PSS once the first page is up and settled, CPU time used
# in 10 s of idling on the first page, CPU time of clicking through every
# page (it includes making the pages not made yet), the peak RSS after that,
# and the CPU time from launch to the settled first page. The demo's own 300 ms
# "startup" nap is in "first page".
set -euo pipefail
runs=${1:-7}
bin=${TELAMON_WIZARD_BIN:-build/dev/telamon-wizard}
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT

one() {
    local i=$1
    local tmp=$out/run$i
    mkdir -p "$tmp"/{config,data,cache,runtime}
    chmod 700 "$tmp/runtime"
    cp "$here/schemes/TelamonLight.colors" "$tmp/config/kdeglobals"
    printf '\n[General]\nfont=IBM Plex Sans,10,-1,5,400,0,0,0,0,0,0,0,0,0,0,1\n[Icons]\nTheme=breeze\n' >>"$tmp/config/kdeglobals"
    printf '{"step":"%s","language":"en_US.UTF-8","keyboardLayout":"us","timezone":"Europe/Berlin","hostname":"telamon"}\n' "${BENCH_STEP:-welcome}" >"$tmp/answers.json"
    cat >"$tmp/bus.conf" <<'EOF2'
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig><type>session</type><listen>unix:tmpdir=/tmp</listen><auth>EXTERNAL</auth>
<policy context="default"><allow send_destination="*" eavesdrop="true"/><allow eavesdrop="true"/><allow own="*"/></policy></busconfig>
EOF2
    cat >"$tmp/run.sh" <<INNER
#!/bin/bash
set -e
px() { import -window root -crop 1x1+960+300 -depth 8 txt:- 2>/dev/null | tail -1 | grep -o '#[0-9A-F]*' | head -1; }
bg=\$(px)
t0=\$(date +%s%N)
"$bin" >/dev/null 2>&1 &
app=\$!
first=; page=; prev=\$bg
while :; do
    c=\$(px)
    now=\$(date +%s%N)
    if [ -z "\$first" ]; then
        if [ "\$c" != "\$bg" ]; then first=\$(( (now - t0) / 1000000 )); prev=\$c; fi
    # The card's own colour is not the window's: a second change.
    elif [ "\$c" != "\$prev" ]; then
        page=\$(( (now - t0) / 1000000 ))
    fi
    [ -n "\$page" ] && break
    [ \$(( (now - t0) / 1000000 )) -gt 20000 ] && break
done
[ -n "\$page" ] || page=\$first
sleep 2
rss=\$(awk '/^Rss:/{print \$2}' /proc/\$app/smaps_rollup)
pss=\$(awk '/^Pss:/{print \$2}' /proc/\$app/smaps_rollup)
cpu() { awk '{print \$14+\$15}' /proc/\$app/stat; }
start=\$(( \$(cpu) * 10 ))
c0=\$(cpu); sleep 10; c1=\$(cpu)
idle=\$(( (c1 - c0) * 10 ))
c0=\$(cpu); w0=\$(date +%s%N)
for n in 1 2 3 4 5 6 7 8 9; do
    xdotool mousemove \$((960+346)) \$((540+283)) click 1
    sleep 1
    [ -n "\${BENCH_TRACE:-}" ] && echo "click \$n cpu \$(( (\$(cpu) - c0) * 10 ))" >>"$tmp/trace"
done
w1=\$(date +%s%N); c1=\$(cpu)
walk=\$(( (c1 - c0) * 10 ))
hwm=\$(awk '/^VmHWM:/{print \$2}' /proc/\$app/status)
echo "\$first \$page \$rss \$pss \$idle \$walk \$hwm \$start" >"$tmp/result"
[ -f "$tmp/trace" ] && cp "$tmp/trace" /src/out/trace.$$ || true
kill \$app 2>/dev/null || true
wait \$app 2>/dev/null || true
INNER
    chmod +x "$tmp/run.sh"
    env XDG_CONFIG_HOME="$tmp/config" XDG_DATA_HOME="$tmp/data" XDG_CACHE_HOME="$tmp/cache" \
        XDG_RUNTIME_DIR="$tmp/runtime" LANG=C.UTF-8 LC_ALL=C.UTF-8 QT_QPA_PLATFORM=xcb \
        TELAMON_WIZARD_DEMO=1 TELAMON_WIZARD_ANSWERS="$tmp/answers.json" \
        timeout -k 5 120 dbus-run-session --config-file="$tmp/bus.conf" -- \
        xvfb-run -n "$((200 + i))" -s "-screen 0 1920x1080x24" "$tmp/run.sh" >/dev/null 2>&1
}

for i in $(seq 1 "$runs"); do one "$i"; done
python3 - "$out" <<'PY'
import glob, statistics, sys
rows = [list(map(int, open(f).read().split())) for f in sorted(glob.glob(sys.argv[1] + "/run*/result"))]
names = ["first frame (ms)", "first page (ms)", "RSS idle (kB)", "PSS idle (kB)",
         "CPU idle 10 s (ms)", "CPU click-through (ms)", "RSS peak (kB)", "CPU start-up (ms)"]
print(f"{len(rows)} runs")
for i, n in enumerate(names):
    col = [r[i] for r in rows]
    print(f"{n:26s} median {statistics.median(col):>8.0f}   min {min(col):>8}   max {max(col):>8}")
PY

#!/bin/bash
# Headless screenshots of the setup pages in demo mode, inside the dev
# container (never on a real display; OUT.png must be under the repo, which
# is /src in the container):
#   scripts/dev.sh scripts/screenshot.sh OUT.png STEP [light|dark] [SCALE] [ACTIONS] [ANSWERS-JSON]
# STEP is a page id (welcome, language, keyboard, wifi, timezone, account,
# hostname, appearance, privacy, finish) or `first-login`. SCALE is
# QT_SCALE_FACTOR (1, 1.7, 2): the virtual screen is 1920x1080 at 1 and
# 3840x2160 above, like the 4K screen the setup runs on at 1.7. ACTIONS is a
# space-separated list run once the page shows: key:Tab, type:text (~ is a space), click:DX,DY
# (logical pixels from the screen's centre), move:DX,DY, sleep:SECONDS, now (no wait before the shot). ANSWERS-JSON seeds more answers (look,
# accent, userName ...). The colours are Telamon OS's TelamonLight and
# TelamonDark schemes in scripts/schemes/.
# Env: SHOT_SIZE=WxH (the virtual screen), TELAMON_WIZARD_BIN (default build/dev/telamon-wizard), SHOT_WAIT
# seconds before the shot (default 2.5),
# SHOT_LANG (locale, default C.UTF-8), SHOT_FONT_SCALE (text scale answer).
set -euo pipefail

out=${1:?usage: screenshot.sh OUT.png STEP [light|dark] [SCALE] [ACTIONS] [ANSWERS-JSON]}
step=${2:-welcome}
theme=${3:-light}
scale=${4:-1}
actions=${5:-}
extra=${6:-}
bin=${TELAMON_WIZARD_BIN:-build/dev/telamon-wizard}
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
out=$(realpath -m "$out")

tmp=$(mktemp -d)
# A display of its own (xvfb-run -a races when several runs start together):
# the first free number, taken with an atomic mkdir.
disp=
for n in $(seq 100 199); do
    if mkdir "/tmp/.telamon-shot-$n" 2>/dev/null; then
        disp=$n
        break
    fi
done
[ -n "$disp" ] || { echo "screenshot.sh: no free display" >&2; exit 1; }
trap 'rm -rf "$tmp" "/tmp/.telamon-shot-$disp"' EXIT
mkdir -p "$tmp"/{config,data,cache,runtime} "$(dirname "$out")"
chmod 700 "$tmp/runtime"
scheme=$here/schemes/TelamonLight.colors
[ "$theme" = dark ] && scheme=$here/schemes/TelamonDark.colors
cp "$scheme" "$tmp/config/kdeglobals"
printf '\n[General]\nfont=IBM Plex Sans,10,-1,5,400,0,0,0,0,0,0,0,0,0,0,1\nsmallestReadableFont=IBM Plex Sans,8,-1,5,400,0,0,0,0,0,0,0,0,0,0,1\n[Icons]\nTheme=breeze%s\n' "$([ "$theme" = dark ] && echo -dark)" >>"$tmp/config/kdeglobals"
mkdir -p "$tmp/config/fontconfig"
cat >"$tmp/config/fontconfig/fonts.conf" <<'FC'
<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "fonts.dtd">
<fontconfig>
  <alias binding="strong"><family>sans-serif</family><prefer><family>IBM Plex Sans</family></prefer></alias>
</fontconfig>
FC

# The seeded answers: where to start, plus what the page shows.
python3 - "$tmp/answers.json" "$step" "$theme" "$extra" "${SHOT_FONT_SCALE:-1}" <<'PY'
import json, sys
path, step, theme, extra, fs = sys.argv[1:6]
a = {"step": step, "language": "en_US.UTF-8", "keyboardLayout": "us", "timezone": "Europe/Berlin",
     "hostname": "telamon", "look": theme, "textScale": float(fs)}
if extra:
    a.update(json.loads(extra))
json.dump(a, open(path, "w"))
PY

args=()
[ "$step" = first-login ] && args=(--welcome)
width=1920; height=1080
if [ "$scale" != 1 ]; then width=3840; height=2160; fi
# SHOT_SIZE=WxH overrides the screen (a small laptop: 1366x768).
if [ -n "${SHOT_SIZE:-}" ]; then width=${SHOT_SIZE%x*}; height=${SHOT_SIZE#*x}; fi
lang=${SHOT_LANG:-C.UTF-8}

# A session bus that starts no services (no portals, no daemons).
cat >"$tmp/bus.conf" <<'EOF2'
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=/tmp</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
EOF2

cat >"$tmp/run.sh" <<INNER
#!/bin/bash
set -e
cd "$PWD"
"$bin" ${args[*]:-} >"$tmp/app.log" 2>&1 &
app=\$!
w=\$(timeout 60 xdotool search --sync --onlyvisible --name "Telamon Setup" | head -1)
if [ -z "\$w" ]; then
	echo "screenshot.sh: no window after 60 s" >&2
	cat "$tmp/app.log" >&2
	kill \$app 2>/dev/null || true
	exit 1
fi
xdotool windowfocus "\$w" || true
xdotool mousemove 2 2
sleep 1.5
for a in $actions; do
	case "\$a" in
	key:*) xdotool key --delay 80 "\${a#key:}" ;;
	type:*) xdotool type --delay 40 -- "\$(printf %s "\${a#type:}" | tr '~' ' ')" ;;
	click:*) IFS=, read -r x y <<<"\${a#click:}"; xdotool mousemove "\$(awk "BEGIN{print int($width/2+\$x*$scale)}")" "\$(awk "BEGIN{print int($height/2+\$y*$scale)}")" click 1 ;;
	move:*) IFS=, read -r x y <<<"\${a#move:}"; xdotool mousemove "\$(awk "BEGIN{print int($width/2+\$x*$scale)}")" "\$(awk "BEGIN{print int($height/2+\$y*$scale)}")" ;;
	sleep:*) sleep "\${a#sleep:}" ;;
	now) quick=1 ;;
	esac
	sleep 0.25
done
[ -n "\${quick:-}" ] || sleep "${SHOT_WAIT:-2.5}"
import -window root "$out"
kill \$app 2>/dev/null || true
wait \$app 2>/dev/null || true
grep -v "^$" "$tmp/app.log" | head -40 >&2 || true
INNER
chmod +x "$tmp/run.sh"

env XDG_CONFIG_HOME="$tmp/config" XDG_DATA_HOME="$tmp/data" \
    XDG_CACHE_HOME="$tmp/cache" XDG_RUNTIME_DIR="$tmp/runtime" \
    LANG="$lang" LC_ALL="$lang" \
    QT_QPA_PLATFORM=xcb QT_SCALE_FACTOR="$scale" QT_FORCE_STDERR_LOGGING=1 \
    TELAMON_WIZARD_DEMO=1 TELAMON_WIZARD_ANSWERS="$tmp/answers.json" \
    timeout -k 5 120 \
    dbus-run-session --config-file="$tmp/bus.conf" -- xvfb-run -n "$disp" -s "-screen 0 ${width}x${height}x24" "$tmp/run.sh"

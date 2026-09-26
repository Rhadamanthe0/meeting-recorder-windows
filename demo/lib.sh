# Helpers for shooting screenshots and videos in the omavm VM.
#
#   export OUT=/some/dir; source demo/lib.sh
#
# Everything is driven from the host; nothing here plays audio on the host.
# Coordinates for q/park are logical pixels of the guest's 1920x1080 space
# (3840x2160 at scale 2), see demo/README.md.

R=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
OUT=${OUT:-/tmp/meeting-recorder-shoot}; mkdir -p "$OUT"
# `omavm user` runs as the guest user but without the session bus; GTK apps,
# pactl and `omarchy theme set` need it.
BUS='export DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus'
DEMO_MEETING='$HOME/Documents/Meetings/202609241439 Launch sync'

now(){ python3 -c 'import time;print(time.time())'; }
# The app's (or the animation preview's) window as [x, y, w, h], logical pixels.
win(){ omavm hypr clients -j | jq -c '[.[] | select(.class|test("OmarchyMeeting|AnimationPreview")) | .at+.size][0]'; }
# A full 4K screenshot, with the window geometry logged next to it for cropping.
shot(){ omavm shot "$OUT/$1.png" >/dev/null 2>&1; echo "$1 $(win)" >> "$OUT/geom.txt"; }
# A timestamped note in the current take's log, to find moments when cutting.
mark(){ echo "$(python3 -c "import time;print(round(time.time()-${T0:-0},1))") $1 $(win)" >> "$OUT/${LOG:-take}.log"; }
q(){ python3 "$R/demo/qmp.py" "$@"; }
park(){ q move 1915 1075; }   # the pointer out of the way, bottom right

# Screen recording inside the guest (4K, 30 fps, CPU H.264). The second
# argument is yes to show the pointer (for clicks and drags), no otherwise.
rec_start(){ omavm user "rm -f /tmp/$1.mp4; setsid nohup gpu-screen-recorder -w screen -f 30 -k h264 -cursor ${2:-no} -fallback-cpu-encoding yes -o /tmp/$1.mp4 >/tmp/gsr.log 2>&1 < /dev/null & echo \$! > /tmp/gsr.pid"; T0=$(now); LOG=$1; : > "$OUT/$1.log"; sleep 1.5; }
rec_stop(){ omavm user 'kill -INT $(cat /tmp/gsr.pid)'; sleep 4; omavm pull "/tmp/$1.mp4" "$OUT/$1.mp4" >/dev/null 2>&1; }

app(){ omavm user "$BUS; setsid nohup omarchy-meeting-recorder $* >/tmp/app.log 2>&1 < /dev/null &"; }
state(){ timeout 3 omavm user 'omarchy-meeting-recorder watch | head -1'; }
wait_done(){ while ! state | grep -q '"done"'; do sleep 2; done; }
# The demo tracks into the virtual microphone and the default output.
talk(){ omavm user "$BUS; setsid nohup paplay --device=fakemic /tmp/maya-90.wav >/dev/null 2>&1 < /dev/null & setsid nohup paplay --device=demo_speakers /tmp/tom-90.wav >/dev/null 2>&1 < /dev/null &"; }
quiet(){ omavm user 'pkill -x paplay; true'; }

theme(){ omavm user "$BUS; omarchy theme set \"$1\" >/dev/null 2>&1"; sleep 6; }
theme_set(){ omavm user "$BUS; pkill -KILL -x omarchy-meeting; pkill -x paplay; pkill -f transcribe-animation-preview; true"; theme "$1"; }

# Every main state of the app in one theme: $1 theme name, $2 short slug.
# Click positions are for the windows centred at 1920x1080 (done page at
# 410,173 1100x760; recording page at 720,203 480x700).
per_theme(){
  local t=$2
  theme_set "$1"
  app "\"$DEMO_MEETING\""; sleep 4; park
  q click 605 649; sleep 0.8; park; sleep 6; shot $t-done   # play from the third chapter
  q click 827 265; sleep 0.6; park                           # pause playback
  q click 695 817; sleep 2; park                             # New recording
  talk; sleep 3; shot $t-ready
  omavm user 'omarchy-meeting-recorder start'; sleep 4; shot $t-rec
  omavm user 'omarchy-meeting-recorder pause'; sleep 1.5; shot $t-pause
  omavm user 'omarchy-meeting-recorder pause'; sleep 1
  omavm user 'omarchy-meeting-recorder compact'; sleep 2.5; shot $t-compact
  # Throw the recording away: kill it and clear the recovery cache.
  omavm user "$BUS; pkill -9 -x omarchy-meeting; pkill -x paplay; rm -rf ~/.cache/omarchy-meeting-recorder; true"; sleep 1
  omavm user "$BUS; setsid nohup transcribe-animation-preview >/dev/null 2>&1 < /dev/null &"; sleep 3
  local A=$(omavm hypr clients -j | jq -r '.[] | select(.class|test("AnimationPreview")) | .address')
  omavm hypr dispatch "hl.dsp.window.float({ action = \"enable\", window = \"address:$A\" })" >/dev/null
  omavm hypr dispatch "hl.dsp.window.resize({ x = 480, y = 700, window = \"address:$A\" })" >/dev/null
  omavm hypr dispatch "hl.dsp.window.center({ window = \"address:$A\" })" >/dev/null; park
  sleep 12; shot $t-anim; omavm user 'pkill -f transcribe-animation-preview; true'
}

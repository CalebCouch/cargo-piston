#!/bin/bash
set -eu
session=$1
port=$2
shift 2
export DISPLAY=:98
export XDG_CURRENT_DESKTOP=GNOME
export XDG_SESSION_TYPE=x11
export XDG_RUNTIME_DIR="$session/runtime"
mkdir -p "$XDG_RUNTIME_DIR" "$session/web"
chmod 700 "$XDG_RUNTIME_DIR"
trap 'kill $(jobs -pr) 2>/dev/null || true' EXIT
Xvfb :98 -screen 0 1280x900x24 -nolisten tcp > "$session/xvfb.log" 2>&1 &
for attempt in {1..100}; do
    [ -S /tmp/.X11-unix/X98 ] && break
    sleep 0.1
done
openbox > "$session/openbox.log" 2>&1 &
dunst > "$session/notifications.log" 2>&1 &
dbus-update-activation-environment DISPLAY XDG_CURRENT_DESKTOP XDG_RUNTIME_DIR XDG_SESSION_TYPE
cp -as /usr/share/novnc/. "$session/web/"
cp "$session/token" "$session/web/piston-session"
IFS= read -r password < "$session/password"
x11vnc -storepasswd "$password" "$session/vnc-auth" >/dev/null
chmod 600 "$session/vnc-auth"
x11vnc -display :98 -localhost -rfbport 5901 -forever -shared -rfbauth "$session/vnc-auth" \
    -noxdamage > "$session/vnc.log" 2>&1 &
websockify --web="$session/web" "127.0.0.1:$port" 127.0.0.1:5901 > "$session/viewer.log" 2>&1 &
"$@" &
wait "$!"

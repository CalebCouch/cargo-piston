#!/bin/bash
set -eu
required=(cc pkg-config Xvfb openbox dunst x11vnc websockify python3 curl)
missing=false
for tool in "${required[@]}"; do command -v "$tool" >/dev/null || missing=true; done
for package in libssl-dev libasound2-dev libwayland-dev libxkbcommon-dev libxkbcommon-x11-0 \
    libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev gstreamer1.0-plugins-base \
    gstreamer1.0-plugins-good libvulkan1 mesa-vulkan-drivers novnc xdg-desktop-portal-gtk; do
    dpkg-query -W -f='${Status}' "$package" 2>/dev/null | grep -q 'install ok installed' || missing=true
done
if "$missing"; then
    sudo apt-get update
    sudo DEBIAN_FRONTEND=noninteractive apt-get install -y build-essential pkg-config libasound2-dev \
        libwayland-dev libxkbcommon-dev libxkbcommon-x11-0 xvfb openbox dunst x11vnc novnc \
        websockify xdg-desktop-portal xdg-desktop-portal-gtk dbus-x11 curl ca-certificates \
        libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev gstreamer1.0-plugins-base \
        gstreamer1.0-plugins-good libvulkan1 mesa-vulkan-drivers libssl-dev
fi
piston_swap=/var/lib/piston/swap
if ! swapon --show=NAME --noheadings | grep -Fxq "$piston_swap"; then
    if [ "$(sudo blkid -p -s TYPE -o value "$piston_swap" 2>/dev/null || true)" != swap ]; then
        sudo install -d -m 700 /var/lib/piston
        sudo install -m 600 /dev/null "$piston_swap"
        sudo dd if=/dev/zero of="$piston_swap" bs=1M count=2048 status=none
        sudo mkswap "$piston_swap"
    fi
    sudo swapon "$piston_swap"
fi
if [ ! -x "$HOME/.cargo/bin/cargo" ]; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o "$HOME/.cache/piston/rustup.sh"
    sh "$HOME/.cache/piston/rustup.sh" -y --profile minimal
fi

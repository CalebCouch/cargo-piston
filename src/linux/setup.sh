#!/bin/bash
set -eu
required=(cc pkg-config Xvfb openbox dunst x11vnc websockify python3 curl)
missing=false
for tool in "${required[@]}"; do command -v "$tool" >/dev/null || missing=true; done
for package in libasound2-dev libwayland-dev libxkbcommon-dev libxkbcommon-x11-0 \
    libvulkan1 mesa-vulkan-drivers novnc xdg-desktop-portal-gtk; do
    dpkg-query -W -f='${Status}' "$package" 2>/dev/null | grep -q 'install ok installed' || missing=true
done
if "$missing"; then
    sudo apt-get update
    sudo DEBIAN_FRONTEND=noninteractive apt-get install -y build-essential pkg-config libasound2-dev \
        libwayland-dev libxkbcommon-dev libxkbcommon-x11-0 xvfb openbox dunst x11vnc novnc \
        websockify xdg-desktop-portal xdg-desktop-portal-gtk dbus-x11 curl ca-certificates \
        libvulkan1 mesa-vulkan-drivers
fi
if [ ! -x "$HOME/.cargo/bin/cargo" ]; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o "$HOME/.cache/piston/rustup.sh"
    sh "$HOME/.cache/piston/rustup.sh" -y --profile minimal
fi

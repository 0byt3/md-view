#!/usr/bin/env bash
# Headless Wayland session for running the md-view desktop app in a Cloud
# Agent (no physical GPU or display). Runs a wlroots compositor (sway) with
# its headless backend and the software (pixman) renderer in the foreground so
# its logs stay visible in this terminal.
#
# md-view itself renders through Vulkan; point it at the software Vulkan ICD
# (lavapipe) with the matching env vars. Use ./.cursor/run-md-view.sh <file>,
# which sets everything up for you, or export the same vars manually:
#   export XDG_RUNTIME_DIR=/tmp/mdview-wayland
#   export WAYLAND_DISPLAY=wayland-1
#   export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json
set -euo pipefail

export XDG_RUNTIME_DIR=/tmp/mdview-wayland
mkdir -p "${XDG_RUNTIME_DIR}"
chmod 700 "${XDG_RUNTIME_DIR}"

export WLR_BACKENDS=headless
export WLR_RENDERER=pixman
export WLR_LIBINPUT_NO_DEVICES=1
export WAYLAND_DISPLAY=wayland-1

CONFIG="${XDG_RUNTIME_DIR}/sway.config"
cat > "${CONFIG}" <<'EOF'
output HEADLESS-1 resolution 1280x900 position 0 0
default_border none
EOF

echo "Starting headless sway compositor on WAYLAND_DISPLAY=${WAYLAND_DISPLAY}"
echo "Run the viewer with: ./.cursor/run-md-view.sh <file.md>"
exec sway -c "${CONFIG}"

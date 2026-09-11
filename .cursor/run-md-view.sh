#!/usr/bin/env bash
# Convenience launcher: runs the built md-view binary against the headless
# Wayland session started by .cursor/wayland-session.sh, using software Vulkan
# (lavapipe). Usage: ./.cursor/run-md-view.sh <file.md>
set -euo pipefail

FILE="${1:-demo.md}"

export XDG_RUNTIME_DIR=/tmp/mdview-wayland
export WAYLAND_DISPLAY=wayland-1
export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json

if [[ ! -S "${XDG_RUNTIME_DIR}/${WAYLAND_DISPLAY}" ]]; then
  echo "No headless Wayland session found at ${XDG_RUNTIME_DIR}/${WAYLAND_DISPLAY}." >&2
  echo "Start it first (it runs as the 'wayland-headless' terminal), e.g.:" >&2
  echo "  ./.cursor/wayland-session.sh &" >&2
  exit 1
fi

BIN=./target/debug/md-view
[[ -x "${BIN}" ]] || BIN=./target/release/md-view
exec "${BIN}" "${FILE}"

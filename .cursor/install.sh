#!/usr/bin/env bash
# Idempotent Cloud Agent bootstrap for md-view.
#
# Installs the system libraries GPUI needs to build and run on Wayland, makes
# sure a Rust toolchain new enough for the pinned zed/GPUI checkout (which uses
# edition 2024, so >= 1.85) is the default, and warms the cargo build so the
# large git dependency is compiled once.
set -euo pipefail

# --- System packages -------------------------------------------------------
# Build deps for GPUI + its transitive crates, plus a headless runtime stack
# (software Vulkan via lavapipe, a wlroots compositor, screenshot/clipboard
# tools) so the desktop viewer can actually run without a physical GPU/display.
PACKAGES=(
  build-essential pkg-config cmake clang
  libssl-dev libwayland-dev libxkbcommon-dev libxkbcommon-x11-dev
  libfontconfig-dev libfreetype-dev libasound2-dev libzstd-dev
  libvulkan-dev libgbm-dev
  mesa-vulkan-drivers vulkan-tools sway grim wl-clipboard fonts-dejavu
)

export DEBIAN_FRONTEND=noninteractive
sudo apt-get update -y
sudo apt-get install -y --no-install-recommends "${PACKAGES[@]}"

# --- Rust toolchain --------------------------------------------------------
# The base image ships Rust 1.83, which predates edition 2024; the pinned GPUI
# checkout requires it. Install and default to the current stable toolchain.
if command -v rustup >/dev/null 2>&1; then
  rustup toolchain install stable --profile minimal --component rustfmt --component clippy
  rustup default stable
fi

rustc --version
cargo --version

# --- Warm the build --------------------------------------------------------
# Compiles the pinned zed/GPUI git dependency once so later builds/tests are
# fast. Safe to re-run: cargo is incremental and idempotent.
cargo build

echo "md-view environment ready."

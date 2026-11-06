#!/usr/bin/env bash
# install.sh - One-command installer for macbook-setup
# Usage: /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/nullaroni/soulcaster/main/install.sh)"
#
# Works on a bare Mac: installs Xcode Command Line Tools (for git),
# clones/updates this repo, then hands off to bootstrap.sh.

set -euo pipefail

REPO_URL="https://github.com/nullaroni/soulcaster.git"
INSTALL_DIR="$HOME/Developer/macbook-setup"

echo "================================"
echo "  macbook-setup installer"
echo "================================"
echo

# 1. Xcode Command Line Tools (provides git)
if xcode-select -p >/dev/null 2>&1; then
  echo "[ok] Xcode Command Line Tools already installed"
else
  echo "==> Installing Xcode Command Line Tools..."
  echo "    A system dialog will appear - click 'Install' to continue."
  xcode-select --install 2>/dev/null || true

  # Wait for the installation to finish
  until xcode-select -p >/dev/null 2>&1; do
    sleep 5
  done
  echo "[ok] Xcode Command Line Tools installed"
fi

# 2. Clone or update the repo
if [[ -d "$INSTALL_DIR/.git" ]]; then
  echo "==> Updating existing repo in $INSTALL_DIR..."
  git -C "$INSTALL_DIR" pull
else
  echo "==> Cloning macbook-setup to $INSTALL_DIR..."
  mkdir -p "$(dirname "$INSTALL_DIR")"
  git clone "$REPO_URL" "$INSTALL_DIR"
fi

# 3. Hand off to bootstrap
echo "==> Starting bootstrap..."
echo
exec "$INSTALL_DIR/bootstrap.sh" "$@"

#!/usr/bin/env bash
# xcode.sh - Install Xcode Command Line Tools

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$SCRIPT_DIR/lib.sh"

log_section "Xcode Command Line Tools"

if xcode-select -p >/dev/null 2>&1; then
  log_ok "Xcode Command Line Tools already installed"
  exit 0
fi

log_warn "Xcode Command Line Tools not found."

if [[ "${DRY_RUN:-false}" == "true" ]]; then
  log_skip "(dry-run) Would trigger xcode-select --install"
  exit 0
fi

log_section "Triggering Xcode Command Line Tools installation..."
log_warn "A GUI dialog will appear. Please complete the installation."

# Trigger the install
xcode-select --install || true

# Wait for the installation to finish
log_warn "Waiting for installation to finish..."
until xcode-select -p >/dev/null 2>&1; do
  sleep 5
done

log_ok "Xcode Command Line Tools installed successfully"

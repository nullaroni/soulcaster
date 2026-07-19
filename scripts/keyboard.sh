#!/usr/bin/env bash
# keyboard.sh - Configure keyboard settings

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$SCRIPT_DIR/lib.sh"

log_section "Keyboard Settings"

# Fastest key repeat
run_or_dry defaults write NSGlobalDomain KeyRepeat -int 2
log_ok "Key repeat: fastest"

# Shortest delay until repeat
run_or_dry defaults write NSGlobalDomain InitialKeyRepeat -int 15
log_ok "Initial key repeat: shortest"

# Disable press-and-hold accent menu
run_or_dry defaults write NSGlobalDomain ApplePressAndHoldEnabled -bool false
log_ok "Press-and-hold accent menu: disabled"

log_ok "Keyboard settings applied (log out/restart required to take effect)"

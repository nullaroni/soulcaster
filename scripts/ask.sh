#!/usr/bin/env bash
# ask.sh - Interactive prompts for missing arguments

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$SCRIPT_DIR/lib.sh"

ask_role() {
  if [[ -n "${MACHINE_ROLE:-}" ]]; then
    log_ok "Role already set: $MACHINE_ROLE"
    return 0
  fi

  echo
  echo "Select machine role:"
  select role in personal work; do
    if [[ -n "$role" ]]; then
      MACHINE_ROLE="$role"
      export MACHINE_ROLE
      log_ok "Role set to: $MACHINE_ROLE"
      break
    fi
  done
}

ask_git_name() {
  if [[ -n "${GIT_NAME:-}" ]]; then
    log_ok "Git name already set: $GIT_NAME"
    return 0
  fi

  # Fall back to existing global git config, if any
  local existing
  existing="$(git config --global user.name 2>/dev/null || true)"
  if [[ -n "$existing" ]]; then
    GIT_NAME="$existing"
    export GIT_NAME
    log_ok "Git name from existing git config: $GIT_NAME"
    return 0
  fi

  read -rp "Enter your Git full name: " name
  if [[ -z "$name" ]]; then
    log_fail "Git name cannot be empty."
    exit 1
  fi
  GIT_NAME="$name"
  export GIT_NAME
  log_ok "Git name set."
}

ask_git_email() {
  if [[ -n "${GIT_EMAIL:-}" ]]; then
    log_ok "Git email already set: $GIT_EMAIL"
    return 0
  fi

  # Fall back to existing global git config, if any
  local existing
  existing="$(git config --global user.email 2>/dev/null || true)"
  if [[ -n "$existing" ]]; then
    GIT_EMAIL="$existing"
    export GIT_EMAIL
    log_ok "Git email from existing git config: $GIT_EMAIL"
    return 0
  fi

  read -rp "Enter your Git email: " email
  if [[ -z "$email" ]]; then
    log_fail "Git email cannot be empty."
    exit 1
  fi
  GIT_EMAIL="$email"
  export GIT_EMAIL
  log_ok "Git email set."
}

confirm_choices() {
  echo
  echo "================================"
  echo "  Configuration Summary"
  echo "================================"
  echo "  Role:      ${MACHINE_ROLE}"
  echo "  Git Name:  ${GIT_NAME}"
  echo "  Git Email: ${GIT_EMAIL}"
  echo "================================"
  echo

  # Auto-confirm if running non-interactively with all flags set
  if [[ -n "${MACHINE_ROLE:-}" && -n "${GIT_NAME:-}" && -n "${GIT_EMAIL:-}" && ! -t 0 ]]; then
    log_ok "Non-interactive mode: auto-confirming"
    return 0
  fi

  read -rp "Continue? [Y/n]: " confirm || true
  if [[ "$confirm" =~ ^[Nn]$ ]]; then
    log_fail "Aborted by user."
    exit 1
  fi
}

#!/usr/bin/env bash
# bootstrap.sh - Main entrypoint for macbook-setup
# Usage: ./bootstrap.sh [--role personal|work] [--git-name NAME] [--git-email EMAIL]
#                       [--skip-brew] [--skip-macos] [--skip-dotfiles] [--skip-packages]
#                       [--dry-run] [--help]

set -euo pipefail

# Determine repo root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$SCRIPT_DIR"

# shellcheck source=scripts/lib.sh
source "$REPO_ROOT/scripts/lib.sh"

# Default flags
MACHINE_ROLE=""
GIT_NAME=""
GIT_EMAIL=""
SKIP_BREW=false
SKIP_MACOS=false
SKIP_DOTFILES=false
SKIP_PACKAGES=false
DRY_RUN=false
DOTFILES_REPO=""

# Print usage
usage() {
  cat <<EOF
Usage: ./bootstrap.sh [OPTIONS]

Options:
  --role personal|work    Machine role
  --git-name NAME         Git user name
  --git-email EMAIL       Git user email
  --dotfiles-repo URL     Clone dotfiles from this Git repo (optional)
  --skip-brew             Skip Homebrew installation
  --skip-macos            Skip macOS settings
  --skip-dotfiles         Skip dotfiles linking
  --skip-packages         Skip package installation
  --dry-run               Show what would be done
  --help                  Show this help message
EOF
}

# Parse arguments
while [[ $# -gt 0 ]]; do
  case "$1" in
    --role)
      MACHINE_ROLE="$2"
      shift 2
      ;;
    --git-name)
      GIT_NAME="$2"
      shift 2
      ;;
    --git-email)
      GIT_EMAIL="$2"
      shift 2
      ;;
    --dotfiles-repo)
      DOTFILES_REPO="$2"
      shift 2
      ;;
    --skip-brew)
      SKIP_BREW=true
      shift
      ;;
    --skip-macos)
      SKIP_MACOS=true
      shift
      ;;
    --skip-dotfiles)
      SKIP_DOTFILES=true
      shift
      ;;
    --skip-packages)
      SKIP_PACKAGES=true
      shift
      ;;
    --dry-run)
      DRY_RUN=true
      shift
      ;;
    --help)
      usage
      exit 0
      ;;
    *)
      log_fail "Unknown option: $1"
      usage
      exit 1
      ;;
  esac
done

# Load persisted choices from a previous run (gitignored).
# Entries use ${VAR:-default} form, so CLI flags above always win.
LOCAL_ENV="$REPO_ROOT/.local.env"
if [[ -f "$LOCAL_ENV" ]]; then
  # shellcheck disable=SC1090
  source "$LOCAL_ENV"
fi

# Export flags so sub-scripts can see them
export MACHINE_ROLE GIT_NAME GIT_EMAIL SKIP_BREW SKIP_MACOS SKIP_DOTFILES SKIP_PACKAGES DRY_RUN DOTFILES_REPO

# Banner
echo "================================"
echo "  macbook-setup"
echo "  macOS Provisioning"
echo "================================"
echo

# Validate macOS
require_macos

# Interactive prompts for missing values
if [[ -z "$MACHINE_ROLE" ]]; then
  # shellcheck source=scripts/ask.sh
  source "$REPO_ROOT/scripts/ask.sh"
  ask_role
fi

if [[ -z "$GIT_NAME" ]]; then
  # shellcheck source=scripts/ask.sh
  source "$REPO_ROOT/scripts/ask.sh"
  ask_git_name
fi

if [[ -z "$GIT_EMAIL" ]]; then
  # shellcheck source=scripts/ask.sh
  source "$REPO_ROOT/scripts/ask.sh"
  ask_git_email
fi

# Re-source ask.sh if it hasn't been loaded, for confirm_choices
if ! type confirm_choices &>/dev/null; then
  # shellcheck source=scripts/ask.sh
  source "$REPO_ROOT/scripts/ask.sh"
fi
confirm_choices

# Persist choices so reruns are hands-free (file is gitignored).
# Delete .local.env to be asked again on the next run.
if [[ "$DRY_RUN" != "true" ]]; then
  cat > "$LOCAL_ENV" <<EOF
# macbook-setup local configuration (auto-generated, gitignored)
# Delete this file to be asked again on the next run.
MACHINE_ROLE="\${MACHINE_ROLE:-$MACHINE_ROLE}"
GIT_NAME="\${GIT_NAME:-$GIT_NAME}"
GIT_EMAIL="\${GIT_EMAIL:-$GIT_EMAIL}"
EOF
fi

# Source role config
if [[ "$MACHINE_ROLE" == "personal" ]]; then
  # shellcheck source=config/personal.env
  source "$REPO_ROOT/config/personal.env"
elif [[ "$MACHINE_ROLE" == "work" ]]; then
  # shellcheck source=config/work.env
  source "$REPO_ROOT/config/work.env"
else
  log_fail "Invalid role: $MACHINE_ROLE"
  exit 1
fi

# Source common config
# shellcheck source=config/common.env
source "$REPO_ROOT/config/common.env"

# 1. Xcode Command Line Tools
bash "$REPO_ROOT/scripts/xcode.sh"

# 2. Homebrew
if [[ "$SKIP_BREW" != "true" ]]; then
  bash "$REPO_ROOT/scripts/homebrew.sh"
else
  log_skip "Homebrew installation skipped"
fi

# 3. Packages (before some macOS tools that need them)
if [[ "$SKIP_PACKAGES" != "true" ]]; then
  bash "$REPO_ROOT/scripts/packages.sh"
else
  log_skip "Package installation skipped"
fi

# 4. Git config
bash "$REPO_ROOT/scripts/git.sh"

# 5. Developer directory
log_section "Developer Directory"
ensure_dir "$HOME/Developer"
log_ok "~/Developer ready"

# 6. Dotfiles
bash "$REPO_ROOT/scripts/dotfiles.sh"

# 7. macOS settings
if [[ "$SKIP_MACOS" != "true" ]]; then
  bash "$REPO_ROOT/scripts/keyboard.sh"
  bash "$REPO_ROOT/scripts/finder.sh"
  bash "$REPO_ROOT/scripts/dock.sh"
  bash "$REPO_ROOT/scripts/menu-bar.sh"
  bash "$REPO_ROOT/scripts/macos.sh"
else
  log_skip "macOS settings skipped"
fi

# 8. Kotlin
bash "$REPO_ROOT/scripts/kotlin.sh"

# 8.5 Kotlin Language Server
bash "$REPO_ROOT/scripts/kotlin-lsp.sh"

# 9. Android
bash "$REPO_ROOT/scripts/android.sh"

# 10. PATH verification / fixup for non-Homebrew tools
bash "$REPO_ROOT/scripts/paths.sh"

# 11. Postflight
bash "$REPO_ROOT/scripts/postflight.sh"

# Report failed package installs with rerun commands
FAILED_BREWFILES_FILE="/tmp/macbook-setup-failed-brewfiles"
if [[ "$DRY_RUN" != "true" && -s "$FAILED_BREWFILES_FILE" ]]; then
  echo
  log_warn "Some packages failed to install. Rerun them with:"
  while IFS= read -r brewfile; do
    echo "  brew bundle --file $brewfile"
  done < "$FAILED_BREWFILES_FILE"
  rm -f "$FAILED_BREWFILES_FILE"
  echo
fi

# Manual steps reminder
echo
echo "================================"
echo "  Remaining Manual Steps"
echo "================================"
echo "  See: docs/manual-steps.md"
echo "================================"
echo

log_ok "All done!"

# Offer to restart so system settings take effect
if [[ "$DRY_RUN" == "true" ]]; then
  log_skip "(dry-run) Would offer to restart"
elif [[ -t 0 ]]; then
  echo
  read -rp "Restart now to apply all changes? [y/N]: " restart || true
  if [[ "$restart" =~ ^[Yy]$ ]]; then
    log_warn "Restarting..."
    sudo shutdown -r now
  else
    log_ok "No restart. Log out and back in (or restart) later to apply all settings."
  fi
fi

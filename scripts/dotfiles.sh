#!/usr/bin/env bash
# dotfiles.sh - Clone and symlink dotfiles from a real dotfiles repo

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"
# shellcheck source=lib.sh
source "$SCRIPT_DIR/lib.sh"

log_section "Dotfiles"

if [[ "${SKIP_DOTFILES:-false}" == "true" ]]; then
  log_skip "Dotfiles linking skipped via flag"
  exit 0
fi

DOTFILES_DIR="$HOME/Developer/dotfiles"

# Ask for dotfiles repo if not provided via flag
if [[ -z "${DOTFILES_REPO:-}" ]]; then
  if [[ -t 0 ]]; then
    read -rp "Dotfiles repo URL (leave empty to skip): " DOTFILES_REPO || true
  fi
fi

# If still empty, skip
if [[ -z "${DOTFILES_REPO:-}" ]]; then
  log_skip "No dotfiles repo provided. Apps will use default configs."
  exit 0
fi

# Clone or update the dotfiles repo
if [[ -d "$DOTFILES_DIR/.git" ]]; then
  log_ok "Dotfiles repo already cloned"
  if [[ "${DRY_RUN:-false}" != "true" ]]; then
    log_section "Updating dotfiles repo..."
    git -C "$DOTFILES_DIR" pull || log_warn "git pull failed, using existing dotfiles"
  fi
else
  if [[ "${DRY_RUN:-false}" == "true" ]]; then
    log_skip "(dry-run) Would clone $DOTFILES_REPO to $DOTFILES_DIR"
    exit 0
  fi
  ensure_dir "$HOME/Developer"
  log_warn "Cloning dotfiles repo..."
  git clone "$DOTFILES_REPO" "$DOTFILES_DIR"
  log_ok "Dotfiles repo cloned to $DOTFILES_DIR"
fi

# If the dotfiles repo has its own install script, run that instead
if [[ -f "$DOTFILES_DIR/install.sh" ]]; then
  log_section "Running dotfiles install.sh..."
  if [[ "${DRY_RUN:-false}" == "true" ]]; then
    log_skip "(dry-run) Would run: bash $DOTFILES_DIR/install.sh"
  else
    bash "$DOTFILES_DIR/install.sh"
  fi
  log_ok "Dotfiles install.sh complete"
  exit 0
fi

# Otherwise, symlink known dotfiles from the repo
log_section "Symlinking dotfiles..."

# Individual files and directories
link_file "$DOTFILES_DIR/zsh/zshrc"          "$HOME/.zshrc"
link_file "$DOTFILES_DIR/git/gitignore_global" "$HOME/.gitignore_global"
link_file "$DOTFILES_DIR/ghostty"            "$HOME/.config/ghostty"
link_file "$DOTFILES_DIR/aerospace/aerospace.toml" "$HOME/.config/aerospace/aerospace.toml"
link_file "$DOTFILES_DIR/nvim"               "$HOME/.config/nvim"
link_file "$DOTFILES_DIR/starship/starship.toml" "$HOME/.config/starship.toml"
link_file "$DOTFILES_DIR/yazi"               "$HOME/.config/yazi"

log_ok "Dotfiles linked from $DOTFILES_DIR"

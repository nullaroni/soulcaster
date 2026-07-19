#!/usr/bin/env bash
# packages.sh - Install packages via Homebrew bundle

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"
# shellcheck source=lib.sh
source "$SCRIPT_DIR/lib.sh"

# bootstrap.sh reads this file at the end to print rerun commands
FAILED_BREWFILES_FILE="/tmp/macbook-setup-failed-brewfiles"

log_section "Package Installation"

if [[ "${SKIP_PACKAGES:-false}" == "true" ]]; then
  log_skip "Package installation skipped via flag"
  exit 0
fi

if ! command_exists brew; then
  if [[ "${DRY_RUN:-false}" == "true" ]]; then
    log_skip "(dry-run) Homebrew not installed yet, skipping brew bundle steps"
    exit 0
  fi
  log_fail "Homebrew not found. Run homebrew.sh first."
  exit 1
fi

if [[ "${DRY_RUN:-false}" != "true" ]]; then
  : > "$FAILED_BREWFILES_FILE"
fi

# Update Homebrew to ensure taps are available
log_section "Updating Homebrew..."
if [[ "${DRY_RUN:-false}" == "true" ]]; then
  log_skip "(dry-run) Would run: brew update"
else
  brew update || log_warn "brew update had issues, continuing..."
fi

# Trust third-party taps declared in the Brewfiles so Homebrew will load
# their formulae/casks (Homebrew 6+ refuses untrusted taps)
log_section "Trusting Brewfile taps..."
while IFS= read -r tap; do
  if [[ "${DRY_RUN:-false}" == "true" ]]; then
    log_skip "(dry-run) Would run: brew tap $tap && brew trust $tap"
  else
    brew tap "$tap" >/dev/null 2>&1 || true
    brew trust "$tap" || log_warn "Could not trust tap: $tap"
  fi
done < <(grep -h '^tap ' "$REPO_ROOT"/brew/Brewfile.* 2>/dev/null \
  | sed -E 's/^tap "([^"]+)".*/\1/' \
  | grep -v '^homebrew/' \
  | sort -u)

# Run brew bundle for a Brewfile, recording (not aborting on) failures
run_bundle() {
  local brewfile="$1"

  if [[ ! -f "$brewfile" ]]; then
    return 0
  fi

  if [[ "${DRY_RUN:-false}" == "true" ]]; then
    log_skip "(dry-run) Would run: brew bundle --file $brewfile"
    return 0
  fi

  if ! brew bundle --file "$brewfile"; then
    log_warn "Some packages in $brewfile failed to install:"
    brew bundle check --verbose --file "$brewfile" || true
    echo "$brewfile" >> "$FAILED_BREWFILES_FILE"
  fi
}

# Common packages
log_section "Installing common packages..."
run_bundle "$REPO_ROOT/brew/Brewfile.common"

# Role-specific packages
if [[ "${MACHINE_ROLE:-}" == "personal" ]]; then
  log_section "Installing personal packages..."
  run_bundle "$REPO_ROOT/brew/Brewfile.personal"
elif [[ "${MACHINE_ROLE:-}" == "work" ]]; then
  log_section "Installing work packages..."
  run_bundle "$REPO_ROOT/brew/Brewfile.work"
fi

if [[ "${DRY_RUN:-false}" != "true" && -s "$FAILED_BREWFILES_FILE" ]]; then
  log_warn "Some packages failed to install. Rerun commands will be shown in the final summary."
else
  log_ok "Packages installed"
fi

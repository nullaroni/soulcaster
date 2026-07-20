#!/usr/bin/env bash
# apply-config.sh - Apply a machine.json config produced by the web UI
# Usage: ./scripts/apply-config.sh [path/to/machine.json]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"

# shellcheck source=lib.sh
source "$SCRIPT_DIR/lib.sh"

CONFIG_FILE="${1:-$REPO_ROOT/machine.json}"

if [[ ! -f "$CONFIG_FILE" ]]; then
  log_fail "Config not found: $CONFIG_FILE"
  echo "Generate one from the web UI, or pass a path: ./scripts/apply-config.sh ./machine.json" >&2
  exit 1
fi

require_macos

log_section "Loading config: $CONFIG_FILE"

# Parse JSON + emit Brewfile via Python 3 (available on modern macOS).
# Catalog package mapping must stay in sync with web/js/catalog.js
EVAL_AND_BREW="$(
  python3 - "$CONFIG_FILE" "$REPO_ROOT/brew/Brewfile.generated" <<'PY'
import json, shlex, sys
from pathlib import Path

config_path, brewfile_path = sys.argv[1], sys.argv[2]

CATALOG = {
    "dockutil": {"brew": "dockutil"},
    "neovim": {"brew": "neovim"},
    "aerospace": {"cask": "nikitabobko/tap/aerospace", "tap": "nikitabobko/tap"},
    "eza": {"brew": "eza"},
    "fzf": {"brew": "fzf"},
    "zoxide": {"brew": "zoxide"},
    "yazi": {"brew": "yazi"},
    "starship": {"brew": "starship"},
    "gh": {"brew": "gh"},
    "jj": {"brew": "jj"},
    "opencode": {"brew": "opencode"},
    "lua-language-server": {"brew": "lua-language-server"},
    "basedpyright": {"brew": "basedpyright"},
    "typescript-language-server": {"brew": "typescript-language-server"},
    "vscode-langservers-extracted": {"brew": "vscode-langservers-extracted"},
    "maestro": {"brew": "mobile-dev-inc/tap/maestro", "tap": "mobile-dev-inc/tap"},
    "junie": {"brew": "junie", "tap": "jetbrains/junie"},
    "linearmouse": {"cask": "linearmouse"},
    "ghostty": {"cask": "ghostty"},
    "jetbrains-toolbox": {"cask": "jetbrains-toolbox"},
    "discord": {"cask": "discord"},
    "azure-cli": {"brew": "azure-cli"},
}

with open(config_path, encoding="utf-8") as f:
    cfg = json.load(f)

def emit(name, value):
    print(f"export {name}={shlex.quote(str(value))}")

git = cfg.get("git") or {}
dot = cfg.get("dotfiles") or {}
apps = cfg.get("apps") or []
settings = cfg.get("settings") or []
modules = cfg.get("modules") or []

emit("CFG_GIT_NAME", git.get("name") or "")
emit("CFG_GIT_EMAIL", git.get("email") or "")
emit("CFG_DOTFILES_REPO", dot.get("repo") or "")
emit("CFG_APPS", " ".join(apps))
emit("CFG_SETTINGS", " ".join(settings))
emit("CFG_MODULES", " ".join(modules))

taps = []
lines = []
for app_id in apps:
    meta = CATALOG.get(app_id)
    if not meta:
        continue
    if meta.get("tap"):
        taps.append(meta["tap"])
    if meta.get("brew"):
        lines.append(f'brew "{meta["brew"]}"')
    if meta.get("cask"):
        lines.append(f'cask "{meta["cask"]}"')

brewfile = []
for tap in sorted(set(taps)):
    brewfile.append(f'tap "{tap}"')
if taps:
    brewfile.append("")
brewfile.extend(lines)

Path(brewfile_path).write_text("\n".join(brewfile) + ("\n" if brewfile else ""), encoding="utf-8")
emit("CFG_BREWFILE", brewfile_path)
PY
)"

eval "$EVAL_AND_BREW"

export GIT_NAME="${CFG_GIT_NAME}"
export GIT_EMAIL="${CFG_GIT_EMAIL}"
export DOTFILES_REPO="${CFG_DOTFILES_REPO}"
export DRY_RUN="${DRY_RUN:-false}"
export SKIP_BREW=true
export SKIP_PACKAGES=true
export SKIP_MACOS=true
export SKIP_DOTFILES=true
export MACHINE_ROLE="${MACHINE_ROLE:-custom}"

has_module() {
  [[ " ${CFG_MODULES} " == *" $1 "* ]]
}

has_setting() {
  [[ " ${CFG_SETTINGS} " == *" $1 "* ]]
}

echo "================================"
echo "  macbook-setup"
echo "  Applying machine.json"
echo "================================"
echo
log_ok "Apps: ${CFG_APPS:-"(none)"}"
log_ok "Settings: ${CFG_SETTINGS:-"(none)"}"
log_ok "Modules: ${CFG_MODULES:-"(none)"}"

if has_module xcode; then
  bash "$REPO_ROOT/scripts/xcode.sh"
else
  log_skip "Xcode CLT module not selected"
fi

if has_module homebrew; then
  export SKIP_BREW=false
  bash "$REPO_ROOT/scripts/homebrew.sh"
else
  log_skip "Homebrew module not selected"
fi

if [[ -n "${CFG_APPS// }" ]]; then
  if ! command_exists brew; then
    log_fail "Homebrew is required to install selected apps"
    exit 1
  fi

  BREWFILE="${CFG_BREWFILE}"
  log_section "Installing selected packages from Brewfile.generated"
  if [[ "$DRY_RUN" == "true" ]]; then
    log_skip "(dry-run) Would brew bundle --file $BREWFILE"
    cat "$BREWFILE"
  else
    brew update || log_warn "brew update had issues, continuing..."
    while IFS= read -r tap; do
      [[ -z "$tap" ]] && continue
      brew tap "$tap" >/dev/null 2>&1 || true
      brew trust "$tap" || log_warn "Could not trust tap: $tap"
    done < <(grep -E '^tap "' "$BREWFILE" | sed -E 's/^tap "([^"]+)".*/\1/' | grep -v '^homebrew/' || true)

    if ! brew bundle --file "$BREWFILE"; then
      log_warn "Some packages failed to install:"
      brew bundle check --verbose --file "$BREWFILE" || true
    else
      log_ok "Selected packages installed"
    fi
  fi
else
  log_skip "No apps selected"
fi

if has_module git; then
  if [[ -z "$GIT_NAME" || -z "$GIT_EMAIL" ]]; then
    log_warn "Git module selected but name/email empty — skipping git config"
  else
    bash "$REPO_ROOT/scripts/git.sh"
  fi
else
  log_skip "Git module not selected"
fi

if has_module developer_dir; then
  log_section "Developer Directory"
  ensure_dir "$HOME/Developer"
  log_ok "~/Developer ready"
fi

if [[ -n "${DOTFILES_REPO}" ]]; then
  export SKIP_DOTFILES=false
  bash "$REPO_ROOT/scripts/dotfiles.sh"
else
  log_skip "No dotfiles repo in config"
fi

if has_setting keyboard; then bash "$REPO_ROOT/scripts/keyboard.sh"; fi
if has_setting finder; then bash "$REPO_ROOT/scripts/finder.sh"; fi
if has_setting dock; then bash "$REPO_ROOT/scripts/dock.sh"; fi
if has_setting menu_bar; then bash "$REPO_ROOT/scripts/menu-bar.sh"; fi
if has_setting macos; then bash "$REPO_ROOT/scripts/macos.sh"; fi

if has_module kotlin; then bash "$REPO_ROOT/scripts/kotlin.sh"; fi
if has_module kotlin_lsp; then bash "$REPO_ROOT/scripts/kotlin-lsp.sh"; fi
if has_module android; then bash "$REPO_ROOT/scripts/android.sh"; fi

bash "$REPO_ROOT/scripts/paths.sh"
bash "$REPO_ROOT/scripts/postflight.sh"

echo
log_ok "machine.json applied"
echo "  Config: $CONFIG_FILE"
echo "  Manual steps: $REPO_ROOT/docs/manual-steps.md"
echo

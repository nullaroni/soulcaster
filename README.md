# macbook-setup

A boring, rerunnable, inspectable macOS provisioning system for Apple Silicon Macs.

## Quick Start

One command on a bare Mac (installs Xcode Command Line Tools, clones this repo, runs bootstrap):

```bash
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/peytonmscott/macbook-setup/main/install.sh)"
```

Or manually:

```bash
git clone https://github.com/peytonmscott/macbook-setup.git ~/Developer/macbook-setup
cd ~/Developer/macbook-setup
./bootstrap.sh
```

After the first run, your role and Git identity are saved to `.local.env` (gitignored), so reruns are hands-free. Delete that file to be asked again.

## Features

- **One command** to provision a new Mac.
- **Role-based** setup: `personal` or `work`.
- **Idempotent** — safe to rerun.
- **Dry-run mode** for testing changes.
- **Boring Bash** — no magic, easy to audit.

## Structure

```
macbook-setup/
├── install.sh            # One-command installer (curl | bash)
├── bootstrap.sh          # Main entrypoint
├── config/               # Environment presets
├── scripts/              # Modular setup scripts
├── brew/                 # Brewfiles for packages
└── docs/                 # Documentation
```

## Usage

### Interactive (default)

```bash
./bootstrap.sh
```

### Non-interactive

```bash
./bootstrap.sh --role personal --git-name "Your Name" --git-email "your.email@example.com"
```

With a dotfiles repo:

```bash
./bootstrap.sh --role personal --git-name "Your Name" --git-email "your.email@example.com" \
  --dotfiles-repo "https://github.com/yourusername/dotfiles.git"
```

### Flags

| Flag | Description |
|------|-------------|
| `--role personal\|work` | Machine role |
| `--git-name <name>` | Git user name |
| `--git-email <email>` | Git user email |
| `--dotfiles-repo <url>` | Clone dotfiles from this Git repo |
| `--skip-brew` | Skip Homebrew installation |
| `--skip-macos` | Skip macOS settings |
| `--skip-dotfiles` | Skip dotfiles linking |
| `--skip-packages` | Skip package installation |
| `--dry-run` | Show what would be done |
| `--help` | Show usage |

## Role-Based Configuration

The `--role` flag controls which apps and settings get installed. This is useful if you have separate work and personal machines with different needs.

### How It Works

- **Common packages** (`brew/Brewfile.common`) are installed on all machines
- **Role-specific packages** are installed based on `--role personal` or `--role work`
- **Dock items** are configured differently for each role
- **macOS settings** apply regardless of role

### Work vs Personal

| | Personal | Work |
|---|---|---|
| **Extra packages** | `brew/Brewfile.personal` | `brew/Brewfile.work` |
| **Example apps** | Discord, personal tools | Azure CLI, work tools |
| **Dock items** | Messages, Notes, App Store | (none extra) |

### Customizing Roles

1. **Add packages**: Edit `brew/Brewfile.personal` or `brew/Brewfile.work`
2. **Add dock items**: Edit `scripts/dock.sh` (search for `MACHINE_ROLE`)
3. **Add env vars**: Edit `config/personal.env` or `config/work.env`

Example Brewfile customization:

```ruby
# brew/Brewfile.personal
cask "discord"
brew "spotify-tui"
```

```ruby
# brew/Brewfile.work
brew "azure-cli"
cask "docker"
```

## What It Does

1. Validates macOS
2. Asks for role, Git name, and email (if not provided)
3. Optionally clones your real dotfiles repo (via `--dotfiles-repo` or interactive prompt)
4. Installs Xcode Command Line Tools
5. Installs Homebrew
6. Installs packages via `brew bundle`
7. Configures Git
8. Creates `~/Developer`
9. Symlinks dotfiles from your repo (with backups for conflicts)
10. Applies macOS settings (Dock, Finder, Keyboard, Menu Bar)
11. Installs Kotlin and Android CLI tools
12. Runs postflight checks and prints summary

If any package fails to install, the run continues and the final summary
prints a `brew bundle --file ...` command to retry the failures. At the end
you'll be offered a restart to apply all system settings.

## Dotfiles

This repo does **not** ship fake dotfiles. Instead, it can optionally clone your real dotfiles repository:

- **Flag**: `--dotfiles-repo https://github.com/yourusername/dotfiles.git`
- **Interactive**: You'll be prompted for a repo URL during bootstrap (leave blank to skip)
- **Auto-link**: If your dotfiles repo follows the structure below, `bootstrap.sh` will symlink them automatically. If the repo has an `install.sh`, that will be run instead.

Expected dotfiles repo structure:

```
dotfiles/
├── zsh/zshrc              -> ~/.zshrc
├── git/gitignore_global   -> ~/.gitignore_global
├── ghostty/               -> ~/.config/ghostty
├── aerospace/aerospace.toml -> ~/.config/aerospace/aerospace.toml
├── nvim/                  -> ~/.config/nvim
├── starship/starship.toml -> ~/.config/starship.toml
└── yazi/                  -> ~/.config/yazi
```

If no dotfiles repo is provided, apps will use their default configs.

## Safety

- Existing files are backed up with a timestamp before being replaced.
- Scripts use `set -euo pipefail`.
- Steps are skipped if already applied.

## Manual Steps

See [docs/manual-steps.md](docs/manual-steps.md) for remaining items.

## License

MIT

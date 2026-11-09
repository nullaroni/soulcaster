# macbook-setup

A boring, rerunnable, inspectable macOS provisioning system for Apple Silicon Macs.

## MacFlayer (in progress)

The native Mac app replacing the Bash below. A Rust workspace, Apple Silicon only:

```text
crates/macflayer-core/   Catalog, Config parsing, Plan, Item states, event stream, command-runner seam
crates/macflayer-cli/    `macflayer`: validate a Config and print its plan
app/                      The Rune window, with a simulated engine for now
catalog/catalog.json      The launch Catalog
schema/                   The Config and Recipe JSON Schemas, with their examples
```

```sh
cargo test --workspace
cargo run -p macflayer-cli -- validate schema/examples/developer.json
cargo run -p macflayer-cli -- plan schema/examples/developer.json
cargo run -p macflayer-app                                    # the window
cargo run -p macflayer-app -- --list-scenes
cargo run -p macflayer-app -- --snap out.png --scene catalog-picked
```

None of this installs anything yet: the CLI only reads a Config, and the app plays a simulated Run.

### Build the app

```sh
packaging/build-app.sh               # target/package/MacFlayer.app, ad-hoc signed and checked
open target/package/MacFlayer.app
```

This needs Rust and Command Line Tools. [docs/releasing.md](docs/releasing.md) covers releases, signing and notarization.

## Install MacFlayer

On an Apple Silicon Mac with macOS 15 or later:

```bash
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/nullaroni/soulcaster/main/install.sh)"
```

**This doesn't install anything yet.** No notarized release exists, so the command stops with a message pointing to the source build above.

Once a notarized release exists, the command downloads it, checks its checksum, verifies it with `codesign` and `spctl` against the expected Apple team, copies it to `/Applications` and opens it. It works on a bare Mac, with no Git or Command Line Tools. It never asks for a password and refuses an app that fails verification, and nothing can skip that check. If the app is already installed, it verifies and opens it. To get a newer release, move the app to the Trash and run the command again.

## Quick Start (the older Bash setup)

The Bash setup below is being replaced by MacFlayer. `install.sh` no longer runs it, so start it from a clone:

```bash
git clone https://github.com/nullaroni/soulcaster.git ~/Developer/macbook-setup
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
├── install.sh            # Installs the MacFlayer app (verified, one command)
├── packaging/            # Builds and signs MacFlayer.app, and tests install.sh
├── bootstrap.sh          # Main entrypoint of the Bash setup
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

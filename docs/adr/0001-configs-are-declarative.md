# Configs are declarative, with no shell commands

Configs are meant to be shared and Imported from strangers, so a Config that could carry shell commands would effectively be a program running on the importer's Mac. We decided a Config can only name Catalog Items, settings values and Homebrew packages, and any custom install must become a Catalog recipe instead. This costs some flexibility, because people can't script one-off steps in their own Configs.

## Consequences

Declarative Configs don't remove all risk: third-party Homebrew taps, and a Dotfiles Repo, can still run code. A Dotfiles Repo is code even with no install script, because a linked file such as `~/.gitconfig` or `~/.zshenv` runs as soon as ordinary commands start. So Import shows a full preview and asks for separate approval of taps, the install script and the whole `dotfiles` block before a Run.

The Dotfiles Repo's approval is off by default unless the repo belongs to the signed-in GitHub account. It's tied to the repo and a commit SHA in the Machine's local record, and the app never pulls past that commit without a new preview. Links are applied only after the `sudo` helper is gone. The engine runs its own git with `GIT_CONFIG_GLOBAL=/dev/null` and `GIT_CONFIG_NOSYSTEM=1`, and its shells with `zsh -f`, so files already linked can't act on those calls.

# Configs are declarative, with no shell commands

Configs are meant to be shared and Imported from strangers, so a Config that could carry shell commands would effectively be a program running on the importer's Mac. We decided a Config can only name Catalog Items, settings values and Homebrew packages, and any custom install must become a Catalog recipe instead. This costs some flexibility, because people can't script one-off steps in their own Configs.

## Consequences

Declarative Configs don't remove all risk: third-party Homebrew taps and a Dotfiles Repo's install script can still run code, so Import shows a full preview and asks for separate approval of those before a Run.

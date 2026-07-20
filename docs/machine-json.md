# machine.json schema

Exported by the web configurator (`web/`) and consumed by `scripts/apply-config.sh`.

```json
{
  "version": 1,
  "generatedAt": "2026-07-20T20:00:00.000Z",
  "git": {
    "name": "Your Name",
    "email": "you@example.com"
  },
  "dotfiles": {
    "repo": "https://github.com/you/dotfiles.git"
  },
  "apps": ["neovim", "ghostty", "aerospace"],
  "settings": ["keyboard", "finder", "dock", "menu_bar", "macos"],
  "modules": ["xcode", "homebrew", "developer_dir", "git", "kotlin"]
}
```

Keep a copy of this file to restore your checklist via **Import JSON** on the site.

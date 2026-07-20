/** Catalog of apps, settings, and modules for macbook-setup. */
export const CATALOG_VERSION = 1;

export const PRESETS = [
  {
    id: "personal",
    label: "Personal",
    description: "Daily driver with messaging and Discord",
    apps: [
      "dockutil", "neovim", "aerospace", "eza", "fzf", "zoxide", "yazi", "starship",
      "gh", "jj", "opencode", "lua-language-server", "basedpyright",
      "typescript-language-server", "vscode-langservers-extracted", "maestro", "junie",
      "linearmouse", "ghostty", "jetbrains-toolbox", "discord",
    ],
    settings: ["keyboard", "finder", "dock", "menu_bar", "macos"],
    modules: ["xcode", "homebrew", "developer_dir", "git", "kotlin", "kotlin_lsp", "android"],
  },
  {
    id: "work",
    label: "Work",
    description: "Dev tooling plus Azure CLI",
    apps: [
      "dockutil", "neovim", "aerospace", "eza", "fzf", "zoxide", "yazi", "starship",
      "gh", "jj", "opencode", "lua-language-server", "basedpyright",
      "typescript-language-server", "vscode-langservers-extracted", "maestro", "junie",
      "linearmouse", "ghostty", "jetbrains-toolbox", "azure-cli",
    ],
    settings: ["keyboard", "finder", "dock", "menu_bar", "macos"],
    modules: ["xcode", "homebrew", "developer_dir", "git", "kotlin", "kotlin_lsp", "android"],
  },
  {
    id: "minimal",
    label: "Minimal",
    description: "Shell essentials and Ghostty only",
    apps: ["eza", "fzf", "zoxide", "starship", "gh", "ghostty"],
    settings: ["keyboard", "finder"],
    modules: ["xcode", "homebrew", "developer_dir", "git"],
  },
];

/** @typedef {{ id: string, name: string, description: string, category: string, brew?: string, cask?: string, tap?: string }} AppDef */

/** @type {AppDef[]} */
export const APPS = [
  { id: "dockutil", name: "dockutil", description: "CLI for Dock layout", category: "Utilities", brew: "dockutil" },
  { id: "neovim", name: "Neovim", description: "Modal text editor", category: "Editors", brew: "neovim" },
  { id: "aerospace", name: "AeroSpace", description: "i3-like tiling WM for macOS", category: "Desktop", cask: "nikitabobko/tap/aerospace", tap: "nikitabobko/tap" },
  { id: "eza", name: "eza", description: "Modern ls replacement", category: "CLI", brew: "eza" },
  { id: "fzf", name: "fzf", description: "Fuzzy finder", category: "CLI", brew: "fzf" },
  { id: "zoxide", name: "zoxide", description: "Smarter cd", category: "CLI", brew: "zoxide" },
  { id: "yazi", name: "Yazi", description: "Terminal file manager", category: "CLI", brew: "yazi" },
  { id: "starship", name: "Starship", description: "Cross-shell prompt", category: "CLI", brew: "starship" },
  { id: "gh", name: "GitHub CLI", description: "gh for PRs and issues", category: "Dev tools", brew: "gh" },
  { id: "jj", name: "Jujutsu", description: "Git-compatible VCS", category: "Dev tools", brew: "jj" },
  { id: "opencode", name: "OpenCode", description: "OpenCode CLI", category: "Dev tools", brew: "opencode" },
  { id: "lua-language-server", name: "Lua LS", description: "Lua language server", category: "Language servers", brew: "lua-language-server" },
  { id: "basedpyright", name: "BasedPyright", description: "Python type checker / LS", category: "Language servers", brew: "basedpyright" },
  { id: "typescript-language-server", name: "TS Language Server", description: "TypeScript LS", category: "Language servers", brew: "typescript-language-server" },
  { id: "vscode-langservers-extracted", name: "VS Code LSs", description: "HTML/CSS/JSON language servers", category: "Language servers", brew: "vscode-langservers-extracted" },
  { id: "maestro", name: "Maestro", description: "Mobile UI testing CLI", category: "Mobile", brew: "mobile-dev-inc/tap/maestro", tap: "mobile-dev-inc/tap" },
  { id: "junie", name: "Junie", description: "JetBrains Junie CLI", category: "Dev tools", brew: "junie", tap: "jetbrains/junie" },
  { id: "linearmouse", name: "LinearMouse", description: "Mouse & trackpad tuning", category: "Utilities", cask: "linearmouse" },
  { id: "ghostty", name: "Ghostty", description: "Fast GPU terminal", category: "Editors", cask: "ghostty" },
  { id: "jetbrains-toolbox", name: "JetBrains Toolbox", description: "Install JetBrains IDEs", category: "Editors", cask: "jetbrains-toolbox" },
  { id: "discord", name: "Discord", description: "Chat for communities", category: "Apps", cask: "discord" },
  { id: "azure-cli", name: "Azure CLI", description: "Microsoft Azure tooling", category: "Dev tools", brew: "azure-cli" },
];

export const SETTINGS = [
  { id: "keyboard", name: "Fast keyboard", description: "Fastest key repeat, disable press-and-hold accents" },
  { id: "finder", name: "Finder defaults", description: "List view, path bar, Documents home, ~/Developer favorite" },
  { id: "dock", name: "Dock layout", description: "Left side, autohide, clean Dock items" },
  { id: "menu_bar", name: "Menu bar", description: "Hide Spotlight, show Weather (best effort)" },
  { id: "macos", name: "Spaces & hot corners", description: "Disable auto-rearrange Spaces and hot corners" },
];

export const MODULES = [
  { id: "xcode", name: "Xcode CLT", description: "Command Line Tools (required for Homebrew)", recommended: true },
  { id: "homebrew", name: "Homebrew", description: "Package manager for apps below", recommended: true },
  { id: "developer_dir", name: "~/Developer", description: "Create the Developer directory", recommended: true },
  { id: "git", name: "Git identity", description: "Set git user.name and user.email", recommended: true },
  { id: "kotlin", name: "Kotlin", description: "Official Kotlin toolchain installer" },
  { id: "kotlin_lsp", name: "Kotlin LSP", description: "JetBrains kotlin-lsp for editors" },
  { id: "android", name: "Android CLI", description: "Official Android command-line tools" },
];

export function emptyConfig() {
  return {
    version: CATALOG_VERSION,
    git: { name: "", email: "" },
    dotfiles: { repo: "" },
    apps: [],
    settings: [],
    modules: ["xcode", "homebrew", "developer_dir", "git"],
  };
}

export function appsByCategory() {
  const map = new Map();
  for (const app of APPS) {
    if (!map.has(app.category)) map.set(app.category, []);
    map.get(app.category).push(app);
  }
  return map;
}

# Launch Catalog

**Throwaway prototype answering [#18](https://github.com/nullaroni/soulcaster/issues/18).** It isn't merged, and no app reads this file yet. Nothing was installed, and no `sudo` or `brew install` was run.

## Conclusion

The launch Catalog is **243 selectable Items in 15 sections**. Nothing is preselected. A person can scan a section, see the tools that overlap, and tick any of them. Homebrew, Command Line Tools, and `gh` are base Items. They are always installed, and they are not in the list.

Two rules do most of the organizing. A language's runtime, package managers, and version manager sit in one group, and the two polyglot version managers, mise and asdf, sit in their own group because each one covers many languages. An agentic editor, a terminal agent, a chat app, and a local model runner are four groups, so Cursor is not the same row as Cursor CLI, and Claude is not the same row as Claude Code.

Homebrew's published lists were downloaded at **2026-11-07T01:48:33Z** (the evening of 2026-11-06 in US Eastern): 8,704 formulae and 7,818 casks. `check.py` downloaded them again at the same time and passed. Tap files were read from GitHub the same night. Descriptions are editorial. Tokens, homepages, conflicts, bottles, and pkg or sudo evidence are copied from those files.

## Sections

| Section | Items | Why this cut |
|---|---:|---|
| Languages and runtimes | 34 | One current runtime per language, the package managers people actually tick, and the version managers that overlap them. |
| Browsers | 10 | The browsers people install next to Safari. Safari itself ships with macOS. |
| Editors and IDEs | 21 | Editors and the JetBrains IDEs, including the open-source IntelliJ and PyCharm casks beside the paid editions. |
| Agentic development | 16 | Editors built around an agent, terminal agents, the two chat apps those agents are confused with, and two local model apps. |
| Terminals and shells | 11 | Terminal apps, shells other than the system zsh, Starship, tmux, and Zellij. |
| CLI tools | 23 | The small set that shows up in most developer setups. |
| Containers and virtual machines | 13 | Engines that overlap each other, then VM apps. |
| Databases and API clients | 17 | Servers, desktop clients, and two API clients. |
| Git and code hosting | 13 | A newer Git, Git clients, and host CLIs other than `gh`. |
| Mobile development | 12 | Xcode, Android Studio, the SDK pieces, and the tools that target both. |
| Cloud and infrastructure | 24 | Provider CLIs, Kubernetes, infrastructure as code, and platform CLIs. |
| Networking | 8 | Tailscale and the tunnels and debuggers next to it. |
| Window management and utilities | 16 | Layout tools, launchers, menu bar and input, screenshots. |
| Communication and design | 15 | Chat, notes, design, and password managers. |
| Fonts | 10 | Monospaced fonts, plus one Nerd Font build. |

## How competing options are shown

The Catalog does not pick a winner and does not pre-select a stack. Overlap is stated on the Item, and Homebrew's own `conflicts_with` is copied into `conflictsWith` when it exists.

**Languages.** `node` installs Homebrew's current Node.js. `fnm`, `nvm`, and Volta install and switch Node.js versions. `pnpm` and Yarn install packages and still need some Node.js. `python` is Homebrew's current interpreter (`python` currently resolves to `python@3.15`). `pyenv` installs other Pythons. `uv` installs packages and can also install Pythons, so it overlaps both pyenv and Poetry. Poetry, pipx, and a few CLIs depend on `python@3.15` and say so. `mise` and `asdf` are the polyglot pair. `rust` and `rustup` are both listed; Homebrew marks `rustup` keg-only because it conflicts with `rust`. OpenJDK and Temurin are both listed. Kotlin, Scala, Gradle, and Maven depend on the `openjdk` formula.

**Containers.** The `docker` formula is the client. Docker Desktop, Colima, OrbStack, Podman, and Rancher Desktop are the engines. Homebrew records a cask conflict between Docker Desktop and Rancher Desktop only. It does not record the client as covered by the desktop app, so `coversPackages` is empty there.

**Agents.** The line used here: if the product's main window is an editor built around an agent, it is an agentic editor (Cursor, Zed, opencode Desktop, Devin). If it is a terminal program, it is an agent CLI (Claude Code, Codex, opencode, Junie, GitHub Copilot CLI, Cursor CLI, aider, Goose). JetBrains IDEs stay under Editors. Junie, the CLI, is under Agent CLIs. Claude and ChatGPT are chat apps, in their own group, because they are easy to tick instead of the coding agent.

**Git.** Command Line Tools already include Git. The `git` Item is Homebrew's newer Git. `gh` is not listed.

**Data stores that conflict in Homebrew.** MySQL conflicts with MariaDB and with Percona Server. Redis conflicts with Valkey. MariaDB and Valkey are both listed. Percona Server is not.

## Selection rules

- IDs are lowercase kebab-case product names. They do not include a category or an install method. `tailscale` installs the `tailscale-app` cask. `jetbrains-mono` installs `font-jetbrains-mono`. An ID is never reused; this file is the first assignment of these IDs.
- Official Homebrew is used when it has the product. A third-party tap is used only when official Homebrew has no equivalent, and `tap` names it so Import can ask for approval.
- A candidate was dropped if the live formula or cask was `deprecated` or `disabled`, if `caveats_rosetta` was set, or if a formula had no Apple Silicon bottle and a cask had no `arm64_*` platform.
- Vendor `curl | sh` installers are not used. Where Homebrew also publishes the tool, the Catalog uses Homebrew. Where that was the only install, the tool is listed under exclusions.
- `needsFromPerson` is an admin password when a cask artifact includes a `pkg` or sets `sudo`, taken from the cask JSON. OpenJDK's caveat and yabai's caveat are the two formula cases, and each says so. Xcode's Apple Account is from the tool-installs research. The Android SDK license and Xcode's license step are from the item-lifecycle prototype. Tap approval is the map's rule for third-party taps.
- `opensAfter` is only filled when a Homebrew caveat or a research note in this repo names a first-open step. It does not stop a Run.
- `conflictsWith` is Homebrew's `conflicts_with`, plus two extras that the conflict list does not carry: Tailscale's installer stops if the `tailscale` formula is present (tool-installs research), and `rustup` is keg-only because it conflicts with `rust`.
- `coversPackages` is empty on every Item. Nothing in the conflict data said one selected Item installs another Item's files.
- License `open-source` is used when Homebrew publishes an SPDX license, or when the cask text or a fetched page says open source. `free` and `freemium` are used only where a page or a cask name said so. Other casks stay `unverified`, with a `pricingPage` when a page returned HTTP 200 and the plan was not classified.

## Exclusions

**Base Items, omitted on purpose.** Homebrew, Command Line Tools, and `gh`.

**Deprecated or disabled in the live data.**

| Token | What the data says |
|---|---|
| `arc` | cask deprecated, discontinued |
| `chromium` | cask disabled, `fails_gatekeeper_check` |
| `alacritty` | cask disabled, `fails_gatekeeper_check` |
| `intellij-idea-ce` | cask deprecated, discontinued. `intellij-idea` and `intellij-idea-oss` are included |
| `pycharm-ce` | cask deprecated, discontinued. `pycharm` and `pycharm-oss` are included |
| `gemini-cli` | formula deprecated, unsupported |
| `fleet` | cask disabled, `no_longer_available` |
| `copilot` | formula deprecated, `repo_archived`. Its description is an Amazon ECS tool. GitHub Copilot CLI is the `copilot-cli` cask, which is included |

**No official formula or cask, and not given a `curl \| sh` installer.** Oh My Zsh, SDKMAN, Windsurf, Codeium, Amazon Q, and VMware Fusion. A search of the downloaded JSON found no token for these names. Windsurf may have been renamed or removed; that was not established.

**Installable, and left out by judgment.** These exist, are not deprecated, and were still cut so the list stays scannable.

- Lima, because Colima is the Item people tick and Lima is the VM under it.
- Finch, a container tool from AWS, beside the engines already listed.
- The `consul` cask. Consul is real. It is further into operations than Terraform, Packer, and Vault, which are included. Nomad, Boundary, and Waypoint were not in official Homebrew.
- Hyper. The cask is live. Ghostty, iTerm2, Warp, kitty, and WezTerm are the terminals in the list.
- Setapp, because it is a subscription bundle rather than one tool.
- `xcodes` and `xcodes-app`. The item-lifecycle prototype installs Xcode from the App Store, because `xcodes` takes an Apple password.
- `powershell@preview`. The stable `powershell` formula is the Item.
- The homebrew-cask token `maestro`. That cask is a different product, at runmaestro.ai. Mobile.dev's Maestro is the tap formula.
- The `tailscale` formula, the `ollama` formula, and the `wireshark` formula. Each has a desktop cask in the Catalog. The Tailscale formula also trips the Standalone installer's conflict check.
- iOS Simulator and the Android emulator as their own Items. There is no Homebrew token for either. Xcode downloads simulators. Android Studio or `sdkmanager` installs the emulator.
- macOS settings. Those are Catalog Items too, and ticket #6 owns them. This file is the software list.
- Lua, Julia, Haskell, OCaml, R, and Clojure. They are real and, where checked, bottled. They are a narrower audience than the languages included.
- Trae, Kiro, Kiro CLI, Google Antigravity IDE, Google Antigravity CLI, Qwen Code, and Devin CLI. Each has a live Homebrew token and was left out so the agent section stays scannable. Adding them later does not reuse an ID.
- Railway, `doctl`, ClickHouse, Insomnia, Yaak, Beekeeper Studio, tig, Multipass, GNU findutils, and the long tail of Nerd Fonts. Inconsolata, Victor Mono, and Commit Mono were the closest font cuts. CaskaydiaCove is the one Nerd Font included.
- WhatsApp, Mattermost, and Mimestream.

**Intel-only and Rosetta.** Any candidate with `caveats_rosetta`, or without an Apple Silicon bottle or `arm64_*` platform, was dropped. The disabled Chromium and Alacritty casks are the ones that came up by name. This was not a census of all 7,818 casks.

## What was checked by hand outside the JSON

These are not in `formula.json` or `cask.json`. `check.py` only confirms the formula file is still published.

| Item | What was read |
|---|---|
| AeroSpace | `nikitabobko/homebrew-tap` cask. `file` on the 0.21.3-Beta zip showed a universal Mach-O with arm64. |
| Maestro | `mobile-dev-inc/homebrew-tap` formula, Apache-2.0, one zip. The launcher is a shell script and the payload is jars. It depends on OpenJDK. |
| Tart | `cirruslabs/homebrew-cli` formula, license line `Fair Source`. `file` on the 2.32.1 tarball showed a universal Mach-O with arm64. |
| Terraform, Packer, Vault | `hashicorp/homebrew-tap`. Each formula has a `darwin_arm64` URL. The file's SPDX header was not treated as the product license. |
| MongoDB Community | `mongodb/homebrew-brew`. The formula selects a `mongodb-macos-arm64` archive. It has no license line. |
| yabai | `asmvik/homebrew-formulae` (the old `koekeishiya/homebrew-formulae` URL redirects here). `file` on the 7.1.25 tarball showed a universal Mach-O with arm64. The caveat documents a sudoers entry for the scripting-addition. It does not mention SIP, so this Catalog does not either. |

Xcode is not a Homebrew cask. The App Store listing fetched the same night says Free. Apple Silicon support is taken from the tool-installs note that the listing requires M1 or later. The binary was not downloaded.

## Validation

```bash
python3 docs/prototypes/catalog/check.py
```

Run at 2026-11-07T01:48:33Z:

```text
catalog /Users/nullaroni/.t3/worktrees/macbook-setup/t3-7054ad6e/docs/prototypes/catalog/catalog.json
live fetch started 2026-11-07T01:48:33Z
formulae 8704 casks 7818 bytes 32227397 19381750
items 243
  languages              34
  browsers               10
  editors                21
  agents                 16
  terminals              11
  cli                    23
  containers             13
  databases              17
  git                    13
  mobile                 12
  cloud                  24
  networking             8
  window-management      16
  communication          15
  fonts                  10
PASS
```

The script checks that every official token still exists and is not deprecated or disabled, that every ID is unique and lowercase kebab-case, that the base Items are absent, that Apple Silicon evidence still holds, that recorded `conflicts_with` entries match the live lists, that pkg or sudo admin flags match the live artifacts, and that each tap file still contains the expected text.

## Open questions

1. **Settings.** Dock, Finder, keyboard, and the rest of ticket #6 are Catalog Items. They are not in this file. They need a section of their own before the picker is complete.
2. **Docker Desktop and the Docker client.** Homebrew does not say the cask conflicts with, or contains, the `docker` formula. Whether the desktop app should mark the client as covered is still open. `coversPackages` is empty everywhere for that reason.
3. **How fresh the agent casks are.** Claude Code, Codex, opencode, and Junie use Homebrew because the lifecycle prototype refuses `curl | sh`. The earlier tool-installs note found Junie's tap lagging the vendor release. This pass did not compare cask versions with the vendor release channels.
4. **opencode's two Homebrew installs.** The Catalog uses the homebrew-core formula. The tool-installs note also describes `anomalyco/tap/opencode-v2` as the more current one. That comparison was not repeated today.
5. **Windsurf.** No token in the live JSON. If the product was renamed, the new token was not identified.
6. **Paid, free, and freemium.** Most casks have no license field. Only a few models are filled, where a cask name or a fetched page was explicit: Bruno and the JetBrains OSS casks as open source, Xcode, DBeaver Community, and Burp Suite Community Edition as free, and Linear, Notion, and Raycast as freemium. JetBrains Ultimate and PyCharm Professional are named that way because those are the cask names. A price was not copied.
7. **Simulators.** iOS simulators and the Android emulator are steps inside Xcode and the Android SDK, not rows. They can become Items if the picker needs to show them on their own.
8. **First open.** Sign-in for Claude Code, Codex, opencode, and Junie, Tailscale's system extension, AeroSpace's Accessibility prompt, and OrbStack's "open the app" caveat are recorded. Rectangle, Raycast, Karabiner-Elements, and Docker Desktop's privileged helper were not in the caveats or the research notes, so they are not recorded. That does not mean those prompts are absent.

# How Tailscale, gh, the agent CLIs and full Xcode install and sign in

Research for [ticket #7](https://github.com/nullaroni/macbook-setup/issues/7), checked on 2026-11-06 against each vendor's docs, source code, Homebrew metadata and published installers.

**Nothing was installed or signed in to.** Installers were downloaded to a temp folder and read, and `.pkg` files were inspected with `pkgutil` and `spctl`. Behaviour described below comes from reading those files, not from running them. Each tool's section ends with what is inferred or still unverified.

## Short answer

Every tool except full Xcode can install without a person, given admin rights where noted. None of them can finish sign-in without a person, short of a pre-made secret, so every sign-in belongs in the Needs-You Queue. Full Xcode needs a person before its download can start, and Tailscale also needs macOS approvals unless the Machine is under MDM.

| Item | Vendor-recommended install | Unattended install? | Installed check | Signed-in check | Sign-in needs |
|---|---|---|---|---|---|
| Tailscale | Standalone `.pkg`, also available as the `tailscale-app` cask | Files yes, with admin. Activation no: a person approves it in System Settings unless MDM pre-approves it | Bundle ID `io.tailscale.ipn.macsys` | `tailscale status --json`, field `BackendState` | Browser sign-in to the tailnet's identity provider |
| `gh` | `brew install gh` | Yes | `brew list --formula gh` | `gh auth status --json hosts` | GitHub account; browser plus a one-time code |
| Claude Code | `curl -fsSL https://claude.ai/install.sh \| bash` | Yes via the cask. The script's non-terminal behaviour is untested | `~/.local/bin/claude` or the cask | `claude auth status`: JSON, exit 0 or 1 | Paid Claude plan or Console account; browser |
| Codex | `curl -fsSL https://chatgpt.com/codex/install.sh \| sh` | Yes, with `CODEX_NON_INTERACTIVE=1` or via the cask | `~/.local/bin/codex` or the cask | `codex login status`: exit 0 or 1 | ChatGPT plan with browser sign-in, or an API key |
| opencode | `curl -fsSL https://opencode.ai/v2/install \| bash`, or the `anomalyco/tap/opencode-v2` formula | Yes | `~/.opencode/bin/opencode` or the formula | `opencode auth list --format json` | Depends on provider; local models need no account |
| Junie | `curl -fsSL https://junie.jetbrains.com/install.sh \| bash` | Yes | `~/.local/bin/junie` | **No documented check** | JetBrains account in a browser, an API key, or a provider key |
| Full Xcode | Mac App Store | **No** | Bundle ID `com.apple.dt.Xcode` | **No documented check** of the App Store or Xcode account | Apple Account, always, for either `mas` or `xcodes` |

## Tailscale

### Distributions

Tailscale [says](https://tailscale.com/docs/concepts/macos-variants): "We always recommend installing the Standalone variant". It also warns: "Do not install the Mac App Store variant and the Standalone variant on the same machine."

| | Standalone | Mac App Store | Open source `tailscaled` |
|---|---|---|---|
| Suitable for this app? | **Yes, the vendor default** | Poor: needs an Apple ID and has more limits | Only for advanced users |
| Install | `.pkg` or `brew install --cask tailscale-app` | App Store | `brew install --formula tailscale`, or `go install` |
| Apple ID | No | Yes | No |
| GUI | Yes | Yes | No |
| Runs before login | No | No | Yes |
| Auto-update | Yes, in-app (Sparkle) | Yes, App Store | No |
| Exit nodes | Yes | Yes | Can advertise, can't use |
| Tailscale SSH server | No | No | Yes |
| `tailscale ssh` command | Yes | No | Yes |
| Screen Time web filter | Compatible | Not compatible | Compatible |
| Open source | No | No | Yes |

All three need macOS 13 or later. The [wiki page for `tailscaled`](https://github.com/tailscale/tailscale/wiki/Tailscaled-on-macOS) says it is "less tested" and that users must set `100.100.100.100` as their DNS server themselves for MagicDNS. Tailscale recommends it "only" for unattended installs managed by experienced administrators. The Homebrew formula runs it as a root service via `sudo brew services start tailscale`.

The [cask](https://formulae.brew.sh/api/cask/tailscale-app.json) installs the Standalone pkg, currently 1.104.5. A cask named `tailscale` returned 404 from the Homebrew API.

### Install

- **Package:** `https://pkgs.tailscale.com/stable/Tailscale-latest-macos.pkg`. It is signed with a `Developer ID Installer: Tailscale Inc.` certificate and notarized, and `spctl` accepted it.
- **Pre-install script:** it quits running copies. If it finds the App Store copy, it shows a dialog and waits for a button press. If it finds the Homebrew formula, it shows another. **Check for both before installing**, or a Run can stall on a dialog.
- **Post-install script:** it writes a `/usr/local/bin/tailscale` wrapper if none exists, copies man pages, then launches the app with `open -a`.
- **Approvals:** on first launch the app installs a VPN configuration. For the system extension, [Tailscale says](https://tailscale.com/kb/1286/macos-mdm) "By default, macOS blocks this, and requires the user to manually approve Tailscale in the System Settings app."
- **Pre-approval:** the docs describe it only through MDM, because "a payload with system scope can only be deployed by an MDM server". `tailscale configure sysext activate` and `tailscale configure mac-vpn install` [exist](https://tailscale.com/docs/reference/tailscale-cli), but the docs don't say they avoid the prompts.

### Detect

- **Standalone:** bundle ID `io.tailscale.ipn.macsys`, or `pkgutil --pkg-info com.tailscale.ipn.macsys` (the package ID comes from the cask).
- **App Store:** bundle ID `io.tailscale.ipn.macos`.
- **`tailscaled`:** `brew list --formula tailscale`, or `/Library/LaunchDaemons/com.tailscale.tailscaled.plist`.
- **CLI path:** `/Applications/Tailscale.app/Contents/MacOS/Tailscale`. Set `TAILSCALE_BE_CLI=1` when calling it from a script, as the docs advise.
- **Extension state:** `tailscale configure sysext status`.
- **Sign-in state:** `tailscale status --json`, field `BackendState`. The [source](https://github.com/tailscale/tailscale/blob/main/ipn/backend.go) lists `NoState`, `InUseOtherUser`, `NeedsLogin`, `NeedsMachineAuth`, `Stopped`, `Starting` and `Running`. The CLI docs warn "format subject to change", so this needs a test against each pinned version.

### Sign in

- The default is a browser sign-in with an SSO identity provider, through the app's onboarding, `tailscale up` or `tailscale login`.
- [Auth keys](https://tailscale.com/kb/1085/auth-keys) register a device without a browser via `tailscale up --auth-key=…`. Only an Owner, Admin, IT admin or Network admin can generate one, and keys last 1 to 90 days. A key is a secret, so it can't go in a Config.
- A device sits in `NeedsMachineAuth` when the tailnet requires device approval and an admin hasn't given it yet (inferred from the state name).

### Updates

Standalone updates through a **Settings** option, "Automatically install updates". [Tailscale says](https://tailscale.com/kb/1067/update) devices can opt in or out at any time, but doesn't state the per-device default. New tailnets have auto-updates enabled by default. This may conflict with the map's rule that updates run only when the user chooses.

### Inferred or unverified

- **Possible missing wrapper (inferred):** the post-install script doesn't create `/usr/local/bin`, so the wrapper may not appear on a bare Mac. The docs still describe a separate "CLI integration" setting. Use the app-bundle path.
- **Unverified:** whether `--auth-key` works with Standalone. The docs list it with no variant restriction and don't confirm it.
- **Unverified:** what `tailscale status --json` returns before the extension is approved.
- **Unverified:** whether `open -a` in the post-install script behaves the same when the pkg is installed by Homebrew or by `sudo installer`.

**Page dates ("Last validated"):** variants, install and MDM Jan 5 2026; CLI Jul 30 2026; `tailscale up` Jan 26 2026; auth keys Jun 30 2026; system policies Sep 29 2026; updates Dec 4 2025. The wiki was last edited Aug 26 2025. Release 1.104.5 was published Nov 3 2026.

## gh

### Install

- [Official methods](https://github.com/cli/cli/blob/trunk/docs/install_macos.md) are Homebrew and precompiled release binaries. MacPorts, Conda, Spack, Flox and Webi are community-run.
- `brew install gh` gets 2.103.3 from homebrew-core.
- **The `.pkg` is unsigned.** The docs say it is unsigned "as of May 29th" with no year. The current `gh_2.103.3_macOS_universal.pkg` confirms it: `pkgutil` reports "no signature" and `spctl` rejects it. It installs to `/usr/local/bin/gh`.

### Detect

- **Installed:** `brew list --formula gh`, or the binary at `$(brew --prefix)/bin/gh`.
- **Signed in:** [`gh auth status`](https://cli.github.com/manual/gh_auth_status) exits 1 on any authentication problem. With `--json hosts` it "will always exit with zero", so a script must parse the JSON. Add `--hostname github.com` to limit the check to one host.
- **Masking:** `GH_TOKEN` and `GITHUB_TOKEN` take precedence over stored credentials and can hide the real state.

### Sign in

- **Web flow:** `gh auth login --web --hostname github.com --git-protocol https` uses a one-time code.
- **Without a terminal:** the [source](https://github.com/cli/cli/blob/trunk/internal/authflow/flow.go) prints the code, then "Open this URL to continue in your web browser", rather than waiting for Enter. The app could show both to the user (inferred; not run).
- **Scopes:** the minimum is `repo`, `read:org` and `gist`. Add others with `--scopes`.
- **Storage:** the [system credential store](https://cli.github.com/manual/gh_auth_login), falling back to a plain-text file if that fails.
- **Git:** `gh auth setup-git` makes `gh` the credential helper. `--git-protocol ssh` also prompts about an SSH key unless `--skip-ssh-key` is passed.
- **Token option:** `--with-token` reads a token from stdin. It needs a secret, which the map keeps out of Configs.

**Dates:** the pages show none. The install doc was last committed Jul 2 2026, the manual's `Last-Modified` header is Nov 3 2026, and 2.103.3 was released Oct 26 2026.

## Claude Code

### Install

[Source](https://code.claude.com/docs/en/setup). Requires macOS 13+ and 4 GB+ RAM. Git is optional.

| Method | Location | Updates |
|---|---|---|
| **Native script (recommended)** | `~/.local/bin/claude`, a symlink into `~/.local/share/claude/versions/` | Automatic, in the background. `DISABLE_AUTOUPDATER=1` turns this off |
| `brew install --cask claude-code` | Homebrew prefix | Manual by default. Stable channel, about a week behind (2.1.311) |
| `brew install --cask claude-code@latest` | Homebrew prefix | Manual by default. Latest channel (2.1.318) |
| `npm install -g @anthropic-ai/claude-code` | npm global | Needs Node.js 22+. Never use `sudo` |

- The script refuses to run under `sudo`, checks SHA-256 against a release manifest, then runs the downloaded binary's `install` command.
- It doesn't change `PATH`. It prints a fix if `~/.local/bin` is missing.
- The macOS binary is signed by Anthropic PBC and notarized, according to the docs.
- `CLAUDE_CODE_PACKAGE_MANAGER_AUTO_UPDATE=1` opts Homebrew installs into auto-update.

### Detect

- **Installed:** `~/.local/bin/claude` or the cask. Avoid keeping two installs; the docs advise keeping one.
- **Signed in:** [`claude auth status`](https://code.claude.com/docs/en/cli-reference) prints JSON and "Exits with code 0 if logged in, 1 if not". `authMethod` is `none`, `claude.ai`, `oauth_token`, `api_key`, `api_key_helper` or `third_party`.
- **Masking:** [environment variables](https://code.claude.com/docs/en/authentication) such as `ANTHROPIC_API_KEY` outrank a stored sign-in and can change the result.

### Sign in

- `claude auth login`, with optional `--email`, `--sso` or `--console`. The first run of `claude` also starts it.
- It opens a browser. If the browser shows a code, the user pastes it into the terminal.
- **Accounts:** Pro, Max, Ultra, Team, Enterprise or Console, or a cloud provider such as Bedrock, Google Cloud or Microsoft Foundry. "The free claude.ai plan does not include Claude Code access."
- **Storage:** the macOS Keychain, falling back to `~/.claude/.credentials.json` (mode 0600) if the write is rejected.
- `claude setup-token` creates a one-year token but still uses a browser, and the token can only make model requests.

### Inferred or unverified

- **Untested:** whether the binary's `install` step works without a terminal.
- **Untested:** whether `claude auth login` works from a process with no terminal, and whether reading the Keychain from the app triggers a prompt.

**Dates:** the pages show none, and their `Last-Modified` header equals the request time, so it isn't an edit date.

## Codex

### Install

[`developers.openai.com/codex`](https://developers.openai.com/codex/cli) now redirects to `learn.chatgpt.com`. The [README](https://github.com/openai/codex/blob/main/README.md) lists three methods:

- `curl -fsSL https://chatgpt.com/codex/install.sh | sh`
- `npm install -g @openai/codex`
- `brew install --cask codex` (0.167.0)

What the [script](https://chatgpt.com/codex/install.sh) does:

- It installs to `~/.local/bin`, or to `CODEX_INSTALL_DIR` if set.
- It verifies SHA-256.
- On zsh on macOS, it adds a marked `PATH` block to `~/.zprofile`.
- It asks "Start Codex now?" when a terminal is available. `CODEX_NON_INTERACTIVE=1` skips prompts, and it answers no when there is no terminal.
- It warns when it finds another managed install.

Updates are not automatic: rerun the installer, `brew upgrade --cask codex`, or [`codex update`](https://developers.openai.com/codex/cli/reference) "when the installed release supports self-update". The docs state no system requirements on the pages read.

### Detect

- **Installed:** `~/.local/bin/codex` or the cask.
- **Signed in:** `codex login status`. The [source](https://github.com/openai/codex/blob/main/codex-rs/cli/src/login.rs) exits 0 when logged in and 1 with "Not logged in" or on error. It writes to stderr and has no JSON output, so use the exit code. The docs only mention the exit-0 case.

### Sign in

[Source](https://developers.openai.com/codex/auth).

- **Browser:** `codex login` opens a browser, and a localhost callback returns the token. The README recommends a ChatGPT Plus, Pro, Business, Edu or Enterprise plan.
- **Device code (beta):** `codex login --device-auth`, for when the callback is blocked. It must first be enabled in ChatGPT security settings or, for a workspace, by an admin.
- **Stdin options:** `--with-api-key` and `--with-access-token` need a secret.
- **API-key limits:** it bills at API rates, and some features that rely on ChatGPT workspace access or cloud services are limited or unavailable.
- **Storage:** `cli_auth_credentials_store` can be `file`, `keyring`, `auto` or `ephemeral`. The docs don't state a default. The [source enum](https://github.com/openai/codex/blob/main/codex-rs/config/src/types.rs) defaults to `file`, meaning `~/.codex/auth.json`.

### Inferred or unverified

- **Inferred:** the default store is `file`, based only on that enum's default. Not checked by running it.

**Dates:** the pages show none. The `Last-Modified` header on the two doc pages is Nov 6 2026, probably a build time. The README was last committed Jul 23 2026. Release 0.167.0 was published Nov 6 2026.

## opencode

### Install

[Source](https://opencode.ai/docs/). V1 and V2 both exist, so the exact name matters:

| Command or source | What you get | Notes |
|---|---|---|
| `curl -fsSL https://opencode.ai/v2/install \| bash` | **V2** | The URL in the current docs |
| `curl -fsSL https://opencode.ai/install \| bash` | **V1** | Older URL; the script resolves a `v1.x` tag and points users to V2 |
| `brew install anomalyco/tap/opencode-v2` | **V2** 2.2.5 | README: "recommended, always up to date". Conflicts with `opencode` |
| `brew install opencode` | **V2** 2.2.5 today | homebrew-core. README: "updated less". This repo's `Brewfile.common` uses it |
| `brew install anomalyco/tap/opencode` | **V1** 1.19.5 | |
| `npm install -g @opencode/cli` | V2 | A postinstall script selects the binary |
| `brew install --cask opencode-desktop` | Desktop app (beta) 2.2.5 | Separate install; the cask is marked as auto-updating |

GitHub's "latest" release is **v1.19.5**, so it can't be used to work out the current V2 version.

The V2 script installs to `$HOME/.opencode/bin` and appends to `PATH` in `.zshrc`, creating that file on a new Mac. `--no-modify-path` turns this off. Both Homebrew formulae depend on `ripgrep`. `opencode upgrade`, alias `update`, handles updates.

### Detect

- **Installed:** `~/.opencode/bin/opencode` or the formula.
- **Signed in:** [`opencode auth list --format json`](https://opencode.ai/docs/cli/commands/). Connections from environment variables appear with type `environment`, so they don't prove a saved account.
- **Exit codes:** not documented, so parse the JSON.

### Sign in

[Source](https://opencode.ai/docs/cli/providers/).

- **Optional to install.** Most providers need an account before use. Local Ollama is discovered without one.
- **CLI:** `opencode auth login [provider] [--method …]`, or `/connect` in the TUI.
- **Methods:** API key, OAuth (browser URL, device code or authorization code), a provider-supplied command, or an environment variable. ChatGPT Plus or Pro and GitHub Copilot sign in without an API key.
- **Terminal needed:** "API-key entry and provider forms require an interactive terminal", as do OAuth methods that ask for a pasted code.
- **Storage:** SQLite in V2, usually `~/.local/share/opencode/opencode.db`. V2 imports the legacy `auth.json` once and doesn't write back to it. Print the path with `opencode debug paths db`. The docs say not to edit the database.

### Inferred or unverified

- **Inferred:** `opencode` commands may start its shared background server, since the docs say it "discovers or starts" one. A detection call could therefore leave a process running. Not tested.
- **Unverified:** Claude Pro or Max login. The [V2 pages read](https://opencode.ai/docs/providers/) don't mention those plans; `--method key` is the documented Anthropic example. A third-party guide says the built-in Anthropic OAuth was removed in March 2026. No official source confirmed that.
- **Unverified:** auto-update behaviour. The V2 pages read mention none, and I didn't check V1 docs.
- **Partly checked:** a search of the V2 script for `sha256` and `shasum` found nothing, but I didn't read the whole script, so it may verify downloads some other way.

**Dates:** the pages show none. The README was last committed Nov 6 2026.

## Junie

### Install

[Source](https://junie.jetbrains.com/docs/junie-cli.html). Junie is available on macOS, Linux and Windows.

- **Script:** `curl -fsSL https://junie.jetbrains.com/install.sh | bash`.
  - It installs a shim at `~/.local/bin/junie` and builds under `~/.local/share/junie/versions`.
  - It verifies SHA-256 when a checksum is published, and warns and continues if one isn't found for a pinned `JUNIE_VERSION`.
  - It appends `PATH` to `~/.zshrc`, or `~/.zprofile` as a fallback, unless one already mentions `.local/bin`.
  - A search found no prompts in it.
- **Homebrew is an alternative, but lags.** The [tap formula](https://github.com/JetBrains/homebrew-junie/blob/main/Formula/junie.rb) was at **3110.7**, last changed Sep 9 2026. The newest [release-channel build](https://github.com/JetBrains/junie/releases) was **3959.5**, from Nov 3 2026.
- **The formula is only a bootstrap.** On first run it copies files to `~/.local/share/junie`, writes `~/.local/bin/junie`, and hands off to that shim from then on.
- **npm:** `npm install -g @jetbrains/junie`.

**Updates:** Junie checks and downloads on startup. `--skip-update-check` disables this per run, and `junie update` fetches a build for the next start. The [CLI reference](https://junie.jetbrains.com/docs/parameters.html) and [environment variable list](https://junie.jetbrains.com/docs/environment-variables.html) show no persistent setting.

**Status conflict:** the [headless page](https://junie.jetbrains.com/docs/junie-headless.html) says "currently in an Early Access Program", while a [March 2026 JetBrains post](https://blog.jetbrains.com/junie/2026/03/junie-cli-the-llm-agnostic-coding-agent-is-now-in-beta/) says Beta. The tap formula's licence link points to EAP terms.

### Detect

- **Installed:** `~/.local/bin/junie`.
- **Signed in: no documented way.** The CLI reference has no auth status, whoami or login command, and `/account` and `/usage` work only inside a session. Credential storage isn't documented. Only environment keys can be seen from outside.

### Sign in

- On first run, the welcome screen offers options. "Continue with JetBrains account" redirects to a browser. `/account` changes it later.
- **API key:** generated at `junie.jetbrains.com/cli`, set as `JUNIE_API_KEY` or `--auth`, with usage-based billing.
- **Provider keys:** `JUNIE_ANTHROPIC_API_KEY`, `JUNIE_OPENAI_API_KEY`, `JUNIE_GOOGLE_API_KEY` and others. The docs say this works "without requiring a JetBrains AI subscription".
- A JetBrains account login uses "your subscription plan with JetBrains".

### Inferred or unverified

- **Inferred:** once the shim takes over, the running version can differ from what `brew list` reports, so Homebrew data can't be trusted for Drift on this Item.
- **Inferred:** repo `jetbrains/junie` and docs `jetbrains-junie/junie` reach the same tap. The GitHub API resolves the latter to `JetBrains/homebrew-junie`. Not tested with `brew tap`.
- **Unverified:** the default update channel and hardware requirements.

**Dates:** each docs page ends "06 November 2026", today's date and probably a build date. The README was last committed Nov 5 2026.

## Full Xcode

Apple [says](https://developer.apple.com/xcode/resources/) "Download Xcode from the Mac App Store". Other versions come from its downloads page: "simply sign in to your Apple Account. Apple Developer Program membership isn't required." That page [redirects to Apple's sign-in](https://developer.apple.com/download/applications/).

The [App Store listing](https://apps.apple.com/us/app/xcode/id497799835) shows Xcode 27.2, dated Oct 30, free, 3.1 GB, needing macOS 26.6 or later and an M1 or later. A Machine on older macOS may need a macOS update first, or an older Xcode from [Apple's table](https://developer.apple.com/support/xcode/).

### Options

| | `mas` | `xcodes` | App Store by hand |
|---|---|---|---|
| Version | [7.0.0](https://github.com/mas-cli/mas), homebrew-core | [2.1.0](https://github.com/XcodesOrg/xcodes), tap and homebrew-core | Current listing |
| Install tool | `brew install mas` | `brew install xcodesorg/made/xcodes` (README's preferred method). Core formula needs macOS 15+ | None |
| Command | `mas get 497799835` | `xcodes install --latest --select --empty-trash` | Click Get |
| Apple Account | Must already be signed in to the App Store | Username and password, plus 2FA, given to the tool | Signed in to the App Store |
| Can it sign in for you? | **No.** There's no `signin` command, only `signout` | Yes, via prompt or `XCODES_USERNAME` and `XCODES_PASSWORD` | No |
| Extra authentication | Touch ID or password per app, [depending on Mac settings](https://github.com/mas-cli/mas#app-store-apple-account-requirements-for-get) | 2FA challenge | The App Store's own prompts |
| Root | Needed; uses sudo credentials or prompts | Needed to finish; `--no-superuser` skips those steps | Not applicable |
| Several versions side by side | No | Yes; can also install simulators with `xcodes runtimes` | No |
| Post-install steps | Not run; must be done separately | Runs them, listed below | Xcode prompts on first launch |
| Apple's own? | Not an Apple tool, but installs through the App Store | Not an Apple tool; depends on Apple's sign-in endpoints | Yes |

`xcodes` details:

- **Fragility:** the 2.1.0 notes (Sep 14 2026) read "Apple has changed how they log in (again)".
- **2FA:** [its login library](https://github.com/XcodesOrg/XcodesLoginKit) handles code, SMS and security-key challenges, plus a browser callback for federated accounts.
- **Automation:** [issue #326](https://github.com/XcodesOrg/xcodes/issues/326) reports an endless 2FA prompt in CI even with the variables set. It is open, last updated Dec 2024.
- **Password storage:** it saves the Apple ID password in the keychain.
- **Terminal:** it reads the macOS password from a terminal prompt unless sudo credentials are already valid.
- **Tension with the map:** the map says sign-ins go through the browser. A non-federated `xcodes` sign-in doesn't.

### After the download

From [xcodes' source](https://github.com/XcodesOrg/xcodes/blob/main/Sources/XcodesKit/Environment.swift), these run under sudo:

- `DevToolsSecurity -enable`
- `dseditgroup -o edit -t group -a staff _developer`
- `xcodebuild -license accept`
- `xcodebuild -runFirstLaunch`

[Apple documents](https://developer.apple.com/documentation/xcode/downloading-and-installing-additional-xcode-components) `xcode-select -s <path>` followed by `xcodebuild -runFirstLaunch`, and `xcodebuild -downloadPlatform iOS` for simulators.

### Detect

- **Installed:** `mdfind "kMDItemCFBundleIdentifier == 'com.apple.dt.Xcode'"`. `mas list --json` also works, but [needs Spotlight indexing](https://github.com/mas-cli/mas#spotlight).
- **Not sufficient:** `xcode-select -p` succeeds with only Command Line Tools installed, returning `/Library/Developer/CommandLineTools`. Check that the path is inside an Xcode app.
- **Ready:** `xcodebuild -version` once Xcode is selected (standard usage; not run here).
- **Signed in to the App Store:** no `mas` command reports it in 7.0.0, and I found no Apple command that does.
- **Signed in within Xcode:** no command found.

### Apple Account inside Xcode

This is a separate step from downloading. The listing says "To test or run applications on an Apple device, all you need is a free Apple Account", and App Store submission needs the paid program. Accounts are added in Xcode's settings.

### Inferred or unverified

- **Inferred:** no route gets a typical user from nothing to Xcode without an Apple Account, so Xcode always has a Needs-You step before its download.
- **Inferred:** a free Apple Account suffices for `mas`, since Xcode is listed as free.
- **Unverified at Apple's own source:** `xcodebuild -license accept` comes from `xcodes` source, not an Apple page I could read. This Mac has no `xcodebuild` man page because only Command Line Tools are present.
- **Unverified:** whether `-downloadPlatform` needs an Apple Account. The Apple page doesn't say.
- **Unverified:** whether simulator-only builds work without an Apple Account in Xcode.
- **Unverified:** the historical claim that `mas signin` broke on recent macOS. All I confirmed is that 7.0.0 has no such command.

**Dates:** the Apple pages show no edit date; the resources page's `Last-Modified` header is Oct 5 2026. The `xcodes` README was last committed Aug 10 2026 and the `mas` README Aug 25 2026. `mas` 7.0.0 came out May 4 2026.

## Findings that affect the spec

These are inferences from the facts above, offered for the owner to decide on.

1. **A Run can't finish Tailscale or Xcode alone.** Tailscale's approvals and every sign-in belong in the Needs-You Queue. So does App Store sign-in, which comes before an Xcode download, so Xcode can't simply queue at the end of a Run.
2. **Three installers edit shell startup files, each differently.** Codex uses `~/.zprofile`, opencode `.zshrc`, and Junie `.zshrc` or `.zprofile`. Claude Code's script edits none. If the Dotfiles step links its own `.zshrc` afterwards, those lines can be lost or need re-adding. Checking absolute paths avoids depending on `PATH`, which a GUI app doesn't inherit from a shell anyway.
3. **Sign-in commands assume a terminal.** Several can't run from a plain GUI process: opencode's API-key entry and `xcodes`' password prompts are confirmed, and `claude` and `junie` are likely. The engine may need a pty, or to open Terminal for these steps.
4. **Auto-updates cut against "updates only run when the user chooses".** Junie and native Claude Code update themselves. Tailscale Standalone can, and its per-device default isn't stated. Claude Code's casks don't by default, and Codex documents no background update.
5. **`gh` may need to come first.** Private Config Repos need GitHub sign-in, so `gh` or an equivalent may have to work before a Config can be read.
6. **Vendor scripts and Homebrew disagree for agents.** Homebrew fits the map's sync model; the vendors' scripts are what they recommend. Junie's tap currently lags, which is a concrete case against Homebrew there.
7. **Agent CLI detection isn't uniform.** Two give machine-readable state (`claude`, `opencode`), one an exit code (`codex`), and one nothing (`junie`). The Catalog may need a third state, such as "sign-in unknown".

### Recommendations for the owner

- **Tailscale:** Standalone through the `tailscale-app` cask, with a check for the App Store and formula copies first. Offer `tailscaled` only if there's a real need for running before login.
- **`gh`:** the Homebrew formula.
- **opencode:** `brew install opencode` or the `opencode-v2` tap formula, and avoid the bare `opencode.ai/install` URL and the tap's `opencode`, since both give V1.
- **Junie:** the vendor script, given the tap's lag.
- **Claude Code and Codex:** the owner's call: casks suit the update rule and sync model, and scripts are the vendor default.
- **Xcode:** `mas` as the default, with App Store sign-in queued first. Offer `xcodes` as an explicit option for specific or multiple versions, with a clear note that it takes an Apple password.

### Decisions left to the owner

- Vendor script or Homebrew for each agent CLI.
- Whether to disable auto-updates for Claude Code, Junie and Tailscale, or to accept them as exceptions.
- Whether `xcodes` is in the Catalog at all, given how it handles the Apple password.
- Whether Junie's undetectable sign-in shows as "unknown" or is simply left off.

# Running a GUI app unattended: sudo, Homebrew, Command Line Tools and macOS permissions

Research for ticket #5. Researched 2026-11-06 against macOS 27.2 (build 26B8125), sudo 1.9.17p2 and Homebrew 7.0.12.

## Answer

A Run can be unattended for almost everything **if the person gives their password once at the start and the engine holds it in memory for a limited time**. Nothing on macOS gives an app silent root access or silently grants privacy permissions on an unmanaged Mac.

- **`sudo`:** `SUDO_ASKPASS` is the only sudo hook Homebrew exposes that works without a terminal. A `sudo -v` keep-alive won't work, because the cached credential doesn't reach Homebrew's `sudo` calls.
- **Homebrew:** formulae and most casks need no `sudo`. About 9% of active casks do, mainly those with `.pkg` installers.
- **Command Line Tools:** these can install with no dialog through `softwareupdate`. It needs admin rights and relies on a placeholder file Apple doesn't document, so it needs a fallback.
- **Permissions:** the app can only prompt for itself and open System Settings for the person. Permissions for other apps always go in the Needs-You Queue. Only a user-approved MDM can pre-approve them, and some it can only deny.

The main security limit: **while the engine can answer `sudo`, any code running as that user can get root.** That includes third-party tap code. The design choice is how to keep that window short, not how to remove it.

## How to read this

| Label | Meaning |
|---|---|
| **Read** | I read this in the cited primary source. I did not run it. |
| **Ran** | I observed this on this Mac with a read-only command. |
| **Inferred** | My conclusion from the sources. It was not tested and no source states it. |

As instructed, I ran no `sudo`, installs, `softwareupdate`, permission prompts or `x-apple.systempreferences:` links. **So nothing here about sudo, brew, Command Line Tools or permission behavior has been run end to end.** [Spikes before building](#spikes-before-building) lists what to test. `[n]` refers to [Sources](#sources).

## What can be automated

| Step | Unattended? | What it needs | Evidence |
|---|---|---|---|
| Keep the Mac awake | Yes | `caffeinate -i -w <pid>`. `-s` only applies on AC power. | Read [13] |
| Command Line Tools | Yes, with sudo | `softwareupdate -i`, which needs admin rights. See [Command Line Tools](#command-line-tools). | Read [1][12] |
| Install Homebrew | Yes, with sudo | `NONINTERACTIVE=1` plus `SUDO_ASKPASS`, or the `.pkg` installed as root | Read [1][5] |
| Formulae | Yes | No `sudo` | Read [8] |
| Tap trust | Yes | `brew trust`, or a fully qualified name that trusts only that Item. It still needs the separate Import approval the map requires. | Read [9] |
| Casks that just move an app | Yes on an admin account | No `sudo` when the destination is writable, otherwise brew retries with sudo | Read [4]; Inferred for the admin default |
| Casks with `pkg`, or `installer script` with `sudo: true` | Only with sudo | `SUDO_ASKPASS`, or queue them | Read [3][4] |
| Replacing an existing app (upgrades, some Adopt or Apply cases) | **May stop for a prompt** | App Management permission, even with `sudo` | Read [4] |
| Casks with `installer manual:` | No: queue | The cask only prints an `open …` instruction | Read [4] |
| Kernel extensions | No: queue | Recovery boot on Apple Silicon, Reduced Security, admin password, restart | Read [17] |
| System extensions | No: queue | The owning app activates them when launched, and may need user approval | Read [15][16] |
| Privacy permissions for installed apps | No: queue | See [macOS permissions](#macos-permissions) | Read [14][18]; Inferred for the per-app limit |
| First launch of a quarantined app | No: queue or hint | Gatekeeper's first-launch dialog | Read [9][19] |
| Touch ID for sudo, if enabled | **Could stall a Run** | See [Ways to hold `sudo`](#ways-to-hold-sudo) | Inferred |
| Restart | No: offer it | `bootstrap.sh` uses `sudo shutdown -r now` today. Leaving it to the person is a design choice. | Read (repo) |
| Non-admin accounts | Partly | No sudo: formulae work, and compatible app casks need a writable `--appdir`. Homebrew itself must be set up by an admin first. | Read [8] |

Today's scripts use `sudo` only for the optional restart (`bootstrap.sh:257`). The Homebrew installer and Brewfile casks can also ask for it. **Inferred:** demand for `sudo` is small and comes from a known set of steps, so the plan can predict it.

**How common is sudo among casks?** From Homebrew's cask API on 2026-11-06, there are 6,947 casks that aren't disabled [10]:

| Cask type | Count | Share |
|---|---:|---:|
| `pkg` artifact | 586 | 8.4% |
| `installer script` with `sudo: true` | 30 | 0.4% |
| `installer manual:` | 37 | 0.5% |

These counts cover install artifacts only, not `sudo` fallbacks or uninstall steps. Examples with `pkg` include `zoom`, `microsoft-teams`, `microsoft-office`, `tailscale-app` and `karabiner-elements`. Of this repo's casks, `discord`, `ghostty`, `jetbrains-toolbox` and `linearmouse` have no `pkg`. `nikitabobko/tap/aerospace` is in a third-party tap, so I did not check it.

**Inferred:** the plan can sort Items in advance from this metadata (or `brew info --json=v2`). The queue should use that metadata rather than exit codes, because a manual-installer cask only prints a message at install time [4].

## How `sudo` behaves without a terminal

**`SUDO_ASKPASS` (Read [11]).** With `-A`, sudo runs the helper at that path and reads the password from its stdout. Without a terminal or a helper, sudo fails with `a terminal is required to read the password`.

**Homebrew supports it (Read [3][5]).**

- `bin/brew` keeps `SUDO_ASKPASS` when it filters the environment.
- `system_command.rb` adds `-A` whenever it's set.
- `install.sh` uses `-A` when it's set, even with `NONINTERACTIVE`. Without it, `NONINTERACTIVE` means `sudo -n`, which fails unless no password is needed.
- Homebrew calls `/usr/bin/sudo` by full path, so a replacement `sudo` on `PATH` is not an option.

**Credential caching works against a keep-alive.**

- With no terminal, sudo keys its 5-minute record to the **parent process ID** (Read [11][20]). A record from the engine's own `sudo -v` doesn't match a `sudo` started by `brew`, because its parent is different (Inferred).
- A pseudo-terminal doesn't rescue this. Homebrew's first sudo check in a process runs `sudo --reset-timestamp`, which invalidates the terminal's record (Read [6][20]). `brew bundle` runs casks through `brew install --cask` [7], so this can recur per cask (Inferred).
- `HOMEBREW_SUDO_CHECKED=1` appears to skip that check [3], but it's an internal variable, so I wouldn't rely on it.

**A pseudo-terminal also changes `install.sh`.** It treats a terminal on stdin as interactive and waits for a keypress unless `NONINTERACTIVE=1` is set. So set that variable every time (Read [1]).

**Who can use sudo.** Apple's published sudoers file gives `%admin` `(ALL) ALL` and includes `/private/etc/sudoers.d` [21]. The live `/etc/sudoers` isn't readable from this account, so I did not compare it. The engine can check `admin` group membership with `id -Gn` without touching sudo. Homebrew avoids needless sudo probes to prevent auth log noise [22].

## Ways to hold `sudo`

| Option | Works with `brew` casks? | Person's involvement | Security trade-off |
|---|---|---|---|
| **A. Password in engine memory, served by an askpass helper** | Yes | Types it once per Run | Any same-user code can run `sudo -A` with the helper during the window and get root. The password itself could leak if the helper hands it out carelessly. Ends with the Run. |
| **B. Queue sudo Items with `HOMEBREW_NO_SUDO=1`** | Rejects them up front [8] | Returns later and enters the password | **No password held while away.** Those Items wait, which reduces how hands-off a Run feels. |
| **C. `sudo -v` keep-alive** | **No**, see above | Types it once | Doesn't solve the problem. Also grants root to anything sharing the record. |
| **D. `NOPASSWD` drop-in in `/etc/sudoers.d`** | Yes | Types it once to install | Silent root for all same-user code, and it survives reboots if cleanup fails. A narrower rule doesn't help much (see below). |
| **E. Privileged daemon via `SMAppService`** | **Not directly** | Admin approves it once in Login Items | Apple's recommended route for ongoing privileges [23]. Permanent root code, so it must verify callers [24][25]. Brew can't call it. |
| **F. System admin dialog** (`do shell script … with administrator privileges`) | No | Dialog for each script | The app never sees the password, and the cache lasts 5 minutes per script [26]. Apple DTS says `sudo` and `osascript` are "not appropriate to use as API" [23]. |
| **G. Our own notarized `.pkg` does the bootstrap** | No, bootstrap only | System installer password prompt | Apple DTS calls an installer package "by far" the easiest route when only installation needs privileges [23]. Doesn't help later casks. |
| **H. Password saved in Keychain** | Yes | None after the first time | Lasting custody of an admin password in a public app. Same-window risk on every Run. |

**Also ruled out:** `AuthorizationExecuteWithPrivileges` ("Do not use it in a widely distributed product") and setuid-root ("Do not use a setuid-root executable. Ever.") [23]. `SMJobBless` has been deprecated since macOS 13 [27].

**Option A's limit (Inferred).** The helper can't identify its caller in any useful way. An attacker can run `sudo -A` with the helper, so its parent is the genuine `/usr/bin/sudo`, and sudo then runs the attacker's command as root. Tokens and parent checks only stop direct requests for the password. They still protect something real: **root during a Run ends with the Run, but a leaked password doesn't.**

Don't follow Strap's helper design. Strap comes from a Homebrew maintainer and writes the password in plaintext to a temp script, relying on `chmod 700` and cleanup on exit [28]. For a public app, a crash could leave that file behind (Inferred). Instead:

- Keep the password out of files, command arguments and environment variables.
- Serve it over an inherited pipe or socket. Assume environment variables are visible to same-user code (Inferred, not tested).
- Sign the helper and enable the Hardened Runtime, which notarization requires anyway [29]. Notarization also rejects `get-task-allow`, which would otherwise allow code injection [30].
- Clear the password when the privileged phase ends.
- App Sandbox is only required for the Mac App Store [31]. I expect a sandbox to be incompatible with running `sudo` (Inferred).
- Rune has a macOS secure-input claim for password windows at `dev` rev `ddf1414` [32]. I did not evaluate whether that's enough for this field.

**Option D's limit.** Homebrew passes `-E`, and only rules matching `ALL` get `SETENV` automatically [3][12a], so a narrow rule must add `SETENV`. A rule limited to `/usr/sbin/installer` is still root for anyone who can build a `.pkg`. A rule naming anything under `/opt/homebrew` is worse, because the Homebrew owner can edit that code [33]. The sudoers manual warns that a program run through sudo can start other programs [12a].

**Option E's limit (Inferred).** Since Homebrew can't call the daemon, it could only serve `brew` by running `.pkg` steps itself or by writing a temporary sudoers rule, which is option D with extra steps. Homebrew deliberately won't substitute for the installer step [8]. The map says updates run only when the user chooses, so the person is present for later Runs, which weakens the case for permanent root code.

**Touch ID (Inferred).** `/etc/pam.d/sudo` includes `sudo_local`, where `pam_tid.so` can be enabled (Ran). On a Machine with that set, `sudo` may ask for a fingerprint before using askpass, so a Run could wait for a person who has left. Strap treats Touch ID as a reason to skip its askpass setup [28]. Whether this happens with no terminal is untested.

**The third-party tap problem (Read [9]; Inferred for root).** Loading a non-official tap can run Ruby as the user, and `bin/brew` passes `SUDO_ASKPASS` through. So a tap loaded while the helper is live can get root. Trusting a tap therefore means more during a privileged phase than Homebrew's "user's privileges" wording suggests.

### Recommendation for the human to weigh

Combine A and B, and keep the privileged phase short:

1. Build the plan first and ask for a password only if it contains sudo Items.
2. Fetch downloads first, then run privileged Items together at the start: Command Line Tools, Homebrew, then `pkg` casks.
3. Provide `SUDO_ASKPASS` only to the specific child process that needs it.
4. Clear the password, then run everything else with `HOMEBREW_NO_SUDO=1`. A surprise sudo need then fails fast and goes to the Needs-You Queue instead of stalling.
5. Offer "don't hold my password" as a choice, which queues all sudo Items (option B).
6. Rule out C, D and H. Consider E only if a future feature needs root with nobody present.

G could complement this if the bare-Mac install command ends up using a `.pkg`.

The CLI fallback has a terminal, so `sudo` can prompt there directly. **Inferred:** Homebrew's timestamp reset could make it prompt again mid-Run, so a walk-away CLI Run needs the same helper design.

### Decisions for the human

1. Is holding the password in memory for the privileged phase acceptable, or should sudo Items always be queued?
2. Can third-party tap Items run while the helper is live, or must they wait until it's gone? Should the Import preview say that trusting a tap can mean root during a Run?
3. Are non-admin accounts supported, or refused with an explanation?

## Homebrew non-interactive mode

| Setting | Effect | Source |
|---|---|---|
| `NONINTERACTIVE=1` | `install.sh` doesn't prompt. Also turned on by `CI`, or when stdin isn't a terminal. | [1][8] |
| `HOMEBREW_NO_SUDO=1` | Brew never calls sudo, and casks that require it are rejected before install | [2][8] |
| `HOMEBREW_FORBIDDEN_CASK_ARTIFACTS` | Refuses listed artifact types, such as `pkg installer` | [2] |
| Ask mode | Default behavior, and the confirmation is skipped without a terminal | [2] |
| `.pkg` installer | Apple Silicon only, installs to `/opt/homebrew`, and `installer` needs root | [8][12] |

The current `Homebrew.pkg` is signed with Developer ID and notarized: `pkgutil` and `spctl` accepted it (team `927JGANW46`) (Ran). Its post-install script deliberately avoids triggering the developer tools prompt and doesn't install Command Line Tools, so it only resets its Git checkout when Git already exists [34].

Homebrew refuses to treat `sudo brew` as safe, because the prefix owner can change what runs as root [33]. So the engine must never run `brew` itself under sudo. Homebrew only elevates individual commands.

## Command Line Tools

**Unattended route (Read [1]).** Homebrew's own installer does this:

1. Create `/tmp/.com.apple.dt.CommandLineTools.installondemand.in-progress`.
2. Find the label with `softwareupdate -l`, filter for `Command Line Tools`, and take the highest version with `sort -V`.
3. Run `softwareupdate -i <label>` as root, then `xcode-select --switch /Library/Developer/CommandLineTools`.
4. Remove the placeholder.

Strap uses the same placeholder [28]. **I found no Apple document describing it**, so treat it as a working convention that could break. `softwareupdate` needs admin rights for everything except `--list` [12]. I did not run this on macOS 27.2, where Command Line Tools are already installed.

**Fallback.** `xcode-select --install` opens a dialog [12b] and needs no `sudo` in Homebrew's installer, which suits the Needs-You Queue. `install.sh` only tries it with a terminal on stdin. Without one, it continues and then aborts for lack of Git [1], so the engine should handle this step itself first.

**Other routes.** A signed-in Apple Developer download provides the `.pkg` [35][36]. Homebrew documents that route for MDM deployments [36]. It needs an Apple account, and I did not check whether redistributing it is allowed.

**Don't touch the stubs.** `/usr/bin/git` can be a stub that triggers the install prompt [36]. On a bare Mac, the engine shouldn't call it or any similar tool before Command Line Tools exist. Check for `/Library/Developer/CommandLineTools/usr/bin/git`, as Homebrew does [1]. Today's `scripts/xcode.sh` uses `xcode-select -p` for this. Homebrew verifies with `xcrun git --version` after install [36].

**Full Xcode (opt-in).** Homebrew's message for this case is `sudo xcodebuild -license` [1]. Getting Xcode itself isn't covered here.

## macOS permissions

Apple states that apps must get user consent for protected file locations, that full-disk apps must be "explicitly added" in System Settings, and that accessibility and automation need user permission [14].

| Permission | Own-process request | Pane link key on 27.2 | MDM can pre-approve? |
|---|---|---|---|
| Accessibility | `AXIsProcessTrustedWithOptions` with `kAXTrustedCheckOptionPrompt`, asynchronous [37] | `Privacy_Accessibility` | Yes, but the key is deprecated in 27, which shows a notification [18] |
| Full Disk Access | None found. Homebrew says the system only prompts for App Management [4]. | `Privacy_AllFiles` **(documented [38])** | Yes [18] |
| App Management | No API found. Trying to modify an app triggers the prompt [4]. | `Privacy_AppBundles` | Yes [18] |
| Screen Recording | `CGRequestScreenCaptureAccess`, and no second prompt after a denial [37] | `Privacy_ScreenCapture` | **No, deny only** [18] |
| Input Monitoring | `IOHIDRequestAccess`: **not verified** (no doc or header found) | `Privacy_ListenEvent` | **No, deny only** [18] |
| Automation | `AEDeterminePermissionToAutomateTarget`. Target must be running, and the call can block [37]. | `Privacy_Automation` | Yes [18] |
| Files and folders | Prompts on access [14] | `Privacy_FilesAndFolders` | Yes [18] |
| Login Items | `SMAppService.openSystemSettingsLoginItems()` **(supported API [39])** | Not needed | Separate mechanism [40] |

- **Links.** The documented form is `x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_AllFiles`, in `EndpointSecurity/ESClient.h`, for that one key [38]. The other keys appear in this Mac's System Settings files (Ran), but I did not open them and found no Apple documentation for them. They could change between releases, so the fallback should be a plain Privacy & Security link and a written instruction.
- **Prompting is per process.** These APIs act for the current process [37]. **Inferred:** the app can't request permissions for an app it just installed, so those Items go in the queue with a link and instructions. For example, `docs/manual-steps.md` already lists Accessibility for AeroSpace.
- **No local grant path.** `tccutil` only supports `reset` [12c]. The MDM payload needs user-approved MDM and can't be installed by hand [18]. MDM is out of scope for this app. I did not look at editing the privacy database directly and wouldn't recommend it.
- **App Management on replacements.** Homebrew checks this by writing a test file into an existing app, including during `chown` after `pkg` installs, and on failure raises an error or falls back to a reinstall [4]. A fresh install to an empty location skips that check (Inferred from `moved.rb`). **Inferred, needs a spike:** with `brew` launched from our app, macOS would likely prompt for our app, even though Homebrew's text says "your terminal". This matters most for Apply on a Machine in use.

## Spikes before building

Each needs a throwaway test on a clean Mac or VM, not this one:

1. Spawn `brew install --cask <pkg cask>` from a GUI process with no terminal and an askpass helper. Count helper calls and check whether anything else prompts.
2. Run the `softwareupdate` placeholder route on a bare macOS 27 install, and record how it behaves with the screen locked.
3. Enable `pam_tid.so` and repeat test 1 to see whether a Run stalls.
4. Upgrade an app from a GUI-launched `brew`. See which app the App Management prompt names and whether `brew` blocks while it's open.
5. Open each `Privacy_*` link on macOS 15, 26 and 27 to confirm where it lands.

## What I couldn't verify

- **Behavior at runtime:** see [How to read this](#how-to-read-this).
- **Apple's statement on supported URL schemes:** the forum thread [41] returned a bot-check page. The header in [38] is the documentation I relied on.
- **`IOHIDRequestAccess`:** not found in Apple's documentation data or this Mac's SDK headers.
- **Apple's table of prompted versus manual permissions** [14]: its checkmarks are images I couldn't read, so I used the page's text instead.
- **The live `/etc/sudoers`:** not readable from this account. I read Apple's open-source copy, which may differ.
- **Keychain password behavior:** the Apple page I tried yielded nothing usable, so option H's assessment doesn't rely on it.
- **Earlier macOS versions:** local checks were on 27.2 only. Homebrew 7 requires macOS 15 or later [8].

## Sources

Homebrew links are pinned to the commits I read. I confirmed that the files I cite by line, and the docs, matched those commits. Numbers are fixed labels grouped by owner, so they aren't sequential.

**Homebrew** (`Homebrew/install` at `7986efa`, `Homebrew/brew` at `88f401f`, release 7.0.12)

- **[1]** [`install.sh`](https://github.com/Homebrew/install/blob/7986efac25016e41eed7e471666140c06f9d97cf/install.sh): non-interactive logic (lines 125-147), `have_sudo_access` (287-344), `execute_sudo` (372-391), keypress wait (810-814), Command Line Tools (885-921), Git abort (970)
- **[2]** [`env_config.rb`](https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/Library/Homebrew/env_config.rb): `HOMEBREW_ASK` (96-106), `HOMEBREW_FORBIDDEN_CASK_ARTIFACTS` (371), `HOMEBREW_NO_SUDO` (653)
- **[3]** [`system_command.rb`](https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/Library/Homebrew/system_command.rb): `sudo_available?` (147-155), `sudo_prefix` (412-432)
- **[4]** Cask code: [`quarantine.rb`](https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/Library/Homebrew/cask/quarantine.rb) (212-276), [`staged.rb`](https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/Library/Homebrew/cask/staged.rb) (39-53), [`moved.rb`](https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/Library/Homebrew/cask/artifact/moved.rb) (153-168), [`pkg.rb`](https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/Library/Homebrew/cask/artifact/pkg.rb), [`installer.rb`](https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/Library/Homebrew/cask/artifact/installer.rb)
- **[5]** [`bin/brew`](https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/bin/brew): environment filter (303-316)
- **[6]** [`utils/sudo.sh`](https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/Library/Homebrew/utils/sudo.sh): `--reset-timestamp` check
- **[7]** [`bundle/cask.rb`](https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/Library/Homebrew/bundle/cask.rb): `Bundle.brew("install", "--cask", …)`
- **[8]** [Installation](https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/docs/Installation.md): `.pkg`, requirements, running without sudo, unattended installation
- **[9]** [Tap Trust](https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/docs/Tap-Trust.md) and [Security and Supply Chain](https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/docs/Homebrew-Security-and-Supply-Chain.md): tap code, quarantine, Gatekeeper
- **[10]** [Cask API](https://formulae.brew.sh/api/cask.json): counted on 2026-11-06. 868 of 7,815 casks were disabled and excluded.

**Manuals on this Mac** (macOS 27.2, read-only)

- **[11]** `sudo(8)`: `-A`, `-k`, `-n`, `-v`, `SUDO_ASKPASS`, diagnostics; `sudoers(5)`: `timestamp_type`, `timestamp_timeout`
- **[12]** `softwareupdate(8)`, `installer(8)`. 12a: `sudoers(5)` on `SETENV` and "Preventing shell escapes". 12b: `xcode-select(1)` on `--install`. 12c: `tccutil(1)`
- **[13]** `caffeinate(8)`

**Apple**

- **[14]** [Controlling app access to files in macOS](https://support.apple.com/guide/security/secddd1d86a6/web)
- **[15]** [Installing System Extensions and Drivers](https://developer.apple.com/documentation/systemextensions/installing-system-extensions-and-drivers)
- **[16]** `OSSystemExtensionRequest.h`, `requestNeedsUserApproval` (macOS SDK)
- **[17]** [Kernel extensions](https://support.apple.com/guide/security/kernel-extensions-sec8e454101b/web)
- **[18]** [PrivacyPreferencesPolicyControl](https://developer.apple.com/documentation/devicemanagement/privacypreferencespolicycontrol) and its [Services](https://developer.apple.com/documentation/devicemanagement/privacypreferencespolicycontrol/services-data.dictionary) dictionary
- **[19]** [Notarizing macOS software](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution) on the first-launch dialog, and [Safely open apps on your Mac](https://support.apple.com/en-us/102445)
- **[23]** [BSD Privilege Escalation on macOS](https://developer.apple.com/forums/thread/708765), Apple DTS, revised 2025-03-24 (read through a fetch sub-agent, quotes as it reported them)
- **[24]** [`SMAppService`](https://developer.apple.com/documentation/servicemanagement/smappservice), [`register()`](https://developer.apple.com/documentation/servicemanagement/smappservice/register()), [`.requiresApproval`](https://developer.apple.com/documentation/servicemanagement/smappservice/status-swift.enum/requiresapproval)
- **[25]** [`setCodeSigningRequirement(_:)`](https://developer.apple.com/documentation/foundation/nsxpcconnection/setcodesigningrequirement(_:))
- **[26]** [TN2065](https://developer.apple.com/library/archive/technotes/tn2065/_index.html)
- **[27]** [`SMJobBless`](https://developer.apple.com/documentation/servicemanagement/smjobbless(_:_:_:_:))
- **[29]** [Hardened Runtime](https://developer.apple.com/documentation/security/hardened-runtime)
- **[30]** [Resolving common notarization issues](https://developer.apple.com/documentation/security/resolving-common-notarization-issues)
- **[31]** [App Sandbox](https://developer.apple.com/documentation/security/app-sandbox)
- **[35]** [Installing the command-line tools](https://developer.apple.com/documentation/xcode/installing-the-command-line-tools/)
- **[37]** SDK headers: `AXUIElement.h`, `CGWindow.h`, `AppleEvents.h`
- **[38]** SDK header: `EndpointSecurity/ESClient.h` (line 767)
- **[39]** [`openSystemSettingsLoginItems()`](https://developer.apple.com/documentation/servicemanagement/smappservice/openSystemSettingsLoginItems())
- **[40]** [Manage login items and background tasks on Mac](https://support.apple.com/guide/deployment/depdca572563/web)
- **[41]** [Supported URL Schemes](https://developer.apple.com/forums/thread/761314) (could not read)

**Other**

- **[20]** [`sudo-project/sudo` `timestamp.c`](https://github.com/sudo-project/sudo/blob/28079dbd4edc58319f82e264ec04095936de4212/plugins/sudoers/timestamp.c): `ts_init_key`, `timestamp_remove`. This is upstream source. The local version is 1.9.17p2, and the manual agrees on the behavior.
- **[21]** [`apple-oss-distributions/sudo` `files/sudoers`](https://github.com/apple-oss-distributions/sudo/blob/802f972bdc92b5d54dea338bfcc1ac7ad30d1ef6/files/sudoers)
- **[22]** [Homebrew commit history for `utils/sudo.sh`](https://github.com/Homebrew/brew/commits/88f401fb862a12a0a5a201f189de570efc2a9ff1/Library/Homebrew/utils/sudo.sh): "Check sudo access only when first needed", 2026-09-30
- **[28]** [`MikeMcQuaid/strap` `bin/strap.sh`](https://github.com/MikeMcQuaid/strap/blob/e041b6b9304d5d0956e8d044c4199ff191da594c/bin/strap.sh): `sudo_init` (98-139), Command Line Tools (278-295)
- **[32]** [`anomalyco/rune` `secure_input.rs`](https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune/src/input/secure_input.rs)
- **[33]** [Homebrew FAQ, "Why does Homebrew say sudo is bad?"](https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/docs/FAQ.md#why-does-homebrew-say-sudo-is-bad)
- **[34]** [Homebrew `package/scripts/postinstall`](https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/package/scripts/postinstall)
- **[36]** [Homebrew for Mac Admins](https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/docs/Homebrew-for-Mac-Admins.md): "Install Git for management jobs"

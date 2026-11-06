# From one command to a notarized running app on a bare Mac

Research for ticket #4, child of map #2. Researched 2026-11-06. Product decisions are left to the owner and are listed under [Decisions for the owner](#decisions-for-the-owner).

## Answer

A bare Apple Silicon Mac already has everything the entrypoint needs: Terminal, `curl`, `hdiutil`, `ditto`, `codesign`, `spctl` and `open`. None of them needs Command Line Tools. The path is:

1. The person pastes one command into Terminal.
2. A short script downloads a signed, notarized, stapled release.
3. The script checks the signature and Team ID, copies the app to `/Applications` and opens it.
4. The app installs Command Line Tools, Homebrew and everything else during the Run.

`curl` does not set the quarantine attribute, so this path does not trigger the quarantine-driven Gatekeeper check. That means **nothing on this path reliably enforces notarization**, so the script should run the check itself. Notarization is still worth having: browser downloads and Homebrew casks do get quarantined, and Apple says when Gatekeeper runs on other files is undocumented and changes over time.

**How to read the labels**

- **[Tested]**: I ran it on macOS 27.2 on Apple Silicon. This Mac has Command Line Tools, so it is not a bare Mac.
- **[Documented]**: a primary source states it, cited inline.
- **[Inferred]**: my conclusion from tested or documented facts.

## Quarantine: `curl` versus a browser

| | One-line `curl` | Browser download |
|---|---|---|
| Quarantine attribute | Not set. **[Documented]** "Unix-y networking tools, like `curl` and `scp`, don't quarantine the files they download" ([F1]). **[Tested]** A notarized DMG fetched with `curl` had no `com.apple.quarantine`, and neither did any file in the app copied out of it with `ditto`. | Set by "user-level apps" for new downloads ([F1]). |
| Gatekeeper at first launch | Not triggered by quarantine. "The system may run Gatekeeper at other times as well", and the circumstances are "not documented" and change over time ([F1]). | "Running a quarantined app always invokes Gatekeeper" ([F1]). |
| Malware check at first open | Still applies: software is checked "the first time it's opened, regardless of how it arrived on the Mac" ([S1]). | Applies. |
| What the person sees | **[Inferred]** No Gatekeeper dialog. Not observed, because I launched nothing. | An initial launch dialog with notarization information ([A1]). |
| Unnotarized app | **[Tested]** An ad hoc bundle was `rejected` by `spctl`, yet its executable still ran when started directly, because it had no quarantine. | Blocked. The person has to approve it in System Settings > Privacy & Security > Open Anyway ([S2]). Control-click no longer overrides this as of macOS 15 ([N1]). |
| Path randomization (translocation) | **[Inferred]** Avoided, because the script moves the app before it first opens. | Happens if the app is first opened in place from the zip or DMG, and only on that first launch ([A5]). The app must not reach for files outside its bundle by relative path. |

Other facts that shape the design:

- **Quarantine is not an API.** Apple's post says the attribute is "not documented as API" ([F1]). The design should work whether or not a file is quarantined. Notarizing gives you that.
- **Unsigned code does not run at all on Apple Silicon.** The system requires a signature, though "a simple ad-hoc signature is sufficient" ([A9]). **[Tested]** An ad hoc binary ran. The same binary with its signature removed was killed (exit 137).
- **`com.apple.provenance` is not quarantine.** **[Tested]** It appeared on files created here. I found no Apple documentation for it and did not research it further.
- **No macOS 27 change found.** I found no Apple statement changing this model in macOS 27, and the tests above match the documented behavior on 27.2.

## What the Apple Developer Program requires

All **[Documented]** unless marked.

- **Membership:** 99 USD per year ([P1]). Developer ID use is tied to the paid program ([P2]). Notarization is not App Review; it is an automated scan ([A1]).
- **Individual enrollment:** an Apple Account with two-factor authentication, and the legal age of majority ([P1]).
- **Organization enrollment:** a legal entity (no DBAs or trade names), a D-U-N-S Number, a work email on the organization's domain and a public website ([P1]).
- **Who can create the certificate:** only the Account Holder can create a Developer ID certificate ([P3], [F2]), with a limit of five Developer ID Application certificates ([P3]). Cloud-managed Developer ID certificates can also be created by admins with a specific access setting ([P3]).
- **Which certificate:** Developer ID Application for an app or DMG. Developer ID Installer only if you ship a `.pkg` ([A5]).
- **Name shown publicly:** **[Tested]** Zed's certificate reads `Developer ID Application: Zed Industries, Inc. (MQ55VZLNZQ)`, with the same name in its `O=` field. Anyone can read this using `codesign -dvv`. **[Inferred]** An individual enrollment would put the person's legal name there. I did not inspect an individual certificate.
- **Lifetime:** **[Tested]** Zed's certificate runs five years (2026-08-18 to 2031-08-19). Apple's pages I read do not state the period.
- **If the certificate expires:** existing signed versions still download, install and run, but new versions need a new certificate ([P4]).
- **If membership lapses:** signed apps keep working, but you can't get new certificates ([P3]).
- **If the certificate is revoked:** the software can no longer be installed, and it won't launch even if already installed ([P3]).
- **If the key leaks:** the notary service keeps an audit trail, and you can work with Apple to revoke tickets for unauthorized versions ([A1]).

### CI credentials

Notarize with an App Store Connect **Team key**, which Zed and Ghostty's flags suggest they also use ([C1], [C2]). Generating one requires an App Store Connect Admin ([A7]).

Apple's sources disagree about individual keys. The API key page says they can't use `notaryTool` ([A7]), while the `notarytool` man page (1.1.4) says it supports them. A Team key is valid under both, so the conflict doesn't need resolving.

## Signing and notarizing a Rust `.app` from CI

Cargo produces a binary, not a bundle. Rune doesn't produce one either: "Rune doesn't bundle apps" ([R1]). Its `scripts/mac-app` is a development tool that installs to `~/Applications` and signs with `codesign --force --sign` ([R2]). It passes neither `--timestamp` nor `-o runtime`, both of which notarization requires ([A1]). This project needs its own release step.

Apple's flow for third-party build systems ([A4], [A5], [A2]) adapted for this app, assuming a GUI executable and a CLI in one bundle:

```sh
# 1. Build (rustc honors the deployment target variable; see [C5])
MACOSX_DEPLOYMENT_TARGET=<min> cargo build --release --target aarch64-apple-darwin

# 2. Assemble the bundle
#    App.app/Contents/Info.plist
#    App.app/Contents/MacOS/<gui>  and  App.app/Contents/MacOS/<cli>
#    App.app/Contents/Resources/...

# 3. Import the certificate into a temporary keychain (see [C3])

# 4. Sign from the inside out. Do not use --deep.
codesign -s "$ID" -f --timestamp -o runtime -i "$BUNDLE_ID.cli" App.app/Contents/MacOS/<cli>
codesign -s "$ID" -f --timestamp -o runtime --entitlements app.entitlements App.app

# 5. Notarize the app and staple it
ditto -c -k --keepParent App.app App.zip
xcrun notarytool submit App.zip --key key.p8 --key-id "$KEY_ID" --issuer "$ISSUER" --wait
xcrun stapler staple App.app

# 6. Package the stapled app, then notarize and staple the DMG
hdiutil create -srcFolder dmg-root -format UDZO -o App.dmg
codesign -s "$ID" --timestamp -i "$BUNDLE_ID.dmg" App.dmg
xcrun notarytool submit App.dmg --key key.p8 --key-id "$KEY_ID" --issuer "$ISSUER" --wait
xcrun stapler staple App.dmg

# 7. Gate the release
spctl -a -vv -t exec App.app
spctl -a -vv -t open --context context:primary-signature App.dmg
```

Why these details matter:

- **Requirements:** a Developer ID certificate, a secure timestamp and the Hardened Runtime are all required, and `get-task-allow` must be absent ([A1]). `timestamp.apple.com` must be reachable ([A3]).
- **`--deep`:** Apple says not to sign with it, because it applies the same options to every item and skips code in unexpected places ([A4]). Zed uses it anyway ([C1]), so don't copy that part.
- **The CLI:** Apple places helper tools in `Contents/MacOS/` or `Contents/Helpers/` ([A6]), and non-bundled code needs its own `-i` identifier ([A4]). Tickets can't be stapled to a standalone binary ([A2]), which **[Inferred]** is one reason to ship the CLI inside the bundle, as Zed does.
- **Entitlements:** add only what a test build proves necessary. Hardened Runtime exceptions need no provisioning profile (Apple's term), but restricted entitlements do ([A4]). **[Tested]** Zed claims `allow-jit` and `allow-unsigned-executable-memory`. A search of Rune found no mention of those entitlements, but I did not build or notarize a Rune app.
- **Two notarization rounds:** Apple's standard advice is to notarize only the outermost container ([A5]). That leaves the inner app unstapled, which is what I found in Zed. **[Tested]** Zed's DMG has a stapled ticket and its app doesn't. Gatekeeper still accepted that app as `Notarized Developer ID`, presumably by finding the ticket online. `syspolicy_check` calls the missing ticket "Fatal", though Apple's post says stapling "isn't always required" ([F1]). Stapling the app first ([A2]) means the copied app carries its own ticket. **[Inferred]** It costs one more submission and keeps offline launches working.
- **Timing and limits:** most submissions finish within 5 minutes and 98 percent within 15. Apple asks for no more than 75 per day ([A2]). Notarize tagged releases only.
- **Always read the log,** even on success ([A2]).

### CI environment

- **Runners:** `macos-15`, `macos-26` and `xcode-27` are Arm64, and standard runners are free on public repositories ([C4]).
- **Tools:** Xcode isn't strictly required for submitting. **[Tested]** Command Line Tools include `notarytool` (1.1.4) and `stapler`. However, the `/usr/bin/stapler` shim failed with "requires Xcode" under Command Line Tools, while `/Library/Developer/CommandLineTools/usr/bin/stapler` worked. A REST Notary API also exists ([A8]).
- **Secrets:** certificate `.p12` (base64), its password, a throwaway keychain password, the `.p8` key, key ID and issuer ID. GitHub doesn't pass secrets to workflows triggered from a fork ([C6]). Signing should run only on tagged releases.
- **Keychain:** GitHub's guide imports with `-A` ([C3]), while Zed and Ghostty use the narrower `-T /usr/bin/codesign` ([C1], [C2]). **[Inferred]** Prefer `-T`.
- **Backup:** export a `.p12` backup of the Developer ID identity ([F2]), and store it outside the repository.
- **Without a Mac:** `rcodesign` claims to sign and notarize from non-Apple systems ([C7]). It is third party and not needed here, since macOS runners are free.

## Does the app need anything preinstalled?

**No, on the available evidence**, but this hasn't been confirmed with a real Rune build on a bare Mac.

- **Comparable app, [Tested]:** Zed, a Rust GUI app, links zero non-system libraries (33 system ones). The `arm64` build has a minimum OS of 12.0.
- **Rune, [Inferred]:** its macOS dependencies are `objc2` bindings to system frameworks plus Metal and `wgpu` ([R3]), so a Rune app should behave the same.
- **Rosetta, [Inferred]:** not needed for an `arm64`-only build.
- **Rust minimum:** macOS 11.0 on ARM64 ([C5]). Rune's dev script writes 13.0 ([R2]). Homebrew lists macOS 15 or later as its requirement ([H1]), which **[Inferred]** is the effective floor for a Run.

### Which tools work on a bare Mac

**[Tested]** by code-signing identifier. Shims are the `xcode-select` stubs that need developer tools. **[Inferred]** They are the same on a bare Mac, because they live in the base system.

| Real base-system tools | Stubs that need developer tools |
|---|---|
| `curl`, `tar`, `unzip`, `ditto`, `hdiutil`, `xattr`, `codesign`, `spctl`, `open`, `plutil`, `softwareupdate`, `xcode-select`, `installer` | `git`, `python3`, `make`, `clang`, `otool`, `stapler` |

`notarytool` isn't in `/usr/bin` at all. **The entrypoint script can't use `git` until developer tools exist**, which is why the current `install.sh` has to install Command Line Tools before it can clone anything.

### Extraction can break the signature

**[Tested]** on Zed's app:

| Round trip | `codesign --verify --deep --strict` | `spctl` |
|---|---|---|
| `ditto -c -k` then `ditto -x -k` | Pass | Accepted |
| `tar -czf` then `tar -xzf` | Pass | Accepted |
| `ditto -c -k` then `unzip` | **Fail**: "a sealed resource is missing or invalid" | Not run |

I did not determine why. Whatever is used, the script should extract with the matching tool and verify afterward. Apple also says to use `ditto` for copying, because it preserves symlinks ([A4]).

## How comparable tools do it

**[Tested]** A search of the Zed, Homebrew, Ollama, Claude Code and rustup install scripts found no use of `xattr`, `spctl`, `codesign` or anything about notarization. None strips quarantine, because `curl` never set it. None of them verifies Apple's signature at install time.

| Tool | Command form | What happens on macOS | Integrity check in script |
|---|---|---|---|
| **Zed** (Rust GUI) | `curl ... \| sh` | Downloads a DMG, mounts it with `hdiutil`, `rm -rf` any old app, `ditto` to `/Applications`, symlinks the CLI into `~/.local/bin`. Doesn't launch. | None |
| **Ollama** (GUI + CLI) | `curl ... \| sh` | Downloads a zip, `unzip`, `mv` to `/Applications`, symlinks the CLI (`sudo` if needed), then `open -a`. | None found |
| **Homebrew** | `/bin/bash -c "$(curl ...)"` | Installs Command Line Tools with `softwareupdate`, falling back to `xcode-select --install` only with a terminal on stdin. Also ships a `.pkg` ([H1]). | Not checked |
| **Claude Code** (CLI) | `curl ... \| bash` | Downloads a binary into the home directory, and refuses to run under `sudo`. | SHA-256 from a manifest |
| **uv** (CLI) | `curl ... \| sh` ([C8]) | Installs to `~/.local/bin`. | Not checked |
| **winutil** (spirit of this project) | `irm ... \| iex` ([C9]) | Windows. Runs as Administrator. | Not checked |

Homebrew matters for the Catalog too. It deprecated `--no-quarantine`, and said it would disable casks failing Gatekeeper checks in September 2026 ([H2]). A cask of this app would need to pass ([H3]).

## Recommended entrypoint

Everything here is **[Inferred]** from the above, and the owner decides.

**The command** should keep Homebrew's form:

```sh
/bin/bash -c "$(curl -fsSL https://<stable-url>/install.sh)"
```

The script runs only after it has fully downloaded, and stdin stays attached to the terminal. With `curl | bash`, stdin is the script. `/bin/bash` here is 3.2 **[Tested]**, so the script must avoid newer syntax.

**The script** should do only this, with base-system tools:

1. Refuse anything but Darwin on `arm64`, or below the minimum macOS version.
2. If the app is already installed, open it and stop (see below).
3. Download the DMG with `curl -f` from a release URL. GitHub supports a stable `releases/latest/download/<asset>` form ([C10]).
4. Mount with `hdiutil attach -nobrowse -readonly`, and `ditto` the app to a temporary location.
5. Verify, and stop on any failure. Both commands behaved correctly in testing **[Tested]**, including rejecting a wrong Team ID (exit 3):
   ```sh
   codesign --verify --deep --strict \
     -R='anchor apple generic and certificate leaf[subject.OU] = "<TEAMID>"' App.app
   spctl -a -t exec App.app
   ```
6. Move it to `/Applications`, detach and `open` it. `/Applications` is `root:admin` with mode 775 **[Tested]**, so a standard user needs a clear message rather than a raw error.

The script should never strip quarantine or tell people to. Installing Command Line Tools moves into the app, because the app needs it for the Run anyway. That also removes the current script's wait loop, which never exits if the person dismisses the dialog (`install.sh:27`).

**Reruns** should open the existing app rather than replace it. Since macOS 13, modifying another team's app gets blocked, with a notification sending the person to System Settings ([W1], [S3]). **[Inferred]** Terminal running `rm -rf` on the installed app, as Zed's script does, could hit this; I did not test it. The app's own updater shouldn't, because "apps validly signed by the same developer account or team will continue to be able to update each other" ([W1], [A10]).

## Decisions for the owner

1. **Individual or organization enrollment.** An individual enrollment is simpler but likely puts your legal name in every copy of a public app. An organization needs a legal entity and D-U-N-S. **Recommendation:** decide this first, because the script hard-codes the Team ID. I did not verify whether Apple can convert an individual membership or what happens to the Team ID if so.
2. **Where the command's URL lives.** The map says the repo gets renamed. GitHub redirects after a rename, but those redirects stop working if the old name is ever reused ([C11]). **Recommendation:** a domain you control, set up before the command is first shared publicly.
3. **One artifact or two.** **Recommendation:** one DMG with both the app and the DMG stapled, serving script and browser users alike. A tarball avoids the mount but adds a second artifact.
4. **Whether the script verifies signatures.** Comparable tools don't. **Recommendation:** verify, because it restores the check `curl` skips and is only a few lines.
5. **Minimum macOS version.** **Recommendation:** 15, to match Homebrew, pending ticket #3's findings on Rune.

## Not verified

- **No bare Mac run.** Everything ran on a Mac with Command Line Tools. Ticket #9 covers clean-Mac testing.
- **No actual launches.** I never opened a downloaded app, so I did not observe any Gatekeeper dialog, whether from a real browser download or from `curl`. `spctl` gives Gatekeeper's assessment without launching, and it still accepted a copy of Zed after I added a quarantine attribute by hand.
- **No end-to-end notarization.** There is no Developer ID yet, so the CI sequence is assembled from Apple's documentation and from Zed's and Ghostty's scripts, and has not been run. Rune's entitlement needs are unknown until a test build is notarized.
- **Team key access level.** Apple's roles page has a "Notarize software" row, but the fetched table lost its check marks. Confirm the minimum on the first real submission.
- **Offline behavior.** I couldn't turn off networking on this Mac, so stapled and unstapled launches offline rest on Apple's documentation alone.
- **Enrollment timing.** I did not confirm how long approval takes.
- **App Management on script reruns.** The blocking behavior is documented for apps, and my application of it to a Terminal script is inferred.
- **Quoted web pages.** Apple's `developer.apple.com/documentation` pages, man pages and the install scripts were read directly. Apple Support, the forums, the membership pages and the other web pages were read through a page-fetching tool, so check a quote against the source before relying on its exact wording.
- **Rune is a moving target.** Findings are from `dev` at `ddf14140`.

## Sources

**Apple documentation**

- [A1] Notarizing macOS software before distribution
- [A2] Customizing the notarization workflow
- [A3] Resolving common notarization issues
- [A4] Creating distribution-signed code for macOS
- [A5] Packaging Mac software for distribution
- [A6] Placing content in a bundle
- [A7] Creating API keys for App Store Connect API
- [A8] Submitting software for notarization over the web
- [A9] macOS Big Sur 11.0.1 Universal Apps release notes
- [A10] NSUpdateSecurityPolicy
- [W1] WWDC22, What's new in privacy
- [N1] Updates to runtime protection in macOS Sequoia

**Apple Support and membership**

- [S1] Gatekeeper and runtime protection in macOS
- [S2] Safely open apps on your Mac
- [S3] Change Privacy & Security settings on Mac
- [P1] Enrollment
- [P2] Signing Mac software with Developer ID
- [P3] Create Developer ID certificates
- [P4] Certificates overview

**Apple Developer Forums posts**

- [F1] Resolving Trusted Execution Problems
- [F2] The Care and Feeding of Developer ID

**Comparable tools and CI**

- [C1] Zed `script/bundle-mac`
- [C2] Ghostty `release-tag.yml`
- [C3] GitHub, signing Xcode applications
- [C4] GitHub, choosing the runner for a job
- [C5] Rust, Apple Darwin platform support
- [C6] GitHub, using secrets in GitHub Actions
- [C7] `rcodesign` documentation
- [C8] uv installation
- [C9] winutil
- [C10] GitHub, linking to releases
- [C11] GitHub, renaming a repository
- [H1] Homebrew installation
- [H2] Homebrew 5.0.0
- [H3] Homebrew Acceptable Casks
- Install scripts, read directly: [Zed](https://zed.dev/install.sh), [Homebrew](https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh), [Ollama](https://ollama.com/install.sh), [Claude Code](https://claude.ai/install.sh), [rustup](https://sh.rustup.rs)

**Rune** at `ddf14140`

- [R1] `crates/rune/README.md`
- [R2] `scripts/mac_app_install.py`
- [R3] `crates/rune/Cargo.toml`

[A1]: https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution
[A2]: https://developer.apple.com/documentation/security/customizing-the-notarization-workflow
[A3]: https://developer.apple.com/documentation/security/resolving-common-notarization-issues
[A4]: https://developer.apple.com/documentation/xcode/creating-distribution-signed-code-for-the-mac
[A5]: https://developer.apple.com/documentation/xcode/packaging-mac-software-for-distribution
[A6]: https://developer.apple.com/documentation/bundleresources/placing-content-in-a-bundle
[A7]: https://developer.apple.com/documentation/appstoreconnectapi/creating-api-keys-for-app-store-connect-api
[A8]: https://developer.apple.com/documentation/notaryapi/submitting-software-for-notarization-over-the-web
[A9]: https://developer.apple.com/documentation/macos-release-notes/macos-big-sur-11_0_1-universal-apps-release-notes
[A10]: https://developer.apple.com/documentation/bundleresources/information-property-list/nsupdatesecuritypolicy
[W1]: https://developer.apple.com/videos/play/wwdc2022/10096/
[N1]: https://developer.apple.com/news/?id=saqachfa
[S1]: https://support.apple.com/guide/security/gatekeeper-and-runtime-protection-sec5599b66df/web
[S2]: https://support.apple.com/en-us/102445
[S3]: https://support.apple.com/guide/mac-help/change-privacy-security-settings-on-mac-mchl211c911f/mac
[P1]: https://developer.apple.com/programs/enroll/
[P2]: https://developer.apple.com/developer-id/
[P3]: https://developer.apple.com/help/account/certificates/create-developer-id-certificates/
[P4]: https://developer.apple.com/help/account/certificates/certificates-overview/
[F1]: https://developer.apple.com/forums/thread/706442
[F2]: https://developer.apple.com/forums/thread/732320
[C1]: https://github.com/zed-industries/zed/blob/main/script/bundle-mac
[C2]: https://github.com/ghostty-org/ghostty/blob/main/.github/workflows/release-tag.yml
[C3]: https://docs.github.com/en/actions/how-tos/deploy/deploy-to-third-party-platforms/sign-xcode-applications
[C4]: https://docs.github.com/en/actions/how-tos/write-workflows/choose-where-workflows-run/choose-the-runner-for-a-job
[C5]: https://doc.rust-lang.org/rustc/platform-support/apple-darwin.html
[C6]: https://docs.github.com/en/actions/security-for-github-actions/security-guides/using-secrets-in-github-actions
[C7]: https://gregoryszorc.com/docs/apple-codesign/main/
[C8]: https://docs.astral.sh/uv/getting-started/installation/
[C9]: https://github.com/ChrisTitusTech/winutil
[C10]: https://docs.github.com/en/repositories/releasing-projects-on-github/linking-to-releases
[C11]: https://docs.github.com/en/repositories/creating-and-managing-repositories/renaming-a-repository
[H1]: https://docs.brew.sh/Installation
[H2]: https://brew.sh/2025/11/12/homebrew-5.0.0/
[H3]: https://docs.brew.sh/Acceptable-Casks
[R1]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune/README.md
[R2]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/scripts/mac_app_install.py
[R3]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune/Cargo.toml

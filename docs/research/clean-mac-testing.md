# Testing a Run on a clean Mac, repeatably

Research for ticket [#9](https://github.com/nullaroni/macbook-setup/issues/9), part of the map in [#2](https://github.com/nullaroni/macbook-setup/issues/2). Sources were read on 2026-11-06, and all prices are as of that date.

## Short answer

No single option covers everything, so the workable approach is layers:

| Layer | Runs where | Best at | Can't do |
| --- | --- | --- | --- |
| Rune headless | Any Mac with a working GPU device (hosted runner support unconfirmed) | UI states and flows, with fake engine results | Real installs, real windows, OS prompts |
| GitHub-hosted macOS runner | CI, free for this public repo | Fast checks on every PR, and Items that don't depend on a bare start | Bare-Mac behavior: Homebrew, Command Line Tools and Xcode are already there |
| Tart VM from a `vanilla` image | The owner's Mac, or a self-hosted Mac | Full Runs from a near-bare state, repeated from a pinned image | Mac App Store Items, and running on GitHub-hosted runners |
| Real Mac, erased | A spare Apple Silicon Mac, by hand | The final check on what VMs can't show | Cheap or unattended repetition |

**Recommendation (yours to accept or change):** run Rune headless and CLI checks on hosted runners for every PR. Run full Runs in Tart VMs before merging risky changes and before each release. Run a short manual checklist on an erased real Mac before each release.

Nothing here was run. This ticket's constraints ruled out installing tools or starting VMs, so behavior in the matrix below is reasoned from documents and source code, not observed.

## How close each option gets to a bare Mac

### Tart `vanilla` is close, but it is not bare

The [`vanilla-tahoe` template](https://github.com/cirruslabs/macos-image-templates/blob/main/templates/vanilla-tahoe.pkr.hcl) installs macOS from Apple's restore image and adds no packages. The [templates README](https://github.com/cirruslabs/macos-image-templates) describes it as having "no additional software preinstalled". The template does change the system, and these changes matter for this app:

| Change in the image | What it hides from a test |
| --- | --- |
| Gatekeeper is disabled (`spctl --global-disable`, then checked) | The notarized first launch and quarantine behavior |
| Passwordless sudo for `admin` | The app's real admin password prompt |
| Remote Login and Screen Sharing are on | Nothing in the app, but a real Mac has both off by default |
| Auto-login, no screen lock, no sleep, no screensaver | Login and lock-screen interruptions during a long Run |
| FileVault was declined, no Apple Account signed in, time zone is UTC | FileVault cases, and anything assuming a signed-in user |
| Safari was launched once and `safaridriver` enabled | First-launch Safari state |
| Keyboard navigation turned on (`AppleKeyboardUIMode`) | The macOS default for that setting |

The login is `admin` / `admin`, per the [Tart quick start](https://tart.run/quick-start/). The [`vanilla-golden-gate` template](https://github.com/cirruslabs/macos-image-templates/blob/main/templates/vanilla-golden-gate.pkr.hcl) (macOS 27.2) also disables Gatekeeper.

**Inferred:** the project could close most of this gap with a thin extra image layer or test setup step that turns Gatekeeper back on and removes the sudoers file. I did not confirm that either step works unattended. `spctl --global-enable` over SSH is the first thing to test.

For a state closer to a new Mac, `tart create --from-ipsw=latest` stops at Setup Assistant, which must then be [completed by hand](https://tart.run/quick-start/) or scripted the way the template does it. The template drives it with timed keystrokes for each screen, so that script will need upkeep with each macOS release (inferred).

### GitHub-hosted runners are far from bare

The [macOS 15 arm64 image](https://github.com/actions/runner-images/blob/main/images/macos/macos-15-arm64-Readme.md) ships with Homebrew, Command Line Tools, several Xcode versions, Git, Node, Python, Ruby, Java, Rust and more. Its [build scripts](https://github.com/actions/runner-images/tree/main/images/macos/scripts/build) also:

- pre-grant privacy permissions, including Accessibility for `/bin/bash`, `osascript` and Terminal ([`configure-tccdb-macos.sh`](https://github.com/actions/runner-images/blob/main/images/macos/scripts/build/configure-tccdb-macos.sh))
- set up auto-login ([`configure-autologin.sh`](https://github.com/actions/runner-images/blob/main/images/macos/scripts/build/configure-autologin.sh))
- add passwordless sudo ([`configure-machine.sh`](https://github.com/actions/runner-images/blob/main/images/macos/scripts/build/configure-machine.sh))

Each job does get a [fresh VM](https://docs.github.com/en/actions/reference/runners/github-hosted-runners), so runs start from the same image family, but [the image changes about weekly](https://github.com/actions/runner-images) outside the project's control. **Inferred:** uninstalling Homebrew at the start of a job would approximate a bare start, but leftovers in the image would limit how much a pass proves.

I could not determine Gatekeeper or SIP state on hosted runners. The scripts branch on `csrutil status`, so they don't settle it, and the docs don't say.

### A real, erased Mac is the only truly bare option

[Erase All Content and Settings](https://support.apple.com/en-us/102664) removes data, settings and apps but keeps the installed macOS version, and it needs the login and Apple Account passwords. [An Apple Configurator restore](https://support.apple.com/en-us/108900) returns the Mac to factory settings, and needs a second Mac on macOS 14 or later, about 60 GB free and a USB-C data cable. After an erase the Mac goes to a setup assistant, and a restore should leave it in the same position (inferred), so someone has to complete setup. I did not research unattended enrollment through device management.

## Apple's limits and licensing

Not legal advice. The readings below are plain-language interpretations of the text.

**Verified:**

- **Two VMs per Mac.** [The macOS Tahoe license](https://www.apple.com/legal/sla/docs/macOSTahoe.pdf), Section 2B(iii), allows "up to two (2) additional copies or instances" in VMs on each Apple-branded computer "you own or control that is already running the Apple Software". The [macOS 27 license](https://www.apple.com/legal/sla/docs/macOS27.pdf) has the same limit, adding "except as otherwise provided in writing, signed, or issued by an authorized representative of Apple".
- **Allowed purposes.** They include "software development" and "testing during software development". Service bureau, time-sharing and similar services are excluded, apart from what Section 3 allows.
- **Enforcement.** An [Apple engineer on the developer forums](https://developer.apple.com/forums/thread/729580) said in May 2023: "The limit of 2 virtual Macs is part of the macOS End-User License Agreement." That thread's author could not start a third VM. The framework has a [`virtualMachineLimitExceeded`](https://developer.apple.com/documentation/virtualization/vzerror/code/virtualmachinelimitexceeded) error, but its docs give no number.
- **Redistribution.** The license also says "you may not rent, lease, lend, sell, redistribute or sublicense the Apple Software", except as Section 3 allows.
- **Leasing.** Section 3 requires leases of at least 24 consecutive hours with sole use. [AWS cites this license](https://aws.amazon.com/ec2/instance-types/mac/) for its 24-hour minimum on EC2 Mac.
- **No nested macOS VMs.** [GitHub says](https://docs.github.com/en/actions/reference/runners/github-hosted-runners) "Nested-virtualization is not supported due to the limitation of Apple's Virtualization Framework." [Tart's FAQ](https://tart.run/faq/) limits nested virtualization to Linux guests. [Apple's API for it](https://developer.apple.com/documentation/virtualization/vzgenericplatformconfiguration/isnestedvirtualizationsupported) sits on the generic platform and needs an M3 or later.

**Inferred:**

- Testing this app in VMs on a Mac the project owner owns fits "testing during software development". Each contributor would be covered on their own Mac the same way.
- Tart can't run on GitHub-hosted macOS runners, because that would need nested macOS VMs. I found no source that states this about Tart directly.
- Publishing a project-built macOS VM image to a public registry is questionable under the redistribution clause. Keeping any custom image local or private avoids the question. Cirrus Labs does publish such images, but I did not find anything explaining the basis for that.

## Sign-in inside VMs

**Verified:**

- **Apple Account works on macOS 15 and later, with gaps.** [Apple's support article](https://support.apple.com/en-us/120468) (published March 18, 2025) requires Sequoia or later on host and guest. It lists these as unavailable: iCloud Backup, Find My, Apple Wallet, and "Apple Media Services, such as apps, movies, music, books, and more".
- **Identity is tied to the host.** [Apple's developer docs](https://developer.apple.com/documentation/virtualization/using-icloud-with-macos-virtual-machines) say the VM's identity derives from the host's Secure Enclave. Moving a VM to another Mac, or starting a second clone while one is running, creates a new identity that requires signing in again. The VM must also be created from a macOS 15 or later restore image, not upgraded from an older one.
- **Mac App Store.** [Parallels' knowledge base](https://kb.parallels.com/en/131160) (reviewed July 7, 2026) says App Store sign-in "is still not supported" and blames the Virtualization framework. This is a vendor's statement. Apple's closest wording is the "Apple Media Services" line above.
- **Older Tart reports are out of date.** [Tart issue #722](https://github.com/openai/tart/issues/722) says Apple ID sign-in is unsupported, but it dates from 2024, before macOS 15.
- **Keychain over SSH.** [Tart's FAQ](https://tart.run/faq/) says that from macOS 15 headless VMs need an unlocked `login.keychain`, or errors appear. Anything the app stores in the keychain during an SSH-driven test could hit this.

**Inferred:**

- Mac App Store Items can't be tested in a VM. Test them on the real Mac.
- Apple Account sign-ins shouldn't be part of unattended tests, because they involve two-factor prompts, and a saved session wouldn't survive a clone reliably. Testing that the correct Needs-You Queue entries appear covers most of the app's own logic.
- GitHub browser sign-in can be done by hand in a VM over VNC. For unattended runs, a fake sign-in at the engine boundary or a dedicated test account's token would work. [GitHub doesn't pass secrets](https://docs.github.com/en/actions/how-tos/write-workflows/choose-what-workflows-do/use-secrets), other than `GITHUB_TOKEN`, to workflows triggered from a fork, so a job using a test account's token couldn't run on fork PRs.

**Not tested:** signing in to an Apple Account inside a Tart `vanilla` image, or on a GitHub-hosted runner. I found no documentation of either.

## Rune headless

**Verified from the [Rune README](https://github.com/anomalyco/rune) and [crate docs](https://github.com/anomalyco/rune/blob/dev/crates/rune/README.md), at `dev` commit `ddf1414025`:**

- Every app can run headless: render to PNG (`--snap`), dump the element tree as JSON (`--tree`), and play scripted input (`--script`) on a fixed virtual clock.
- Scripts can drive clicks, keys, text, menus and file dialogs.
- Headless runs use the same renderer. They ignore window requests, never write to the system clipboard, and leave native views out of offscreen output.
- It is pre-release, and the project says to pin a `dev` rev. It is MIT licensed.

**Not verified:** whether it works on GitHub-hosted macOS runners. Its docs describe an offscreen GPU readback, and the [plan](https://github.com/anomalyco/rune/blob/dev/PLAN.md) mentions offscreen Metal. Rune's own [workflows](https://github.com/anomalyco/rune/tree/dev/.github/workflows) only build sites on Ubuntu, so there's no public example to point at. The only macOS GPU acceleration I found in GitHub's docs is on its [paid 5-core size](https://docs.github.com/en/actions/reference/runners/larger-runners). The same question applies inside a Tart VM. A one-job spike would answer both.

**Inferred:** headless mode tests the UI, not provisioning. It's useful when the engine sits behind a boundary that tests can fake, so a script can walk through Config selection, a Run in progress, a failed Item and the Needs-You Queue without installing anything.

## Which parts each option can test

All of this is inferred from the facts above, not observed. ✅ realistic, 🟡 partial or with extra setup, ❌ no.

| Part of the app | Rune headless | Hosted runner | Tart vanilla | Real erased Mac |
| --- | --- | --- | --- | --- |
| UI states and flows | ✅ | ✅ (if a GPU device is available) | ✅ | ✅ |
| Real windows, focus, native dialogs | ❌ | 🟡 | ✅ over VNC | ✅ |
| Config parsing, Drift and planning logic | 🟡 UI side only | ✅ | ✅ | ✅ |
| Single install command from bare | ❌ | ❌ | 🟡 | ✅ |
| Installing Command Line Tools | ❌ | ❌ already there | 🟡 see below | ✅ |
| Installing Homebrew | ❌ | ❌ already there | ✅ | ✅ |
| Formulae and casks | ❌ | 🟡 limited disk | ✅ | ✅ |
| macOS settings Items | ❌ | 🟡 | ✅ | ✅ |
| Dotfiles links and backups | ❌ | ✅ | ✅ | ✅ |
| Real admin password prompt | ❌ | ❌ | 🟡 after removing sudoers file | ✅ |
| Notarized first launch, Gatekeeper | ❌ | ❌ unknown state | 🟡 after re-enabling | ✅ |
| Privacy (TCC) prompts | ❌ | ❌ pre-granted | ✅ over VNC | ✅ |
| Restart during a Run | ❌ | ❌ | 🟡 | ✅ |
| Rerun with no changes | ❌ | ✅ | ✅ | ✅ |
| One failed Item doesn't stop a Run | 🟡 UI side only | ✅ | ✅ | ✅ |
| Needs-You Queue contents | ✅ with fakes | ✅ | ✅ | ✅ |
| Doing the Needs-You steps | ❌ | ❌ | 🟡 some | ✅ |
| Mac App Store Items | ❌ | ❌ | ❌ | ✅ |
| Apple Account, iCloud | ❌ | ❌ | 🟡 by hand | ✅ |
| Full Xcode install | ❌ | ❌ already there | 🟡 by hand, disk heavy | ✅ |
| FileVault, Find My | ❌ | ❌ | ❌ | ✅ |

Two notes from this repo:

- [`scripts/xcode.sh`](../../scripts/xcode.sh) and [`install.sh`](../../install.sh) use `xcode-select --install`, which opens a GUI dialog and then waits. That would hang in an SSH-driven VM test unless someone clicks through it over VNC. GitHub's image build [installs Command Line Tools with a script](https://github.com/actions/runner-images/blob/main/images/macos/scripts/build/install-xcode-clt.sh), so there is a precedent to read if the engine should avoid the dialog.
- Runs depend on live Homebrew and vendor downloads, so a pinned VM image makes the start state repeatable, but not the results. Failures from upstream changes will need telling apart from app bugs (inferred).

## Running it repeatably with Tart

**Verified from [Tart's source](https://github.com/openai/tart/tree/main/Sources/tart/Commands) and [docs](https://tart.run/quick-start/):**

- `tart clone` from a registry image pulls it. Local clones use APFS copy-on-write, so they're quick and only changes take space.
- `tart run --no-graphics` starts a VM with no window. `tart ip` gets its address for SSH or VNC. `tart delete` removes it.
- `tart exec` needs the Tart Guest Agent, which [non-vanilla images include](https://github.com/openai/tart/blob/main/Sources/tart/Commands/Exec.swift). With `vanilla`, use SSH.
- Image references accept [`@sha256:` digests](https://github.com/openai/tart/blob/main/Sources/tart/OCI/RemoteName.swift), so a test can pin an exact image.
- A VM defaults to 2 CPUs and 4 GB of memory, changed with `tart set`.

**Measured from the `ghcr.io` manifests:**

| Image (`latest`) | Download | Disk | Uploaded |
| --- | --- | --- | --- |
| `macos-tahoe-vanilla` | 24.0 GB | 50 GB | 2026-08-18 |
| `macos-golden-gate-vanilla` | 30.0 GB | 35 GB | 2026-10-31 |
| `macos-tahoe-base` (for comparison) | 26.4 GB | 50 GB | 2026-10-15 |

A likely test loop, inferred and not run:

```sh
tart clone ghcr.io/cirruslabs/macos-tahoe-vanilla@sha256:<digest> run-test
tart run --no-graphics run-test &
ssh admin@"$(tart ip run-test)" '<single install command, then a headless Run>'
# collect logs and assert on the Machine's state
tart stop run-test && tart delete run-test
```

### Tart's license and owner have changed

The sources disagree, so this needs care:

- [Cirrus Labs announced on April 7, 2026](https://cirruslabs.org/) that it is joining OpenAI, had stopped charging licensing fees, and would relicense its tools.
- [The `LICENSE` file](https://github.com/openai/tart/blob/main/LICENSE) has been FSL-1.1-ALv2 since [June 5, 2026](https://github.com/openai/tart/commit/5ad172e7). The repo now lives at `openai/tart`, and `docs/licensing.md` was [removed](https://github.com/openai/tart/commits/main/docs/licensing.md) from it shortly after.
- [tart.run/licensing](https://tart.run/licensing/) still describes the older Fair Source terms, with a 100-core limit and paid tiers. It looks stale, since its source file is gone from the repo.

FSL permits any purpose except a "Competing Use", and specifically includes "your internal use and access". **Inferred:** testing this app with Tart doesn't compete with Tart. Use a release from after the relicense, such as 2.41.1 (October 15, 2026), and don't rely on tart.run/licensing.

Also new from that announcement: [Cirrus Runners](https://cirrus-runners.app/) no longer accepts new customers, and Cirrus CI shut down on June 1, 2026, so neither is an option. The [image templates](https://github.com/cirruslabs/macos-image-templates) are MIT licensed and were last updated on October 31, 2026. I found no commitment on long-term image upkeep, so it would be sensible to confirm that a template can be built locally in case published images stop.

### Using Tart in CI

Tart in CI means a self-hosted runner on a Mac the project controls. GitHub's docs [recommend](https://docs.github.com/en/actions/how-tos/manage-runners/self-hosted-runners/manage-access) self-hosted runners "only ... with private repositories", because forks of a public repo can run code on the machine through a pull request. The [security guide](https://docs.github.com/en/actions/reference/security/secure-use) says they should "almost never" be used for public repositories.

This repo is public. **Inferred:** a disposable VM per job reduces the exposure but doesn't remove it, since the runner still lives on a real Mac. Safer choices are running Tart locally by script, or limiting the self-hosted job to triggers only maintainers can start, such as manual dispatch or release tags.

## Approximate cost

As of 2026-11-06, in USD.

| Option | Cost | Source and notes |
| --- | --- | --- |
| Hosted macOS runner, public repo | $0, unlimited minutes | [GitHub](https://docs.github.com/en/actions/reference/runners/github-hosted-runners). Up to [5 concurrent macOS jobs](https://docs.github.com/en/actions/reference/limits) and 6 hours per job. |
| Hosted macOS runner, private repo | $0.062 per minute | [GitHub pricing](https://docs.github.com/en/billing/reference/actions-runner-pricing). Applies only if a test moves to a private repo. |
| Larger macOS runner (5-core) | $0.102 per minute | Same page. [Not free on public repos](https://docs.github.com/en/actions/reference/runners/larger-runners), and Team or Enterprise Cloud only. |
| Tart | $0 in license fees | Per the Cirrus Labs announcement and FSL text above. |
| Tart image storage and pulls | $0 currently | [GitHub Packages](https://docs.github.com/en/billing/concepts/product-billing/github-packages) is free for public packages. Container storage is "currently free". |
| Self-hosted runner minutes | $0 | [GitHub billing](https://docs.github.com/en/billing/concepts/product-billing/github-actions) lists these as free. |
| Host for Tart or a real-Mac test | $0 on an existing Mac, about $899 and up for a dedicated Mac mini | $899 is the lowest price in [Apple's store page](https://www.apple.com/shop/buy-mac/mac-mini) HTML. I couldn't match it to a configuration. A base Mac mini lists 16 GB of memory and 256 GB of storage, which is tight for 24–30 GB images with 35–50 GB disks (inferred). |
| AWS EC2 Mac | 24-hour minimum, price not retrieved | [AWS](https://aws.amazon.com/ec2/dedicated-hosts/pricing/) points to its calculator. A 24-hour minimum suits occasional real-hardware runs, not every commit (inferred). |

The standard arm64 hosted runner has [3 CPU cores, 7 GB of memory and a 14 GB disk](https://docs.github.com/en/actions/reference/runners/github-hosted-runners). **Inferred:** a large Run, particularly with full Xcode, may not fit. I did not measure the actual free space or how long a Run takes in any of these environments.

## Decisions for the owner

1. **Release gate.** Which layers must pass before a release, and is the manual real-Mac check required every time?
2. **Hardware.** Use an existing Mac as the VM host, or buy a dedicated one? A real-Mac check also needs a Mac that can be erased.
3. **Tart in CI.** Keep Tart local by script, or set up a self-hosted runner with maintainer-only triggers on this public repo?
4. **How bare is bare enough.** Accept stock `vanilla`, or maintain a project layer that restores Gatekeeper and the sudo prompt?
5. **macOS versions.** Which versions must each release be tested on? Each adds a roughly 24–30 GB image and more run time.
6. **Command Line Tools.** Should the engine keep the GUI dialog or use a scriptable install? This affects how much of a Run can be unattended.

## Suggested first spikes

These are small checks that would replace the largest assumptions here:

1. Run a Rune headless `--snap` and `--tree` on `macos-latest`, to see whether a GPU device is available.
2. Run the current `bootstrap.sh` in a `macos-tahoe-vanilla` clone, recording time, disk use and every prompt.
3. In a clone, try `sudo spctl --global-enable` over SSH and open a notarized, quarantined app.
4. In a clone, try signing in to an Apple Account, then try the App Store, to check Apple's and Parallels' statements on Tart.

## What could not be verified

- Any real behavior: no VM or runner job was run for this research.
- Gatekeeper and SIP state on GitHub-hosted runners.
- Rune headless on hosted runners or in VMs.
- Apple Account or App Store sign-in in Tart images specifically. Apple has no explicit statement on App Store sign-in in VMs beyond the "Apple Media Services" line.
- `spctl --global-enable` working unattended.
- Which Mac mini configuration costs $899, and AWS EC2 Mac prices.
- A 2026 self-hosted runner fee: I recalled one being announced and postponed, but current GitHub docs list self-hosted minutes as free, and I found no announcement in the changelog entries I could read.
- The legal basis for public macOS VM images, and how GitHub licenses its hosted VMs. I did not assess either.
- [Tart's old Orchard post](https://tart.run/blog/) on the two-VM limit returned 404. Apple's license is the stronger source for this anyway.

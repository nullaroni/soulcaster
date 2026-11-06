# Which macOS settings can be reliably read and written?

Research for [ticket #6](https://github.com/nullaroni/macbook-setup/issues/6), a child of the [map](https://github.com/nullaroni/macbook-setup/issues/2). Researched 2026-11-06 against macOS 27.2 "Golden Gate" (build 26B8125).

## Answer

Most common Dock, Finder, keyboard, trackpad and screenshot settings are stored in the user defaults system, which can be read and written without admin rights. As far as the sources show, they also need no privacy permission. **A successful read-back doesn't prove that a setting still works.** Apple has no public contract for most of these keys, and a dead key would still store and return its value, so a Drift check would pass on it.

The settings fall into four groups:

| Group | Settings | Status |
|---|---|---|
| Strongest evidence | Dock appearance and contents, Finder desktop icons | Apple documents the key as an MDM payload, and most of the names were found in the OS binary |
| Probably sound, undocumented | Keyboard, trackpad, most Finder keys, hot corners, Dock Spaces options | The key name is in the current OS, but Apple makes no promise about it |
| Unconfirmed | Menu bar visibility, some screenshot keys | Storage location or scope couldn't be confirmed without changing a setting |
| Broken or unsupported | `sfltool add-item`, `Launchpad.app` in the Dock, most of `menuExtras`, `spctl` global state, `com.apple.alf.plist` | See [what has stopped working](#what-has-stopped-working) |

## How far this evidence goes

**The read-only checks only confirm that a key exists on this one macOS version.** Nothing was written, no process was restarted, and no repo script was run. So:

- **No row proves a write takes effect.** The "Apply" column comes from the repo's scripts and from nix-darwin, not from a test.
- **A key set on the test Mac may be a leftover.** These same scripts could have written it, so "set on this Mac" is the weakest signal below.
- **A key name found in a binary shows the OS mentions it, not that the OS still honors it.** Finding a name in the shared cache also doesn't show which component uses it.
- **A name not found proves little.** Swift can store strings of up to 15 bytes inline, and names can be assembled at runtime. See the `NSStatusItem` case below.
- **Earlier macOS versions weren't tested.** They appear only where Apple's notes mention them.

### Evidence codes

| Code | Meaning | Verified how |
|---|---|---|
| **D** | Apple documents the key in its [device-management schemas](https://github.com/apple/device-management/tree/release/mdm/profiles) | Read the YAML on the `release` branch |
| **G** | Apple's user guide documents the setting, but not the key | Read the [Mac User Guide](https://support.apple.com/guide/mac-help/welcome/mac) |
| **B** | The literal key name is in an app binary on macOS 27.2 | `grep` of the owning app's binary |
| **C** | The literal key name is in the dyld shared cache on macOS 27.2, owner not identified | `grep` of the cache |
| **H** | The key is set on the test Mac, with the stored type shown | `defaults read-type`, which doesn't print values |
| **N** | [nix-darwin](https://github.com/nix-darwin/nix-darwin/tree/master/modules/system/defaults) exposes the key | Secondary source: read its modules, not tested |

An MDM schema documents what an MDM server can manage. It isn't a promise about what a `defaults write` does, so **D** is strong evidence but not a guarantee.

## Facts that apply to all defaults-based settings

### Mechanism

- **Use the defaults system, not the plist files.** `CFPreferences` and `NSUserDefaults` are served by `cfprefsd`, per `cfprefsd(8)`. Apple says [cached preference data doesn't automatically pick up external changes](https://developer.apple.com/documentation/corefoundation/cfpreferencesappsynchronize(_:)). dockutil's source reads through `defaults` because ["we can't trust what is on disk"](https://github.com/kcrawford/dockutil/blob/main/Sources/DockUtil/Dock.swift).
- **The `defaults` CLI and the CFPreferences API both work.** [dockutil](https://github.com/kcrawford/dockutil/blob/main/Sources/DockUtil/Dock.swift) uses [`CFPreferencesCopyKeyList`](https://developer.apple.com/documentation/corefoundation/cfpreferencescopykeylist(_:_:_:)), `CFPreferencesCopyAppValue`, `CFPreferencesSetAppValue` and `CFPreferencesSynchronize`, so a compiled engine doesn't have to shell out. That it works from Rust is an inference; it wasn't tested.
- **Domain form only.** `defaults(1)` warns that it will change in an upcoming major release to "only operate on preferences domains", so file-path arguments shouldn't be relied on.

### Reading back for Drift

1. **Unset isn't the same as false.** Many common keys were unset on the test Mac, and the owning app then uses its built-in default. Apple doesn't publish those defaults for undocumented keys.
2. **Types vary, so compare after normalizing.** `com.apple.AppleMultitouchTrackpad Clicking` reads back as an integer, while `TrackpadThreeFingerDrag` in the same domain is a boolean (**H**).
3. **Scope matters.** `defaults` can address any host or `-currentHost`, per `defaults(1)`, and a key in one scope doesn't appear in the other. Each Catalog Item would need to record its scope. `com.apple.Spotlight MenuItemHidden` is set in one scope on the test Mac and not the other.
4. **Admin-managed values can't be changed by the user.** Apple provides [`CFPreferencesAppValueIsForced`](https://developer.apple.com/documentation/corefoundation/cfpreferencesappvalueisforced(_:_:)) and [`objectIsForced(forKey:)`](https://developer.apple.com/documentation/foundation/userdefaults/objectisforced(forkey:)) to detect this. Not tested on a managed Machine.
5. **Read-back shows what's stored, not what's happening.** macOS 27 adds a case: the [menu bar now hides items behind an overflow indicator](https://support.apple.com/en-us/148830) when space runs short, so a "visible" item may not be on screen.

### Applying a change

- **Running apps can overwrite a write.** `defaults(1)` says a running app "won't see the change and might even overwrite the default". So the write-then-restart order in the repo's scripts isn't guaranteed to win. Quitting the app before writing may be safer, but that's an inference.
- **Restarting is safe for the system agents.** The launchd plists for Dock, Finder, SystemUIServer and ControlCenter all set `KeepAlive` with `SuccessfulExit = false` (verified in `/System/Library/LaunchAgents`). The expected result is that launchd relaunches them after `killall`, though this wasn't run. A [comment on nix-darwin](https://github.com/nix-darwin/nix-darwin/issues/658) says restarting Finder or Dock makes windows jump around, which matters if someone is using the Mac during a Run.
- **`killall cfprefsd` has no documented purpose here.** `macos.sh` does it, but no source I read calls for it. `cfprefsd(8)` says only that users shouldn't run it manually.
- **`activateSettings -u` is private.** It's a binary inside `SystemAdministration.framework`, present on 27.2, with no man page. Its strings mention trackpad preferences. [Users report](https://github.com/nix-darwin/nix-darwin/issues/658) that it applies trackpad settings without a logout. It wasn't run here, and nix-darwin's current source doesn't call it.

## Settings tables

"Apply" is the expected step from the sources above, not a test result. "Permissions" means none beyond the logged-in user unless noted. All rows read back through `defaults read` or CFPreferences unless noted.

### Dock (`com.apple.dock`)

| Setting | Key | Type | Apply | Evidence |
|---|---|---|---|---|
| Position | `orientation` (`bottom`, `left`, `right`) | string | Restart Dock | D B H G |
| Auto-hide | `autohide` | bool | Restart Dock | D B H G |
| Icon size | `tilesize` (16–128) | int | Restart Dock | D B |
| Magnification | `magnification`, `largesize` | bool, int | Restart Dock | D B |
| Minimize effect | `mineffect` | string | Restart Dock | D B G |
| Minimize into app icon | `minimize-to-application` | bool | Restart Dock | D B G |
| Launch animation | `launchanim` | bool | Restart Dock | D B |
| Show suggested and recent apps | `show-recents` | bool | Restart Dock | D B H G |
| Open-apps indicators | `show-process-indicators` | bool | Restart Dock | D B |
| Dock contents | `persistent-apps`, `persistent-others` | array of dicts | Restart Dock | D B H |
| Auto-hide delay and speed | `autohide-delay`, `autohide-time-modifier` | float | Restart Dock | B N |
| Show only open apps | `static-only` | bool | Restart Dock | D B |
| Hot corners | `wvous-{tl,tr,bl,br}-corner`, `-modifier` | int | Restart Dock | B H G N |
| Group windows by app | `expose-group-apps` | bool | Restart Dock | B H G N |
| Rearrange Spaces | `mru-spaces` | bool | Restart Dock | H G N (name not found, but short enough to be inlined) |

- **Dock contents are hard to compare.** Each entry is a nested dict, so Drift would have to compare on a stable field such as the app's URL or bundle ID. That's an inference.
- **Hot corners store a number with no Apple-published meaning.** nix-darwin lists [`1` as Disabled](https://github.com/nix-darwin/nix-darwin/blob/master/modules/system/defaults/dock.nix), and doesn't list `0`, which is the value `macos.sh` writes.

### Finder (`com.apple.finder`, restart Finder)

| Setting | Key | Type | Evidence |
|---|---|---|---|
| Desktop icons | `ShowExternalHardDrivesOnDesktop` | bool | D B H N |
| Desktop icons | `ShowHardDrivesOnDesktop`, `ShowRemovableMediaOnDesktop` | bool | D H N |
| Desktop icons | `ShowMountedServersOnDesktop` | bool | D N |
| New window target | `NewWindowTarget`, `NewWindowTargetPath` | string | B H N |
| Default view | `FXPreferredViewStyle` | string | B H N |
| Search scope | `FXDefaultSearchScope` | string | B H N |
| Path bar, status bar | `ShowPathbar`, `ShowStatusBar` | bool | B H N |
| Recent tags | `ShowRecentTags` | bool | B H |
| Info pane sections | `FXInfoPanesExpanded` | dict | B H |
| Hidden files | `AppleShowAllFiles` | bool | B N |
| Sort folders first | `_FXSortFoldersFirst` | bool | B N |
| POSIX path in title | `_FXShowPosixPathInTitle` | bool | B N |
| Extension change warning | `FXEnableExtensionChangeWarning` | bool | B N |
| Quit menu item | `QuitMenuItem` | bool | B N |
| Desktop icons on or off | `CreateDesktop` | bool | B N |
| Show all extensions | `NSGlobalDomain AppleShowAllExtensions` | bool | C N |
| No `.DS_Store` on network or USB | `com.apple.desktopservices DSDontWriteNetworkStores`, `DSDontWriteUSBStores` | bool | C for the network key only |
| Show `~/Library` | `chflags nohidden ~/Library`, read with `ls -lOd` or `stat -f %Sf` | file flag | `chflags(1)`; flag read here |

- **`NewWindowTargetPath` is machine-specific.** It stores an absolute path containing the home folder, so a shared Config would need a placeholder. That's an inference.
- **Known issue.** The [macOS 26 notes](https://developer.apple.com/documentation/macos-release-notes/macos-26-release-notes) say that with the path bar or status bar on, list view might not let you reach the last item (151917092). The workaround is to turn them off temporarily. The issue ID doesn't appear in the 26.1 to 26.5, 27 or 27.2 notes, as either fixed or still open. There's no separate 27.1 page at the URL I tried.

### Keyboard (global domain unless noted)

| Setting | Key | Type | Apply | Evidence |
|---|---|---|---|---|
| Key repeat rate | `KeyRepeat` | int | Log out, per `keyboard.sh` | C H G N |
| Delay until repeat | `InitialKeyRepeat` | int | Log out, per `keyboard.sh` | C H G N |
| Press and hold accents | `ApplePressAndHoldEnabled` | bool | Restart the app that reads it (inference) | C H N |
| Full keyboard navigation | `AppleKeyboardUIMode` | int | Unverified | C H N |
| Auto-correct and substitutions | `NSAutomatic{SpellingCorrection,Capitalization,DashSubstitution,QuoteSubstitution,PeriodSubstitution}Enabled` | bool | Restart the app that reads it (inference) | C (H for capitalization and period) N |
| Globe/fn key action | `com.apple.HIToolbox AppleFnUsageType` | int | Unverified | C N |
| Key remapping | `hidutil property --set`, read with `--get "UserKeyMapping"` | JSON | Immediate, per [TN2450](https://developer.apple.com/library/archive/technotes/tn2450/_index.html) | Apple TN from macOS 10.12; tool still present |

- **Apple doesn't publish the valid values.** Its guide describes [sliders only](https://support.apple.com/guide/mac-help/change-keyboard-settings-kbdm162/mac). So `KeyRepeat 2` and `InitialKeyRepeat 15` being the "fastest" and "shortest" is the script's claim, not Apple's.
- **Whether `hidutil` remaps survive a reboot isn't stated in TN2450.** If they don't, the mapping has to be reapplied at login, for example by a LaunchAgent.

### Trackpad (restart or log out, unverified)

| Setting | Key | Type | Evidence |
|---|---|---|---|
| Tap to click | `com.apple.AppleMultitouchTrackpad Clicking`, also `com.apple.driver.AppleBluetoothMultitouch.trackpad Clicking` | int | H G N |
| Three-finger drag | `TrackpadThreeFingerDrag`, both domains | bool | C H N |
| Right click | `TrackpadRightClick`, both domains | bool | H N |
| Click pressure (meaning inferred from the name) | `FirstClickThreshold` | int | C H |
| Silent clicking (meaning not confirmed) | `ActuationStrength` | unverified | C |
| Force click | `NSGlobalDomain com.apple.trackpad.forceClick` | bool | H G |
| Natural scrolling | `NSGlobalDomain com.apple.swipescrolldirection` | bool | C G N |
| Tap to click, global key | `NSGlobalDomain com.apple.mouse.tapBehavior` | int | C N |

- **Trackpad settings have two domains.** nix-darwin [writes both](https://github.com/nix-darwin/nix-darwin/blob/master/modules/system/defaults-write.nix), one for the built-in trackpad and one for Bluetooth. Drift would need to read both.
- **Users report these don't apply right away.** One [nix-darwin report](https://github.com/nix-darwin/nix-darwin/issues/658) has `Clicking` taking effect only after `activateSettings -u`.

### Screenshots (`com.apple.screencapture`, restart unverified)

| Setting | Key | Type | Evidence |
|---|---|---|---|
| Save location | `location` | string | B (a generic word, so weak) G N |
| File format | `type` | string | N |
| Disable window shadow | `disable-shadow` | bool | C N |
| Floating thumbnail | `show-thumbnail` | bool | G N (name not found, but short enough to be inlined) |
| Date in file name | `include-date` | bool | N (same) |

The domain name is in `screencapture` and `screencaptureui`, but none of these keys was set on the test Mac, and `screencapture(1)` doesn't mention them. This group needs a toggle test more than the others do. Apple's guide [exposes the options](https://support.apple.com/guide/mac-help/take-a-screenshot-mh26782/mac) in the Screenshot app.

### Menu bar (unconfirmed)

| Setting | Key | Evidence | Status |
|---|---|---|---|
| Clock format | `com.apple.menuextra.clock DateFormat`, `ShowDate`, `ShowSeconds`, `Show24Hour` | B for `DateFormat`, C for `ShowSeconds`, H for `ShowDate`, G. `Show24Hour` not found in ControlCenter; the cache wasn't searched for it | Probably sound |
| Auto-hide menu bar | `NSGlobalDomain _HIHideMenuBar`, `AppleMenuBarVisibleInFullscreen` | C G | Mapping to Apple's [four options](https://support.apple.com/guide/mac-help/change-menu-bar-settings-mchlad96d366/mac) unverified |
| Item spacing | `NSGlobalDomain NSStatusItemSpacing` | C | Undocumented |
| Item visibility | `NSStatusItem Visible <id>` in `com.apple.controlcenter` | H | Unconfirmed, see below |
| Item display mode | `com.apple.controlcenter <Module>`, such as `Sound` | C for `BatteryShowPercentage`, N | Unconfirmed: sources disagree on host scope, see below |
| Hide Spotlight icon | `com.apple.Spotlight MenuItemHidden` | C H G (the setting only) | Scope and effect unconfirmed |

**Visibility keys are assembled at runtime, so they can't be confirmed from strings alone.** The literal `NSStatusItem Visible` isn't anywhere in the OS binaries, but AppKit has `-[NSStatusItem _visibleDefaultsKey]` and a second method, `_visibleCCDefaultsKey`. So the mechanism still exists and now has two variants. Which variant and which domain applies to each item couldn't be determined without toggling a setting.

**Sources disagree on the display-mode key's scope.** One open-source tool [reports testing on Sequoia](https://github.com/openbootdotdev/openboot/pull/70) that `defaults write com.apple.controlcenter Sound -int 18`, with no host flag, changes the dropdown, and that this key is separate from the visibility checkbox. nix-darwin [writes these keys to the `ByHost` file](https://github.com/nix-darwin/nix-darwin/blob/master/modules/system/defaults-write.nix) instead. The keys tested in `-currentHost` scope weren't set on the test Mac, and neither source covers macOS 27.

**Weather has no documented menu bar control.** Apple's [Menu Bar Controls list](https://support.apple.com/guide/mac-help/change-menu-bar-settings-mchlad96d366/mac) doesn't include it.

### Appearance, windows and others

| Setting | Key or tool | Permissions | Evidence |
|---|---|---|---|
| Light or dark | `NSGlobalDomain AppleInterfaceStyle`, `AppleInterfaceStyleSwitchesAutomatically` | None | C, H for the second |
| Scroll bars | `NSGlobalDomain AppleShowScrollBars` | None | C |
| Save to iCloud by default | `NSGlobalDomain NSDocumentSaveNewDocumentsToCloud` | None | C |
| Expanded save panel | `NSGlobalDomain NSNavPanelExpandedStateForSaveMode` | None | C |
| Stage Manager | `com.apple.WindowManager GloballyEnabled` | None | C |
| Click wallpaper to show desktop | `com.apple.WindowManager EnableStandardClickToShowDesktop` | None | C H |
| Edge tiling | `com.apple.WindowManager EnableTilingByEdgeDrag` | None | C |
| Displays have separate Spaces | `com.apple.spaces spans-displays` | None, needs logout per nix-darwin | C N; **see warning below** |
| Reduce motion, transparency | `com.apple.universalaccess reduceMotion`, `reduceTransparency` | **Writer probably needs Full Disk Access** | C for `reduceMotion`, N, [#705](https://github.com/nix-darwin/nix-darwin/issues/705), [#1049](https://github.com/nix-darwin/nix-darwin/issues/1049) |
| Safari, Mail and other containerized apps | `defaults -container` | **Privacy prompt, or Full Disk Access** | See permissions section; not probed |
| Power settings | `pmset -g` to read, `pmset` to write | **Root** to write, per `pmset(1)` | Man page; no restart stated |
| Computer and host names | `scutil --get`, `--set` | **Root** to write, per `scutil(8)` | Man page |
| Time zone, sleep | `systemsetup -get…`, `-set…` | **Admin**, per `systemsetup(8)` | Man page |
| Remote Login, remote Apple events | `systemsetup -setremotelogin`, `-setremoteappleevents` | **Admin and Full Disk Access**, per `systemsetup(8)` | Man page |
| Application Firewall | `/usr/libexec/ApplicationFirewall/socketfilterfw` | Probably root (unverified) | [macOS 15 notes](https://developer.apple.com/documentation/macos-release-notes/macos-15-release-notes); tool present |

**Warning on `spans-displays`.** The [macOS 26 notes](https://developer.apple.com/documentation/macos-release-notes/macos-26-release-notes) list a known issue: with "Displays have separate Spaces" turned off, WindowServer crashes at login, and the workaround involves booting into Recovery (153570422). The issue ID doesn't appear in the 26.1 to 26.5, 27 or 27.2 notes, so I don't know if it's still present. My recommendation is that an unattended Run shouldn't change this setting until someone confirms it's fixed.

## Permissions a notarized app would meet

- **Most settings need no permission.** No source I read lists a privacy permission for the user's ordinary preference domains, and dockutil calls the CFPreferences APIs directly. This is an inference; nothing was written.
- **The app probably can't use App Sandbox.** Apple [requires it for the Mac App Store](https://developer.apple.com/documentation/security/app-sandbox), but the [notarization guide](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution) doesn't list it as a requirement. A sandboxed app probably couldn't reach other apps' preference domains, though I didn't test that.
- **Notarization requires Hardened Runtime.** Apple says [you must enable it](https://developer.apple.com/documentation/security/hardened-runtime) to notarize a macOS app. I found nothing in its description that restricts preferences, but that isn't a test.
- **Containers trigger prompts.** From macOS 14, [accessing another app's container asks the person](https://developer.apple.com/documentation/security/accessing-files-from-the-macos-app-sandbox) for permission, including for apps that aren't sandboxed. If they refuse, the app can't read or write those files.
- **AppleScript is a poor fallback.** Sending Apple events needs the [Apple Events entitlement](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.automation.apple-events) and the person's consent. Since macOS 15, [controlling Finder fails at first](https://developer.apple.com/documentation/macos-release-notes/macos-15-release-notes) and only shows a notification pointing at System Settings, which suits a Needs-You Queue step rather than an unattended Run.
- **The app can't read permission state directly.** In macOS 27, [apps can no longer read the local TCC database](https://support.apple.com/en-us/148830), and `tccutil list` reports on managed devices only. On the test Mac, `tccutil --info` shows only `reset` usage. Detecting a missing permission probably means trying the write and handling the failure.

## What has stopped working

### In the repo's scripts

| Script | Line of behavior | Finding | Verified how |
|---|---|---|---|
| `finder.sh` | `sfltool add-item` for sidebar favorites | **Apparently removed.** `sfltool` usage lists only `csinfo`, `dumpbtm`, `archive`, `clear`, `resetbtm`, `resetlist`, `list` and `list-info`. `\|\| true` would hide a failure, and the script logs "favorites updated" either way | Ran `sfltool` with no arguments for its usage text; `add-item` itself wasn't run |
| `dock.sh` | `dockutil --add /System/Applications/Launchpad.app` | **Target is gone.** The app isn't there; `Apps.app` is. `\|\| true` would hide the failure here too | File check on 27.2; [Apple's guide](https://support.apple.com/guide/mac-help/open-apps-with-launchpad-mh35840/mac) now describes Apps; forum reports date it to macOS 26 |
| `dock.sh` | dockutil as a dependency | **Support claim is out of date.** Its [README](https://github.com/kcrawford/dockutil) claims Big Sur through Sonoma, and its latest release is 3.1.3 from February 2024 | Read the repo; behavior on 27 not tested |
| `menu-bar.sh` | `menuExtras` array | **Mostly dead.** Four of the five paths (`Bluetooth`, `Volume`, `Battery`, `Clock`) no longer exist in `Menu Extras`; only `AirPort.menu` does. It also replaces the whole array | Listed the folder on 27.2 |
| `menu-bar.sh` | `killall SystemUIServer` after writing `com.apple.controlcenter` keys | **Probably misdirected.** ControlCenter is its own agent, and the script never restarts it | Read the script and launchd plists; effect is an inference |
| `menu-bar.sh` | `NSStatusItem Visible` and Weather keys | **Unconfirmed.** See the menu bar section | Literal not found; key may be written by the script itself |
| `menu-bar.sh` | `log_ok` after `\|\| true` | **Reports success regardless** of the result | Read the script |
| `macos.sh` | Hot corners set to `0` | **Doubtful.** The documented disabled value in nix-darwin is `1` | Secondary source |
| `macos.sh` | `killall cfprefsd` | No documented need | See above |

Two comments don't match their code, which matters for porting. `finder.sh` says "Calculate all sizes" above `FXInfoPanesExpanded`, which expands Info window sections. `dock.sh` says "Mission Control / dragging to top" above `expose-group-apps`, which is Apple's "[Group windows by application](https://support.apple.com/guide/mac-help/change-desktop-dock-settings-mchlp1119/mac)" as far as I can tell.

The keys in `keyboard.sh` and the rest of `finder.sh` and `dock.sh` show no sign of having stopped working. That only means they exist on 27.2.

### Elsewhere in macOS

- **Application Firewall.** Since macOS 15, [settings are no longer in a plist](https://developer.apple.com/documentation/macos-release-notes/macos-15-release-notes). Use `socketfilterfw`.
- **Gatekeeper.** `spctl(8)` says that as of macOS 15, operations that change the global assessment state "will no longer be supported", even though the synopsis still lists `--global-disable`.
- **The 27 and 27.2 release notes don't mention any change to these keys.** A search for `defaults`, CFPreferences, `UserDefaults` and `com.apple.*` keys found nothing relevant. That's weak reassurance, because these keys are undocumented and may change without notice (my inference).

## Options for the spec

These are recommendations and open choices, not decisions.

**Suggested regardless of the choices below:**

- **Give each settings Item metadata:** domain, key, type, host scope, apply step, permission need, evidence group and the macOS versions it was last confirmed on.
- **Never report success without checking.** After a write, read the value back. Record it as "stored" rather than "working" unless a real test exists. One failed Item shouldn't stop the Run, but it shouldn't log success either.
- **Run a toggle-and-diff test on each macOS release, on a throwaway Machine or VM.** Change each setting in System Settings and diff `defaults export` before and after, then write the key and confirm the effect. It's the only way to close the gaps in this document, and it would settle the menu bar and screenshot groups.

**Choices for the human:**

1. **Unconfirmed settings:** leave menu bar visibility and screenshot keys out of the first Catalog until a toggle test passes, or include them labeled best-effort.
2. **Unset keys in Drift:** treat unset as no Drift, have the Catalog state an expected default for each key, or show a separate "not set" state.
3. **Dock contents:** keep dockutil as a dependency, or have the engine read and write `persistent-apps` through CFPreferences. dockutil's source shows the second is possible, but the engine would then own that format.
4. **Finder sidebar favorites:** make it a Needs-You Queue step, or drop it. I found no supported command-line replacement.
5. **Restarts and logout:** decide when a Run restarts Dock and Finder, given the visible disruption, and whether "log out to finish" joins the Needs-You Queue.
6. **Permission-gated settings:** decide whether the Catalog includes anything needing Full Disk Access, such as `com.apple.universalaccess`, or containerized app settings.
7. **`activateSettings -u`:** use this private binary to avoid a logout, or accept the logout.
8. **Risky settings:** decide whether `spans-displays`, and security settings such as `com.apple.LaunchServices LSQuarantine`, belong in the Catalog at all.

## Not verified

- **Any write taking effect**, or any apply step, on any macOS version.
- **Exact logout or restart needs** for keyboard, trackpad and screenshot settings.
- **The `com.apple.universalaccess` Full Disk Access requirement on 27.2.** It comes from user reports from 2023 and 2024 and wasn't probed.
- **Safari and Mail keys.** Deliberately not probed, to avoid triggering a privacy prompt on the test Mac.
- **Behavior on managed Machines.**
- **When `sfltool add-item` went away, or whether a hidden form still works.** It's missing from the usage text on 27.2. The version it left in wasn't established.
- **Whether the two macOS 26 known issues are still present.**
- **A stable Apple reference for removing Launchpad.** The missing file is verified on 27.2. The macOS 26 date comes from Apple Community threads, for example [this one](https://discussions.apple.com/thread/256138130).
- **`DoNotOfferNewDisksForBackup` for Time Machine.** The name wasn't in the OS locations searched, so it's left out of the tables.

## Sources

**Apple**

- Schemas: [Dock](https://github.com/apple/device-management/blob/release/mdm/profiles/com.apple.dock.yaml), [Finder](https://github.com/apple/device-management/blob/release/mdm/profiles/com.apple.finder.yaml), [Menu Extras](https://github.com/apple/device-management/blob/release/mdm/profiles/com.apple.mcxMenuExtras.yaml), [Accessibility](https://github.com/apple/device-management/blob/release/mdm/profiles/com.apple.universalaccess.yaml), [Privacy preferences](https://github.com/apple/device-management/blob/release/mdm/profiles/com.apple.TCC.configuration-profile-policy.yaml)
- Release notes: [macOS 27](https://developer.apple.com/documentation/macos-release-notes/macos-27-release-notes), [27.2](https://developer.apple.com/documentation/macos-release-notes/macos-27_2-release-notes), [26](https://developer.apple.com/documentation/macos-release-notes/macos-26-release-notes), [15](https://developer.apple.com/documentation/macos-release-notes/macos-15-release-notes)
- [What's new for enterprise in macOS 27](https://support.apple.com/en-us/148830)
- APIs: [`CFPreferencesAppSynchronize`](https://developer.apple.com/documentation/corefoundation/cfpreferencesappsynchronize(_:)), [`CFPreferencesAppValueIsForced`](https://developer.apple.com/documentation/corefoundation/cfpreferencesappvalueisforced(_:_:)), [`CFPreferencesCopyKeyList`](https://developer.apple.com/documentation/corefoundation/cfpreferencescopykeylist(_:_:_:)), [`UserDefaults`](https://developer.apple.com/documentation/foundation/userdefaults)
- Security: [App Sandbox](https://developer.apple.com/documentation/security/app-sandbox), [Hardened Runtime](https://developer.apple.com/documentation/security/hardened-runtime), [notarization](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution), [sandbox file access](https://developer.apple.com/documentation/security/accessing-files-from-the-macos-app-sandbox), [Apple Events entitlement](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.automation.apple-events)
- User guide: [Menu Bar](https://support.apple.com/guide/mac-help/change-menu-bar-settings-mchlad96d366/mac), [Desktop & Dock](https://support.apple.com/guide/mac-help/change-desktop-dock-settings-mchlp1119/mac), [Keyboard](https://support.apple.com/guide/mac-help/change-keyboard-settings-kbdm162/mac), [Trackpad](https://support.apple.com/guide/mac-help/change-trackpad-settings-mchlp1226/mac), [Screenshots](https://support.apple.com/guide/mac-help/take-a-screenshot-mh26782/mac), [Apps](https://support.apple.com/guide/mac-help/open-apps-with-launchpad-mh35840/mac)
- [TN2450: Remapping Keys](https://developer.apple.com/library/archive/technotes/tn2450/_index.html)
- Man pages read on macOS 27.2: `defaults(1)`, `cfprefsd(8)`, `pmset(1)`, `scutil(8)`, `systemsetup(8)`, `spctl(8)`, `chflags(1)`, `tccutil(1)`

**Local read-only checks on macOS 27.2 (26B8125)**

- `defaults read-type` and `defaults export` (key names only)
- Binary searches of Dock, Finder, ControlCenter, SystemUIServer, Spotlight, `screencapture`, `screencaptureui` and the dyld shared cache
- `/System/Library/LaunchAgents`, `/System/Applications`, `/System/Library/CoreServices/Menu Extras`

**Secondary sources (not Apple)**

- [nix-darwin defaults modules](https://github.com/nix-darwin/nix-darwin/tree/master/modules/system/defaults) and issues [#658](https://github.com/nix-darwin/nix-darwin/issues/658), [#705](https://github.com/nix-darwin/nix-darwin/issues/705), [#1049](https://github.com/nix-darwin/nix-darwin/issues/1049), [#1475](https://github.com/nix-darwin/nix-darwin/issues/1475)
- [dockutil source and README](https://github.com/kcrawford/dockutil)
- [openboot PR #70](https://github.com/openbootdotdev/openboot/pull/70)

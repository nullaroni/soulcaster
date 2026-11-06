# Can Rune carry this app?

Research for [ticket #3](https://github.com/nullaroni/macbook-setup/issues/3), read on 2026-11-06 at Rune `dev` rev [`ddf14140`][rev].

## Answer

**Probably yes. Nothing I verified rules Rune out, but no one has yet built this app's screens on it.** Rune has the controls, the background-work model, the accessibility layer and a shipped notarized app to point to. The main cost is the API itself: Rune is 16 days into its public history, has no releases, and changes daily.

Rune Kit has no ready-made catalog checklist, Run log or wizard, so the app composes those from existing parts. Packaging, notarization, updates, admin prompts and keep-awake handling sit outside Rune, so the app owns them.

I couldn't compile Rune because this Mac has no Rust toolchain and installs were out of bounds. Binary size for this app, real VoiceOver behavior and the macOS 13 floor are therefore unconfirmed (see [Not verified](#not-verified)).

## How to read this

- **Verified** means I read it in Rune's source or docs at the rev above, or inspected it directly, such as a shipped app bundle.
- **Inferred** means my own reasoning from that evidence. It's marked wherever it appears.

## Scorecard

| Question | Finding | Basis |
|---|---|---|
| Checkbox lists, progress, forms | Controls exist; the lists and forms are composed | Verified |
| Log views | No component, but the parts exist | Verified |
| Long-running background work | Well supported | Verified in docs and source; not run |
| Packaging as a `.app` | Rune doesn't do it; the app needs its own release pipeline | Verified |
| Notarization | Works on a shipped Rune app | Verified on Hex 3.3.0 |
| Minimum macOS | 13.0 implied, not published or tested in public CI | Verified evidence, inferred conclusion |
| Accessibility | Real AccessKit integration, not independently audited | Verified in code; not tried with VoiceOver |
| Binary size | About 18 MB stripped for a small example, not the 9.3 MB the README states | Verified from Rune's notes; not built |
| API stability | Compile breaks now rare in the controls this app needs; behavior changes daily | Measured from git; approximate |

## Rune Kit components

Exports are listed in [`rune-kit/src/lib.rs`][kit-lib] and described in the [kit README][kit-readme].

| App need | What Rune offers | What the app builds |
|---|---|---|
| Catalog checklist | [`Checkbox`][checkbox] with label, description and a mixed state for "some selected"; [`virtual_rows`][virtual] for long lists; [`Table`][table-cell] cells can hold any element; `Tree` rows take trailing elements; `Accordion` and `Collapsible` for groups | The Catalog list itself: grouping, select-all, search and filtering |
| Run progress | [`Progress`][progress] (determinate and indeterminate), `ProgressCircle`, `Spinner`, `Loader`, `Badge` | The per-Item status rows and the Run summary |
| Run log | [Follow-the-end scrolling][stick], virtual rows, [selection across virtual rows][select] (copy capped at 10,000 unbuilt rows and 4 MiB), monospace text, and `Span`s for color | **The log view:** the bounded line buffer, batching, filtering and any ANSI color parsing, since Rune has no ANSI parser |
| Forms | `TextInput`, `TextArea`, `Field`, `Select`, `Combobox`, `RadioGroup`, `Switch`, `.secure()` fields, `.invalid()`, and an `.error()` builder on one of the text fields | Form state and validation. I found no form or validation helper in the exports |
| Confirmations and alerts | `Dialog`, `Toast`, `Popover`, `Menu`, `Tooltip` | Nothing beyond the content |
| App layout | [`shell`][kit-readme]: `SplitView`, `Sidebar`, `Toolbar`, `Page` | Screen layout. There's no `Tabs` or wizard component, so use `SegmentedControl` or build a stepper |
| Reviewing Drift and Imports | [`rune_kit::diff`][kit-readme]: unified or split view, from `git diff` output or two texts | Mapping Config changes onto it, if a text diff is wanted at all |
| Look and feel | `Kit::neutral` and `Kit::from_palette`; the OpenCode theme is optional | A small theme |

Other parts this app could use:

- Native notifications through [`rune_mac::notify`][rune-mac], which could flag Needs-You Queue steps. They only work in a bundled app.
- Opening the browser with [`Tx::open_url`][open-url], and receiving `myapp://` links with [`App::on_open_url`][lifetime] when `Info.plist` declares the scheme.
- Headless [PNG rendering, scripted input and element-tree dumps][tooling], which let UI tests run without a window.

## Long-running work alongside the UI

Rune suits an engine that runs on its own threads. All of this is from the [app lifetime docs][lifetime]:

- `AppHandle<S>` is `Clone + Send + Sync`. `handle.update(|state, tx| ..)` can be called from any thread, runs in call order on the main thread and commits on the next frame. [`examples/background`][background] does this from spawned threads.
- `.keep_running()` keeps the process and state alive after the window closes. `on_close` runs first, so the app can warn that a Run is still in progress.
- Hidden, minimized or fully covered windows still apply updates but skip building and drawing. The next visible frame shows the latest state.
- `App::try_run` returns an error instead of panicking when there's no window server, such as over SSH to a Mac with nobody logged in. [`window_server_session`][session] makes that check. That gives the CLI fallback a clean way in.

Three limits matter here:

- **Handlers must not block.** [Debug builds warn at 8 ms][latest]. `rune::Latest` is built for "newest answer wins" requests such as search, so it doesn't fit a Run.
- **Every `handle.update` wakes the loop.** *Inferred:* the engine should batch log lines rather than send one update per line.
- **Rune doesn't keep the Mac awake.** A search of `rune` and `rune-mac` found no App Nap or sleep-assertion API, and only [a code comment][clock] mentions App Nap. A walk-away Run has to handle this itself, so it needs a test on a real Mac.

## Packaging and notarization

**Rune doesn't build release bundles.** The docs say plainly that ["Rune doesn't bundle apps"][no-bundle]. [`scripts/mac-app`][mac-app] installs into `~/Applications` for development:

- [It runs `codesign` without `--options runtime` or `--timestamp`][codesign], and [Apple's notarization requirements][apple] call for both. It can't produce a notarizable build as is.
- Its install protocol is worth copying for self-update: atomic swap with `renamex_np(RENAME_SWAP)`, a readiness receipt, and rollback if the new version fails to start.
- Rune has no updater.

**Notarization does work on a Rune app.** I inspected the Hex 3.3.0 macOS release without running it. Its [`Cargo.toml`][hex-cargo] pins `rune` and `rune-kit`, and the binary contains their source paths:

| Check | Result |
|---|---|
| Gatekeeper (`spctl`) | Accepted, `source=Notarized Developer ID` |
| Notarization ticket | Stapled and validated |
| Code signature | Hardened runtime, secure timestamp |
| Entitlements | Audio, calendars and Apple Events; no JIT or library-validation exception |
| Linked libraries | Only `/System/Library` and `/usr/lib` |

The last row matters for a bare Mac. *Inferred:* a Rune app shouldn't need Command Line Tools or Homebrew just to launch, because Hex links nothing else.

*Inferred:* this release probably drew with wgpu, not Rune's default native Metal backend, because [Hex moved its windows to native Metal on Nov 5][hex-metal], after the Nov 2 release. The spike below should notarize a build using the default backend.

[Deadbolt's signing script][deadbolt-sign] is a working reference for the pipeline: hardened runtime, `notarytool`, stapling and a DMG. Deadbolt has published no releases, so I could only read the script, not inspect its output.

**CLI fallback precedent.** Deadbolt [ships one binary with the app behind a default feature][deadbolt-one]. By default, Rune also [reads its tooling flags from the process arguments][args], and those flags [work on release builds][tooling]. *Inferred:* this app should parse its own CLI first and pass [`.args(Vec::new())`][lifetime], so a shipped build doesn't expose `--script` and `--snap`.

## Minimum macOS

Rune publishes no supported-OS list, and [its own open issue #111][issue-111] asks for one. The evidence points to **13.0**:

- `scripts/mac-app` writes [`LSMinimumSystemVersion` 13.0][min-plist].
- The default Metal backend [needs MSL 3.0, "so needs macOS 13"][metallib]. In that backend, [`msl_version`][msl] picks MSL 3.0 for anything below macOS 14, with no lower path. *Inferred:* opting out to wgpu might run lower, since [its precompiled shader targets macOS 11][metallib-11], but that gives up the default backend.
- Newer features are gated at run time, such as Liquid Glass on macOS 26.

The floor is less proven than it looks:

- Rune's [only GitHub workflows][workflows] deploy websites, so no public CI runs its tests on any macOS version. Native GUI tests [only run when someone sets `RUNE_GUI_TESTS=1`][a11y-test] in a logged-in session.
- Rune's source refers to measurements on macOS 15.3, 26 and 27. A search of its code, docs and changelog found none on 13 or 14.
- Hex sets its own minimum to **15.0**, the only shipped floor I checked.

*Inferred:* treat 13 as theoretical until the app has run there. Every Apple Silicon Mac can update to macOS 13 or later, so a floor mainly affects Macs that haven't been updated. The install command should check the version and explain the upgrade, because the app won't open below its floor.

## Accessibility

Rune's accessibility is real and built in, through the default [`accessibility` feature][rune-cargo]:

- [AccessKit maps every window to NSAccessibility][a11y] for VoiceOver and Voice Control. Nothing is built until an assistive client asks.
- [Kit components report name, value, states and actions][kit-a11y]. Checkboxes report checked or mixed, progress reports its percentage, errors are announced, and toasts are polite live regions.
- Assistive actions run the same handlers as pointer and keyboard input.
- Keyboard focus works, including [arrow-key movement through a `focus_group`][focus-group]. Reduced Motion is respected.
- Test coverage includes [per-component golden files][a11y-golden] and [a macOS test that walks the tree through AppKit and presses a button][a11y-test].

The gaps:

- **It hasn't been audited.** [Issue #111][issue-111] says "We have not audited these" against WCAG 2.1 A and AA, and is open. Its wish list includes following the system text size, so don't assume that works.
- **Virtual lists expose mounted rows only**, plus the total count and each row's index. A long Catalog list needs a VoiceOver check.
- **Custom composed views are on the app.** Anything built from raw elements, such as the log view, must set its own name and live-region behavior.

Two Rune docs are out of date and shouldn't be relied on: the README's [claim of "No AccessKit"][stale-a11y] and the kit's [claim that indeterminate progress is static][stale-progress]. The [source shows it animates][progress-loop].

## Binary size

| Measurement | Size | Source |
|---|---|---|
| `settings` example, stripped | **17.9 MB** (11.2 MB of it code) | [Rune GPU notes][size-new]; file last changed 2026-11-05 |
| "Binary", stripped, in the README | 9.3 MB | [README][size-old], dating from Rune's first two days |
| Rune's goal for a stripped hello app | 5 MiB or less | [PLAN.md][size-goal] |
| Hex 3.3.0 executable | 59.9 MB; zip 27.1 MB | Measured; includes much besides Rune |

The README figure looks stale, so use the newer one. *Inferred:* expect an 18–25 MB executable and a download around 10 MB, which is fine for this app. Turning off default features can drop unused parts such as Ogg audio and SVG, but [the app must then re-add `metal-backend`][launch] and `accessibility`.

## API breakage

**The compile surface for this app's controls has been quiet, but the sample is short.** I compared public item declarations across revs of `crates/rune/src` and `crates/rune-kit/src`:

| Window | Items at end | Signatures gone | Names gone | Names new |
|---|---:|---:|---:|---:|
| Oct 23 → Oct 26 | 2,149 | 12 | 1 | 732 |
| Oct 26 → Oct 29 | 2,470 | 34 | 20 | 261 |
| Oct 29 → Nov 1 | 2,558 | 25 | 12 | 75 |
| Nov 1 → Nov 3 | 2,656 | 3 | 0 | 77 |
| Nov 3 → Nov 6 | 2,789 | 9 | 0 | 90 |

The same comparison on the kit files this app would use (checkbox, list, progress, field, text input, dialog, select, radio, toast, switch, button, table) found **no signatures removed in two weeks**, while items grew from 128 to 197. In `app.rs`, `dev_panel` was removed, and `canvas_id` and `full_viewport` left that file (I didn't check whether they moved elsewhere).

This method is rough. It reads source text, so it counts internal `pub` items, can treat moved or reformatted items as gone, and misses changes in behavior.

**Behavior changes far more often:**

- [The changelog][changelog] exists for changes that can move an app's output. There are [175 fragments still unfolded][fragments] from the 9 days to Nov 6, so the true count is at least that.
- Four entries are labeled Breaking. Two change behavior or appearance for existing controls: [Return no longer presses a focused button][break-return], and [default `Button` height changed][break-button].
- [Rune #44][issue-44] shows the risk: new blend defaults nearly hid a selection in Deadbolt. It was fixed within a day.

**What other apps' upgrades looked like:**

- The 12 latest Hex `Cargo.toml` commits that mention Rune (Nov 2–5) touched no Rust files in 6 cases and 1–8 files in the rest.
- Deadbolt's early upgrades (Oct 26–30) touched 4–10 Rust files, though they also adopted new features. Its [Oct 31 upgrade][deadbolt-bump] touched none.
- Both apps come from Rune's own GitHub organization, so a third party may have a harder time.

**Safeguards and limits:**

- [`dev` is fast-forward-only][ff], so a pinned rev keeps resolving. Pinning fully protects a build until the app chooses to upgrade.
- [Rune's contributor rules][literals] tell its developers to add builder methods rather than fields to types that apps build as literals, including `AppSpec`.
- Some replaced APIs get `#[deprecated]` first, such as `ButtonSize`, though Rune makes no stated promise about this.
- Commits slowed from 125–252 a day on Oct 22–27 to 18–46 a day on Nov 2–6. *Inferred:* the pace may be settling, but 16 days is too short to say.

## Vendor risk

- [Rune is pre-release][pre-release] at version 0.0.1 with no tags or releases. The MIT license allows a fork.
- [It takes no pull requests][no-prs]. Fixes depend on the vendor's agents picking up issues.
- Of 52 issues, 41 are open, and many of those were filed today. Of the 11 closed, 5 closed the same day, 3 the next day and 3 within 3–7 days.
- *Inferred:* a fork is the realistic fallback for a Rune-side bug the vendor won't fix, and a large one for a framework this size. The map's separate engine and CLI limit the damage, because a Run would still work without the UI.

## What the app must build itself

**UI, composed from Rune parts:**

- Catalog checklist
- Run progress view
- Run log view
- Form state and validation
- Wizard or stepper for first run
- Theme

**Outside Rune:**

- Release bundle, signing, notarization and DMG
- Updater
- Keep-awake handling during a Run
- Admin authorization for privileged steps; Rune has no helper for this
- Running child processes and capturing their output
- GitHub sign-in flow, using Rune's URL opening and link handling

## What would rule Rune out

I found no blocker today. These would change the answer:

1. **A hard requirement below macOS 13.**
2. **A hard need to claim accessibility conformance soon**, since Rune is unaudited.
3. **A need for a stable API or a support commitment**, which Rune doesn't offer.
4. **A failed spike** on any item below, especially VoiceOver on the Catalog list, or macOS 13 launch if that floor is required.

## Recommendation

Keep Rune, and run a short spike before the build handoff. These are my suggestions; the decisions are yours.

**Spike, on a pinned rev:**

1. Build a small app with Rune Kit and record its real size.
2. Sign, notarize and staple it, then launch it on a fresh Mac with no Command Line Tools.
3. Launch it on the lowest macOS version you intend to support.
4. Try a Catalog list of about 500 Items with VoiceOver and keyboard only.
5. Stream a noisy simulated Run into a log view, with the window hidden and with the Mac left idle, to check App Nap and sleep.

**Decisions for you:**

- **Minimum macOS:** 13 as Rune implies, or a later version you can actually test, such as 15, which Hex uses?
- **Accessibility bar:** works well with VoiceOver, or a formal conformance claim?
- **Upgrade policy:** how often to bump the pin, and what check must pass first? Rune's headless rendering could support visual regression checks.
- **Vendor risk:** is a pre-release, no-PR dependency acceptable for a public app, given the CLI fallback?

## Not verified

- **Nothing was compiled or run from Rune.** This Mac has no Rust toolchain, and installing one was out of scope. Claims about behavior come from docs and source, not observation.
- **This app's binary size.** The estimate is inferred from other apps.
- **VoiceOver in practice.** I read the accessibility code and tests but didn't use a screen reader.
- **macOS 13 and 14.** No test on either version, by me or in any public CI I could find.
- **Notarization with the default backend.** Hex proves notarization works for its configuration, which probably wasn't native Metal. It doesn't cover every feature set.
- **App Nap's effect on a Run.** I confirmed that Rune has no API for it, not what actually happens.
- **System text size.** I didn't confirm whether Rune follows it.
- **API churn.** The figures are a text comparison, not a compile test across revs.
- **Build time.** Not measured. Rune's Git history alone is 162 MB.

## Sources

Rune links are pinned to rev [`ddf14140`][rev]. External repositories and issues were read on 2026-11-06, and Hex was inspected at release [v3.3.0][hex-release].

[rev]: https://github.com/anomalyco/rune/tree/ddf14140253bd1b963993895e3fa025d1ec58537
[kit-lib]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune-kit/src/lib.rs
[kit-readme]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune-kit/README.md
[kit-a11y]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune-kit/README.md#L126-L149
[checkbox]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune-kit/src/checkbox.rs#L43
[virtual]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune/src/el/virtual_rows.rs#L76
[table-cell]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune-kit/src/table.rs#L155
[progress]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune-kit/src/feedback.rs#L111
[progress-loop]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune-kit/src/feedback.rs#L160
[stick]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune/src/el.rs#L146-L152
[select]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/CHANGELOG.md#L592
[focus-group]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune/src/el.rs#L1002
[rune-mac]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune-mac/README.md
[open-url]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune/src/open_url.rs
[lifetime]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune/README.md#L490-L611
[latest]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune/README.md#L733-L769
[launch]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune/README.md#L613-L632
[tooling]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune/README.md#L2278-L2300
[a11y]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune/README.md#L2207-L2247
[no-bundle]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune/README.md#L590
[stale-a11y]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune/README.md#L2767
[stale-progress]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune-kit/README.md#L437-L439
[size-old]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune/README.md#L2752
[size-new]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/research/34-own-gpu-plan.md#L387-L390
[size-goal]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/PLAN.md#L519
[background]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/examples/background/src/main.rs#L200
[session]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune/src/window/event_loop.rs#L32
[clock]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune/src/frame/clock.rs#L21
[args]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune/src/tooling/args.rs#L90
[mac-app]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/scripts/mac-app.md
[codesign]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/scripts/mac_app_install.py#L139
[min-plist]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/scripts/mac_app_install.py#L130
[metallib]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune/src/render/metallib.rs#L43
[msl]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune/src/gpu/metal/device.rs#L70-L84
[workflows]: https://github.com/anomalyco/rune/tree/ddf14140253bd1b963993895e3fa025d1ec58537/.github/workflows
[rune-cargo]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune/Cargo.toml
[a11y-golden]: https://github.com/anomalyco/rune/tree/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune-kit/tests/a11y
[a11y-test]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune/tests/accessibility_macos.rs
[changelog]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/CHANGELOG.md#L3-L4
[fragments]: https://github.com/anomalyco/rune/tree/ddf14140253bd1b963993895e3fa025d1ec58537/changelog/unreleased
[break-return]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/CHANGELOG.md#L2199
[break-button]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/CHANGELOG.md#L2478
[ff]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/AGENTS.md#L21
[literals]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/AGENTS.md#L29
[pre-release]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/README.md#L55
[no-prs]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/README.md#L61
[issue-44]: https://github.com/anomalyco/rune/issues/44
[issue-111]: https://github.com/anomalyco/rune/issues/111
[hex-release]: https://github.com/anomalyco/hex/releases/tag/v3.3.0
[deadbolt-sign]: https://github.com/anomalyco/deadbolt/blob/HEAD/.github/release/sign-macos.sh
[deadbolt-one]: https://github.com/anomalyco/deadbolt/pull/200
[deadbolt-bump]: https://github.com/anomalyco/deadbolt/pull/186
[hex-cargo]: https://github.com/anomalyco/hex/blob/v3.3.0/Cargo.toml
[hex-metal]: https://github.com/anomalyco/hex/pull/1347
[metallib-11]: https://github.com/anomalyco/rune/blob/ddf14140253bd1b963993895e3fa025d1ec58537/crates/rune/src/render/metallib.rs#L40-L41
[apple]: https://developer.apple.com/documentation/security/resolving-common-notarization-issues

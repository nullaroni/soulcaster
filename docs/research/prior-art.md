# What winutil, Ninite, nix-darwin, chezmoi and Homebrew Bundle teach us

Research for [ticket #8](https://github.com/nullaroni/macbook-setup/issues/8), checked on 2026-11-06 against each project's own docs and source, its issue tracker where it has a public one, and live API responses.

**No project code was run.** Source was cloned to a temp folder and read. Source links are pinned to the commits read: winutil `2ef5e5d`, nix-darwin `ff0e21a`, chezmoi `745ad56`, Homebrew `88f401f`. Counts of winutil's JSON came from parsing those files.

Facts below are verified unless marked **Inferred**. The recommendations are inferences from the evidence, and the choices they raise are listed for the owner in [Decisions for the owner](#decisions-for-the-owner).

## Short answer

Four lessons matter most for the Config format:

1. **A Config should be selections and values, with no behaviour in it.** The other tools' file formats can all run code: winutil's catalog holds PowerShell, a Brewfile is Ruby, nix-darwin is a programming language, and chezmoi runs scripts. Keeping a Config as plain data is what makes a full Import preview possible, because everything it can do can be listed in advance.
2. **Drift needs a third record: what the app last applied.** Comparing only Config and Machine can't show which side changed. chezmoi keeps this record and can tell the two apart. winutil doesn't, and a comment in its service code says so. This record belongs on the Machine, outside the Config.
3. **"Not in the Config" must never mean "remove it".** nix-darwin's cleanup option deleted Mac App Store apps Homebrew had never managed ([#1860][nd-1860]). Removal shouldn't be expressible in a Config at all. It should be a confirmation made on the Machine.
4. **Item IDs are a public API from the first release.** winutil's exported Configs fail completely when one key is renamed, and its share-link format now can't change. IDs need to be stable, not tied to the UI, and backed by rename and retirement records in the Catalog.

Homebrew 6 also changes the Import design. Since [6.0.0][bt-trust], third-party taps need explicit trust, but installing a fully qualified name [grants that trust by itself][bt-trust-36]. So the app has to enforce its own approval step, because Homebrew won't stop it. See [Sharing and trust](#sharing-and-trust).

## How each tool works

| | File format | Can the file run code? | Catalog | Finds differences? | Removes things? |
|---|---|---|---|---|---|
| **winutil** | Flat JSON list of catalog keys | The Config can't. The catalog can, and it ships inside the app | JSON compiled into each release | Partly: ticks boxes for what it detects | Separate manual actions |
| **Ninite** | No file: choices live in a database row or a URL | No, but it's a closed service | Private, run by the vendor | Skips or upgrades at run time. Reports are Pro only | Pro `/uninstall` only |
| **nix-darwin** | Nix language | Yes | Typed options in the source | No, except the `/etc` hash check and Homebrew `check` mode | Off by default. Opt-in, with no prompt |
| **chezmoi** | Files plus name attributes, templates and scripts | Yes | None | Yes, in both directions | Only with explicit attributes such as `remove_` and `exact_` |
| **Homebrew Bundle** | Brewfile (Ruby) | Yes | Homebrew itself | `check`, `dump` | `cleanup`, which prompts unless forced |

## winutil

### Config and catalog files

- The catalog is eight JSON files in [`config/`][w-config] that are [compiled into a single script][w-spec]. [`applications.json`][w-apps] has 239 entries, each with a display name, description, category, link, FOSS flag and package IDs. None has versions. All 239 have a winget ID, and 19 have no Chocolatey ID.
- An exported Config is [a flat list of catalog keys][w-auto-keep], such as `WPFInstall7zip` and `WPFTweaksTelemetry`. There's no version field and no place for a value. [The docs][w-auto-25] say toggles, drop-downs and the package-manager choice can't be applied from a preset.
- With one selection, the export is [a single string rather than a list][w-auto-keep]. That's a side effect of how the JSON gets written.
- The older format stored package IDs in the Config too. That was dropped: ["Current package IDs come from the app catalog"][w-impex-92]. The current code detects the old format [by looking at which property names exist][w-impex-87], because there's no version number to read.
- [`tweaks.json`][w-tweaks] has 69 entries and 121 registry values, each with a target value and usually an `OriginalValue` for undo. 63 of those 121 undo by deleting the value (`<RemoveEntry>`), so "not set" is a real state.
- The JSON isn't fully declarative. 27 of 69 tweaks and 18 of 33 features embed PowerShell in `InvokeScript`, and the longest tweak script runs to 51 lines. Those multi-line strings contain raw newlines, so Python's strict JSON parser rejects `tweaks.json`. The project's contract only requires that it [pass `ConvertFrom-Json`][w-spec-config].

### Drift

- Detection only ticks boxes in the UI. [`Invoke-WPFGetInstalled`][w-getinstalled] selects what it finds. I found no report comparing a Config with the Machine.
- Installed apps are found by [parsing `winget list` table output][w-current-46]. The code has to work around column padding and emoji in names, and users still report misses ([#4919][w-4919]).
- Tweaks are detected by [comparing registry and service values][w-current-90]. The 13 tweaks that are script-only are never detected, because they give the code nothing to read.
- Undo restores the value [written in the catalog][w-tweak-90], not the value the user had. The service path has a guard, and its comment states the limit: ["This compares current state, not history"][w-tweak-56]. Registry undo has no such guard.

### Sharing and trust

- Anyone can import [a local file or any URL][w-impex]. The docs [ask for HTTPS and warn about plain HTTP][w-auto-40]. The file can only select existing catalog keys, so importing runs nothing new.
- Modern imports are all-or-nothing: [one unknown key rejects the whole file][w-auto-keep]. Old-format imports skip unknown keys and report them.
- Because a release can remove keys, the Config Builder [fetches the catalog from the latest stable tag][w-spec-107] and generates a command pinned to the release it used.
- Share links carry keys in the URL hash, and the project's contract [says it can't change that format because the links are posted publicly][w-spec-110].
- The app itself runs through `irm https://christitus.com/win | iex` [as administrator][w-readme]. The release workflow declares [a certificate secret][w-release], but no step in that file signs anything.

### How the catalog stays current

Package versions are winget's and Chocolatey's concern. People propose apps through [a discussion template][w-suggest] with a checklist (popular, free, not bloatware, on winget). I found nothing in CI that checks whether package IDs still resolve, and Dependabot covers only GitHub Actions. Releases are [cut by hand and tagged with the date][w-release].

### What users complain about

Bodies read unless marked *title only*.

| Theme | Reports |
|---|---|
| Reports success when nothing happened | [#2711][w-2711] "tweaks are finished" with nothing applied; [#4821][w-4821] GUI reports success, package still there |
| Tweaks break the OS, or stop working after updates | [#1098][w-1098], [#5130][w-5130], [#5095][w-5095]; *title only:* [#5190][w-5190], [#3645][w-3645] |
| Defaults people disagree with | [#3061][w-3061] disabling IPv6 was preselected (joint top-reacted issue in my search) |
| Overrides the user's own choices | [#1726][w-1726] resets services the user had disabled |
| Detection misses | [#4919][w-4919] Firefox not detected under either package manager |
| Unattended runs that stop for input | [#5021][w-5021] Battle.net waits for a directory at the console |
| Installs dependencies without asking | [#1118][w-1118] Chocolatey installed unprompted |
| Noise from harmless errors | [#5142][w-5142] errors for things "already removed" |
| Antivirus blocks the script | *title only:* [#1742][w-1742], [#2484][w-2484], [#2433][w-2433] |
| Valid reports auto-closed | [#2641][w-2641] asks to remove the stale-issue bot |

**Inferred:** the headless path's [per-step timeout][w-autorun] exists because of hangs like #5021. It's a cheap safeguard for any unattended Run.

## Ninite

Ninite is closed source with no public issue tracker, so this rests on its own help pages.

- **No Config file exists.** [A selection is a database row][n-how] with a random ID stamped into the installer, and the installer fetches current configuration from the server on each run. Sharing is [a permalink like `ninite.com/firefox-flash-skype`][n-share], or [`?select=firefox-skype`][n-select], which lets the recipient change the selection first.
- **It compares at run time.** On each run it [checks what's installed, skips what's current and upgrades what isn't][n-how], so rerunning the same installer is the update mechanism.
- **Failed verification stops that app.** Downloads come from publishers and must match a hash or signature, and there are ["no options or prompts to ignore these issues"][n-sec]. Its docs disagree on the hash: the security page says SHA-256, and the how-it-works page says [SHA-1][n-how].
- **The catalog is run by the vendor.** The homepage says ["You don't have to watch for updates. Our bots do that"][n-home] and lists updates from minutes earlier. New versions are [verified at the origin and virus-scanned][n-sec], and updates are configured in clean snapshotted VMs.
- **Per-app results are named.** The [error page][n-errors] lists failed download, failed installation, skipped because .NET is required, skipped because the program is running or locked, and a miscellaneous skip.
- **It says no on purpose.** [Not Features][n-not] rules out custom install locations, more apps, and drivers, among others. [Install location][n-loc] is called "an implementation detail", and the catalog aims at ["the common 90%"][n-more]. Drivers are out partly because [many need a reboot][n-drivers].
- **Drift reporting is paid.** [`/audit`][n-audit] reports each app as not installed, updatable or OK, and it's a Pro feature, as are [`/select`, `/uninstall` and `/freeze`][n-switch].

**Couldn't verify:** real user complaints. The forum threads a web search surfaced would not load, so none are cited. The Not Features page is the only first-party evidence of what people ask for.

**Inferred:** the free installer can work this way, with no record of past runs, because it only ever installs and upgrades. It never decides whether a difference should flow back, which this project's Adopt does, so its model isn't enough here. Its [hidden-window installer automation][n-how] is per-app work that Homebrew casks avoid.

## nix-darwin

### Config and schema

- Settings are [typed options][nd-dock], such as `system.defaults.dock.autohide` with the type "null or bool". Types produce errors before anything is applied.
- **Null means "don't touch".** Settings default to `null`, and [null values are filtered out][nd-write-11] before any `defaults write`.
- **Removing a setting doesn't revert it.** I found no module that issues `defaults delete`. The [dark mode option's docs][nd-global] tell users to set `null` and run the delete by hand.
- Options carry how they take effect in their docs, such as ["requires logging out"][nd-global]. Dock changes [restart the Dock][nd-write-156].
- User and system scope are separate. User-level settings [need a named primary user][nd-primary], since activation runs as root.
- **Renames and removals are first-class.** [`mkRenamedOptionModule`][nd-dock] forwards an old name, and [`mkRemovedOptionModule`][nd-alf] fails with an explanation of what to use instead.
- [`system.stateVersion`][nd-state] freezes older defaults for existing users, so a new default doesn't disturb a working system. The [CHANGELOG][nd-changelog] records behaviour changes against it.

### Drift

- There's no general detection. Each activation rewrites what the Config declares.
- `/etc` is the exception: a file is overwritten only if its hash is [known][nd-etc]. Otherwise activation aborts.
- `homebrew.onActivation.cleanup` offers [`none`, `check`, `uninstall` or `zap`][nd-hb-78], with `none` as the default. In `check` mode, unlisted packages [abort the whole activation][nd-hb-93].
- [`darwin-rebuild`][nd-rebuild] has `check`, `build`, `--list-generations` and `--rollback`. **Inferred:** rollback can't restore Homebrew packages, since they live outside Nix. Homebrew itself says [reverting a Brewfile doesn't restore versions][bt-teams-125].

### What users complain about

| Theme | Reports |
|---|---|
| Removal goes further than documented | [#1860][nd-1860] `cleanup` removes every unlisted Mac App Store app, with no prompt, while the docs said the opposite |
| Breaks when Homebrew changes its CLI | [#1369][nd-1369] `--no-lock` removed; [#1787][nd-1787] `--cleanup` started requiring `--force` |
| The existing-file check blocks people | [#1509][nd-1509] macOS 26 changed `/etc/zshrc`, and all activation stopped until files were moved by hand; [#149][nd-149] asks for a way to take over existing files |
| Settings written but not in effect | [#658][nd-658] trackpad setting needs `activateSettings -u`; [#1207][nd-1207] |
| Multi-user Macs | [#554][nd-554] |
| Mac App Store on a fresh Mac | [#1694][nd-1694] installs fail under activation but work when run by the user |
| Upstream apps change shape | [#1041][nd-1041] Karabiner v15 broke the service module |
| Slow, repeated reconciling | [#1829][nd-1829] `brew bundle` runs every time. `brew bundle check` takes over a second, and listing commands take hundredths |

## chezmoi

### Files and state

- chezmoi separates [four things][c-concepts]: the source state (what you declared), the target state (computed for this Machine), the destination state (what's actually there), and machine-specific data in a local config file.
- It also keeps a persistent state database recording each entry it last wrote. That's what lets [`apply` ask][c-apply] about a file that "has changed since chezmoi last wrote it". It [stores a SHA-256 of the contents rather than the contents][c-arch], which keeps secrets out of that database.
- [`chezmoi status`][c-status] has two columns: what changed since the last write, and what `apply` will do. [`merge`][c-merge] is a three-way merge, and if the target can't be computed it falls back to two-way.
- [`diff`][c-diff] previews changes, [`verify`][c-verify] answers with an exit code, [`re-add`][c-readd] pulls Machine changes back into the source, and [`unmanaged`][c-unmanaged] lists what isn't tracked. Between them, these cover Apply, Adopt and Untracked.
- Machine differences live in [a local config file][c-machines], filled in once by [`promptStringOnce`][c-prompt]. Prompts are [limited to that step][c-design] because templates are evaluated on every command.
- Attributes are [encoded in filenames][c-attrs], which is powerful but cryptic. [`exact_`][c-targets] is opt-in: it removes anything not listed, and only in that directory.
- [`.chezmoiversion`][c-version] declares a minimum app version, and chezmoi refuses to read anything it's too old for.
- [`.chezmoiexternal`][c-external] entries can carry checksums and refresh periods.

### Sharing and trust

- chezmoi is built for your own repo: [`chezmoi init --apply $USER`][c-quick] applies straight from GitHub. Its `run_` scripts [can run arbitrary commands][c-design-326], and I found no guidance in its docs on trusting someone else's repo. **Inferred:** applying a stranger's repo runs their code, and this project's Import has to differ here.
- The docs themselves say [scripts "break chezmoi's declarative approach and should be used sparingly"][c-scripts].
- [One source of truth is deliberate][c-multi]: multiple sources "do not compose easily if target files overlap". That supports this project's decision that a Machine follows one Config.

### What users complain about

| Theme | Reports |
|---|---|
| Can't choose "keep the Machine's version" at the prompt | [#2162][c-2162] open request. The prompt offers overwrite and skip but not accept |
| Applying over local changes without warning | [#423][c-423] asked for a warning by default before applying |
| Wanted a quick overview before `diff` | [#424][c-424] asked for `status`, since added |
| Removals don't sync | [#4361][c-4361] says deletions can't be detected automatically |
| Diff exposes secrets and needs an unlock | [#5128][c-5128] |
| Apps that replace their own config files | [#1348][c-1348] symlinks destroyed by the app |

## Homebrew Bundle

- **A Brewfile is code.** [It's evaluated as Ruby][bb-303], and Homebrew says to [review changes as code][bt-teams-99]. Entries can carry `postinstall:` shell commands and `system` calls. Importing someone's Brewfile would break ADR 0001's no-shell rule.
- **It offers presence, not versions.** Bundle [doesn't pin or lock][bb-285], and the same revision [can install different versions on different days][bt-teams-125]. Lockfile support was [removed in Homebrew 4.4.0][nd-changelog].
- **Outdated isn't missing.** [`brew bundle check --no-upgrade`][bm-865] skips the outdated check, so the two questions can be asked separately.
- **`dump` writes current state** and [stops without writing if discovery fails][bb-dump], even with `--force`.
- **`cleanup` is sharper than it looks.** It [prompts unless forced][bm-cleanup]. It also [resets Homebrew's trust store][bb-173] to what the Brewfile declares, deleting trust granted any other way.
- **Layering doesn't remove anything.** With several Brewfiles, [removing an entry from one doesn't uninstall it or show nobody needs it][bt-teams-132].
- **The lifecycle is explicit.** [Deprecated warns, disabled errors, and removal follows][bd-life], with [dates, reasons and replacements][bd-cask]. Disabled packages are [auto-removed after a year][bd-life]. Renames go in [`cask_renames.json` and `formula_renames.json`][bd-rename].
- **The metadata is public JSON.** The [JSON API][bd-api] needs no Homebrew install. A live [cask response][bd-firefox] includes `token`, `old_tokens`, `desc`, `homepage`, `auto_updates`, `deprecated`, `disabled`, `deprecation_replacement_cask` and `analytics`. The full [`cask.json`][bd-allcasks] is about 19 MB and `formula.json` about 32 MB.
- **Complaints:** I sampled this tracker lightly. The issues that surfaced were about `dump` not recording things faithfully, mostly since tap trust arrived: [#22622][bb-22622], *title only:* [#22756][bb-22756], [#22753][bb-22753].

## Other Mac tools

- **[Mackup][m-readme]:** its README warns link mode ["will BREAK YOUR PREFERENCES"][m-readme] on macOS 14 and later, since macOS doesn't support symlinked preferences. Users [lost settings][m-2035], and copy mode [was added as the fix][m-1924]. **Inferred:** links in a Config's dotfiles section are safe for dotfiles, but macOS preferences should go through `defaults` or be left out.
- **[Installomator][i-does]:** each label names a download URL and an [`expectedTeamID`][i-label]. A download must be signed and notarized and match the Team ID, or it fails. The cost is maintenance: [a vendor changing certificates breaks a label][i-3162]. Users have also asked for [a minimum release age][i-2974] to reduce supply-chain risk (open).
- **[AutoPkg][a-trust]:** the closest match for sharing configuration with strangers. Local overrides store trust info about a recipe. If the recipe changes, it fails until someone reviews the change and [re-approves it][a-trust]. [`autopkg audit`][a-trust] flags missing signature checks, plain HTTP, and hard-coded credentials, and it can gate CI.
- **[Strap][s-readme]:** idempotent, and strict about scope. It lists what it won't do, including prompting for information during install. It reads a user's Brewfile and dotfiles repos by convention.
- **[mac-dev-playbook][p-readme]:** a [YAML defaults file][p-config] overridden by a user's `config.yml`, including Homebrew lists and a dotfiles repo with named files. It's close to this project's Config in shape, but it needs Ansible and it can run arbitrary tasks.

## Sharing and trust

Three Homebrew facts shape Import directly:

1. **A third-party tap is code.** [Loading one can run Ruby from that tap][bt-trust] with the user's privileges. So a Config without shell commands can still cause code to run through a third-party tap, and the preview should say so plainly.
2. **Homebrew won't enforce the approval.** [Installing a fully qualified name trusts that item][bt-trust-36] with no prompt, and Brewfile entries [can grant trust with `trusted: true`][bb-trusted]. Only the app's own check stands between a Config and that trust.
3. **Trust doesn't follow renames.** If an alias or former name resolves to a new target, [the new formula needs its own trust][bt-trust-47]. A rename in a tap should re-trigger approval.

**Inferred:** the person who writes a Config must never be able to approve anything on behalf of the person importing it. So a Config should be able to contain a third-party tap but not a trust grant. `trusted: true` passed straight through from a Config would hand the author that power. Approvals belong on the Machine, recorded the way AutoPkg records them: tied to what was reviewed, and voided when it changes.

## Keeping catalogs current

| Tool | What the catalog holds | How it stays current | What it costs |
|---|---|---|---|
| winutil | IDs into winget and Chocolatey | People, plus releases | Stale IDs surface as user bugs. Removed keys break old Configs |
| Ninite | Everything, per version | Vendor's bots, checked on each run | Full-time work, and depends on a live server |
| nix-darwin | Typed settings in source | Project releases, with a changelog | Breaks when Homebrew changes its CLI |
| Installomator | URL and Team ID per app | Community pull requests | Labels break when vendors change |
| Homebrew | Full package metadata and code | Automated bumps, plus a lifecycle | None for this project if reused |

**Inferred:** this project's Catalog is mostly a mapping from stable IDs to Homebrew tokens, plus settings definitions, and it holds no versions. That puts it nearer winutil than Ninite, with much lower churn. How it's delivered belongs to the Catalog delivery ticket, but winutil's main gap is cheap to close: a scheduled CI job could check every Catalog token against Homebrew's JSON, and open a pull request for renames, deprecations and disabled packages.

## Recommendations for the Config format

Each recommendation is an inference, with its evidence linked above.

### Shape

```json
{
  "$schema": "https://<catalog-host>/schema/config-1.json",
  "schemaVersion": 1,
  "name": "Example laptop",
  "items": ["agent.claude-code", "app.firefox", "tool.git"],
  "packages": {
    "formulae": ["acme/tools/widget", "ripgrep"],
    "casks": ["rectangle"],
    "taps": [{ "name": "acme/private", "url": "https://git.example.com/acme/homebrew-private.git" }]
  },
  "settings": {
    "dock.autohide": true,
    "finder.show-extensions": true
  },
  "dotfiles": {
    "repo": "owner/dotfiles",
    "links": [{ "source": "zsh/.zshrc", "target": "~/.zshrc" }],
    "installScript": "script/setup"
  },
  "importedFrom": { "repo": "owner/config-repo", "path": "config.json", "commit": "<sha>" }
}
```

### Format and versioning

1. **Use an integer `schemaVersion` and a `$schema` URL, both required.** winutil has to guess its format from property names. If the version is newer than the app understands, refuse to read it and change nothing, as `.chezmoiversion` does.
2. **Keep the version of the format separate from the Catalog's.** A Config refers to Catalog entries only by ID. winutil tried embedding package data in the Config and removed it.
3. **Parse strict RFC 8259 JSON, and fix each field's shape.** A list stays a list even with one entry, unlike winutil's export. Don't allow `null`, so absence is the only way to say "unmanaged".
4. **Write it the same way every time:** sorted lists, fixed key order, no timestamps in the app's normal writes. Adopting one app should produce a one-line Git diff.
5. **Preserve what the app doesn't recognise when it writes back.** An older app that rewrites a newer Config shouldn't silently drop Items. This matters because the app writes the file, not only people.

### Items and IDs

6. **Use stable, namespaced IDs with no UI naming in them,** such as `app.firefox`. winutil's `WPFInstall` prefix ties its public format to its UI toolkit.
7. **Give the Catalog a lifecycle for every ID.** Borrow Homebrew's and nix-darwin's: `aliases` for renames, `deprecated` with a reason and replacement, and `removed` with a message. Never reuse an ID. The app can then rewrite an old ID during Import and show that in the preview.
8. **Split validation into two stages.** A structural error rejects the file before anything happens. An unknown Item, package or setting is reported on that entry alone, and the rest proceeds. winutil's all-or-nothing rule protects against partial state, but it conflicts with this project's rule that one failed Item never stops a Run.

### Settings

9. **Key settings by Catalog ID and validate values against types in the Catalog.** nix-darwin shows this works. The Catalog entry holds the `defaults` domain, key and type, so a Config never names them directly.
10. **Absent means unmanaged.** Removing a setting from a Config leaves the Machine alone. nix-darwin behaves this way, and its docs show the cost: people need a separate way to go back.
11. **Don't put factory values in the Catalog for undo. Capture what was there instead.** winutil's hard-coded `OriginalValue` restores what the catalog assumes rather than what the user had. Record the prior value at first Apply, including "the key didn't exist", which accounts for over half of winutil's registry values.
12. **Have the Catalog declare scope, effect and OS range for each setting:** user or system; immediate, restart an app, or log out; and the macOS versions it works on. Then the app can say "applied, takes effect after logout" rather than reporting a misleading success (nix-darwin #658). A setting unsupported on this macOS should be reported as skipped.

### Packages

13. **List packages by name, with no options.** Allow no `args`, `postinstall`, `link`, `restart_service`, `trusted` or conditionals. Each is either code, or a way to change how Homebrew behaves. Any option the Catalog needs can go on a Catalog Item, where it gets reviewed.
14. **Require fully qualified names for third-party packages** (`owner/tap/name`), so the preview can identify them from the file alone. Keep `taps` only for custom URLs. Validate names against a strict pattern, which matters if the engine ever generates a Brewfile. The exact pattern needs checking against Homebrew's token rules.
15. **Keep approvals out of the Config.** Item-level trust should be the default, and whole-tap trust a separate local choice, as [Homebrew advises][bt-trust].
16. **Don't let the engine depend on `brew bundle` flags, or use `brew bundle cleanup` at all.** Two of nix-darwin's most-reacted issues (#1787, #1369) came from Homebrew changing these flags, and `cleanup` resets the trust store and has removed apps Homebrew never managed. Per-Item `brew` commands also give per-Item results, which the Run needs anyway.
17. **Treat "outdated" as separate from Drift.** A Config says what should be present, not which version. Bundle's [`--no-upgrade` check][bm-865] already separates the two.

### Removal and safety

18. **Leave out `cleanup`, `exact`, `force` and any unattended-approval field.** A shared Config shouldn't be able to remove software, skip a preview or pre-approve anything. Removal is a choice made per Run, on the Machine.
19. **Don't add conditionals, templates, includes or `extends` in version 1.** Each adds a way for the preview to be wrong. chezmoi's reasoning on multiple sources applies, and Homebrew's docs show layered files don't help with removal. This can be revisited, since adding fields later won't break existing Configs.

### Machine values and provenance

20. **Keep Machine values out of the Config format entirely.** The Catalog can declare that `tool.git` needs a name and email. The app then asks up front, as `promptStringOnce` does, and saves the answers in the local file. Rejecting those IDs in the Config schema makes accidental commits fail validation.
21. **Record `importedFrom` with a commit SHA.** Then the app can show what changed upstream without following it automatically. AutoPkg shows why this matters: a harmless shared file [can later turn malicious][a-trust].
22. **Limit `dotfiles` to a repo, named links and one optional script path.** A path with no arguments can be shown and approved. Tie the approval to the script's content hash, so a change asks again, as AutoPkg does.

### Outside the Config file

These don't belong in the Config, but the format assumes them.

23. **A local record of what was last applied, per Machine and never committed.** It lets Drift be classified:

    | Config vs last applied | Machine vs last applied | Meaning | Offer |
    |---|---|---|---|
    | same | same | In sync | Nothing |
    | changed | same | Config moved | Apply |
    | same | changed | Machine moved | Adopt or Apply |
    | changed | changed, and now equal | Both reached the same place | Update the record silently |
    | changed | changed, and now different | Conflict | Choose per entry |
    | no record | matches | Already satisfied | Record it |
    | no record | differs | Never applied | Apply |

24. **A read path for every Catalog entry.** A setting or Item the app can't read can't show Drift or be Adopted. winutil's 13 script-only tweaks are the example to avoid. Prefer structured output to parsing tables, given #4919. This is the concrete reason ADR 0001's no-shell rule helps: everything stays readable as well as writable.
25. **Named per-Item results.** Ninite's vocabulary is a good start: installed, already current, failed download, failed install, skipped because running, skipped as unsupported, and waiting in the Needs-You Queue. Don't report success unless a re-read confirms it, which addresses winutil's #2711 and #4821.
26. **A timeout on every step,** so an installer waiting for input lands in the Needs-You Queue rather than hanging the Run (winutil #5021).

## Decisions for the owner

The evidence doesn't settle these:

- **Raw preference writes.** nix-darwin's [`CustomUserPreferences`][nd-custom] lets a Config write any `defaults` key. That's flexible and contains no shell, but it isn't typed, can't be checked against the Catalog, and a shared Config could set something sensitive. I'd leave it out of version 1, since it can be added later.
- **Version pinning.** Homebrew offers [`pin: true`][bb-pin] but no real lock. Leaving it out keeps the format honest about what it can promise. Some people will want it for a tool that breaks on upgrade.
- **Mac App Store apps.** They aren't in the map's list of synced things. nix-darwin's experience (#1694, #1860) suggests they need the Needs-You Queue and a real account, and shouldn't be plain package entries.
- **Unknown Items at Run time.** Should a Config with entries the Catalog no longer knows be allowed to run with those skipped, or should it wait for a person? Skipping fits the map's "walk away" rule, and the preview can flag them first.
- **Notes.** JSON has no comments, while `brew bundle dump` writes a description beside each package by default. An optional `notes` map is simple, but it's one more thing for the app to round-trip.
- **Starter Configs.** winutil's [presets][w-preset] and its IPv6 dispute show that any default set carries opinions and takes complaints. Whether to ship them belongs with the first-run work, not the format.

## What I couldn't verify

- **Ninite user complaints.** It has no public tracker I could find, and the forum threads a search surfaced wouldn't load. A proxy for demand is all I have.
- **Ninite's internals.** Its catalog process and status codes are known only from its own docs, which also disagree on SHA-1 and SHA-256.
- **Runtime behaviour.** Everything about behaviour comes from reading docs and source, since no project code was run. That includes the Homebrew trust behaviour, which is documented but wasn't exercised here.
- **Complaint frequency.** Tracker searches show what gets reported, not how common it is. Where issues are listed, treat them as examples rather than rates. A few winutil and Homebrew issues were read by title only, and are marked. Homebrew's tracker was only lightly sampled.
- **The smaller tools.** Mackup, Installomator, AutoPkg, Strap and mac-dev-playbook were read from their READMEs, wikis, a sample of files and a few issues, not in depth. Comparable tools beyond these weren't surveyed.
- **Project glossary and ADRs.** `GLOSSARY.md` and `docs/adr/` aren't on `main` yet. Vocabulary and the ADR 0001 rule came from the map (#2) and the ticket brief.

## Other sources

Inline links point to each claim's source. These were surfaced by search but wouldn't load, so no claim relies on them: forum threads surfaced for Ninite on [Tom's Hardware](https://forums.tomshardware.com/threads/anyone-familiar-with-ninite.2709498), [Technibble](https://technibble.com/forums/threads/ninite-installs-excessively-slow-lately.43663) and [toolslib](https://toolslib.net/forum/viewthread/13981).

[w-config]: https://github.com/ChrisTitusTech/winutil/tree/2ef5e5dc7852634bc40ad789a3d1115ee3913c41/config
[w-spec]: https://github.com/ChrisTitusTech/winutil/blob/2ef5e5dc7852634bc40ad789a3d1115ee3913c41/SPEC.md#build-model
[w-spec-config]: https://github.com/ChrisTitusTech/winutil/blob/2ef5e5dc7852634bc40ad789a3d1115ee3913c41/SPEC.md#configuration-contract
[w-spec-107]: https://github.com/ChrisTitusTech/winutil/blob/2ef5e5dc7852634bc40ad789a3d1115ee3913c41/SPEC.md?plain=1#L107
[w-spec-110]: https://github.com/ChrisTitusTech/winutil/blob/2ef5e5dc7852634bc40ad789a3d1115ee3913c41/SPEC.md?plain=1#L110
[w-apps]: https://github.com/ChrisTitusTech/winutil/blob/2ef5e5dc7852634bc40ad789a3d1115ee3913c41/config/applications.json
[w-tweaks]: https://github.com/ChrisTitusTech/winutil/blob/2ef5e5dc7852634bc40ad789a3d1115ee3913c41/config/tweaks.json
[w-preset]: https://github.com/ChrisTitusTech/winutil/blob/2ef5e5dc7852634bc40ad789a3d1115ee3913c41/config/preset.json
[w-auto-keep]: https://github.com/ChrisTitusTech/winutil/blob/2ef5e5dc7852634bc40ad789a3d1115ee3913c41/docs/src/content/docs/guides/automation.mdx?plain=1#L57
[w-auto-25]: https://github.com/ChrisTitusTech/winutil/blob/2ef5e5dc7852634bc40ad789a3d1115ee3913c41/docs/src/content/docs/guides/automation.mdx?plain=1#L25
[w-auto-40]: https://github.com/ChrisTitusTech/winutil/blob/2ef5e5dc7852634bc40ad789a3d1115ee3913c41/docs/src/content/docs/guides/automation.mdx?plain=1#L40
[w-impex]: https://github.com/ChrisTitusTech/winutil/blob/2ef5e5dc7852634bc40ad789a3d1115ee3913c41/functions/public/Invoke-WPFImpex.ps1
[w-impex-87]: https://github.com/ChrisTitusTech/winutil/blob/2ef5e5dc7852634bc40ad789a3d1115ee3913c41/functions/public/Invoke-WPFImpex.ps1#L87
[w-impex-92]: https://github.com/ChrisTitusTech/winutil/blob/2ef5e5dc7852634bc40ad789a3d1115ee3913c41/functions/public/Invoke-WPFImpex.ps1#L92
[w-getinstalled]: https://github.com/ChrisTitusTech/winutil/blob/2ef5e5dc7852634bc40ad789a3d1115ee3913c41/functions/public/Invoke-WPFGetInstalled.ps1
[w-current-46]: https://github.com/ChrisTitusTech/winutil/blob/2ef5e5dc7852634bc40ad789a3d1115ee3913c41/functions/private/Invoke-WinUtilCurrentSystem.ps1#L46
[w-current-90]: https://github.com/ChrisTitusTech/winutil/blob/2ef5e5dc7852634bc40ad789a3d1115ee3913c41/functions/private/Invoke-WinUtilCurrentSystem.ps1#L90
[w-tweak-56]: https://github.com/ChrisTitusTech/winutil/blob/2ef5e5dc7852634bc40ad789a3d1115ee3913c41/functions/private/Invoke-WinUtilTweaks.ps1#L56
[w-tweak-90]: https://github.com/ChrisTitusTech/winutil/blob/2ef5e5dc7852634bc40ad789a3d1115ee3913c41/functions/private/Invoke-WinUtilTweaks.ps1#L90
[w-autorun]: https://github.com/ChrisTitusTech/winutil/blob/2ef5e5dc7852634bc40ad789a3d1115ee3913c41/functions/public/Invoke-WinUtilAutoRun.ps1#L19
[w-readme]: https://github.com/ChrisTitusTech/winutil/blob/2ef5e5dc7852634bc40ad789a3d1115ee3913c41/README.md#quick-start
[w-release]: https://github.com/ChrisTitusTech/winutil/blob/2ef5e5dc7852634bc40ad789a3d1115ee3913c41/.github/workflows/pre-release.yaml
[w-suggest]: https://github.com/ChrisTitusTech/winutil/blob/2ef5e5dc7852634bc40ad789a3d1115ee3913c41/.github/DISCUSSION_TEMPLATE/app-suggestion.yaml
[w-1098]: https://github.com/ChrisTitusTech/winutil/issues/1098
[w-1118]: https://github.com/ChrisTitusTech/winutil/issues/1118
[w-1726]: https://github.com/ChrisTitusTech/winutil/issues/1726
[w-1742]: https://github.com/ChrisTitusTech/winutil/issues/1742
[w-2433]: https://github.com/ChrisTitusTech/winutil/issues/2433
[w-2484]: https://github.com/ChrisTitusTech/winutil/issues/2484
[w-2641]: https://github.com/ChrisTitusTech/winutil/issues/2641
[w-2711]: https://github.com/ChrisTitusTech/winutil/issues/2711
[w-3061]: https://github.com/ChrisTitusTech/winutil/issues/3061
[w-3645]: https://github.com/ChrisTitusTech/winutil/issues/3645
[w-4821]: https://github.com/ChrisTitusTech/winutil/issues/4821
[w-4919]: https://github.com/ChrisTitusTech/winutil/issues/4919
[w-5021]: https://github.com/ChrisTitusTech/winutil/issues/5021
[w-5095]: https://github.com/ChrisTitusTech/winutil/issues/5095
[w-5130]: https://github.com/ChrisTitusTech/winutil/issues/5130
[w-5142]: https://github.com/ChrisTitusTech/winutil/issues/5142
[w-5190]: https://github.com/ChrisTitusTech/winutil/issues/5190
[n-home]: https://ninite.com/
[n-how]: https://ninite.com/help/how-ninite-works/
[n-sec]: https://ninite.com/security
[n-share]: https://ninite.com/help/home/share.html
[n-select]: https://ninite.com/help/home/selection.html
[n-errors]: https://ninite.com/help/errors/
[n-not]: https://ninite.com/help/notfeatures/
[n-loc]: https://ninite.com/help/notfeatures/location.html
[n-more]: https://ninite.com/help/notfeatures/more.html
[n-drivers]: https://ninite.com/help/notfeatures/drivers.html
[n-audit]: https://ninite.com/help/features/audit.html
[n-switch]: https://ninite.com/help/features/switches.html
[nd-dock]: https://github.com/nix-darwin/nix-darwin/blob/ff0e21a414d4547183396da1b44a66f2577b65e9/modules/system/defaults/dock.nix
[nd-global]: https://github.com/nix-darwin/nix-darwin/blob/ff0e21a414d4547183396da1b44a66f2577b65e9/modules/system/defaults/NSGlobalDomain.nix#L44-L52
[nd-write-11]: https://github.com/nix-darwin/nix-darwin/blob/ff0e21a414d4547183396da1b44a66f2577b65e9/modules/system/defaults-write.nix#L11
[nd-write-156]: https://github.com/nix-darwin/nix-darwin/blob/ff0e21a414d4547183396da1b44a66f2577b65e9/modules/system/defaults-write.nix#L156
[nd-primary]: https://github.com/nix-darwin/nix-darwin/blob/ff0e21a414d4547183396da1b44a66f2577b65e9/modules/system/primary-user.nix
[nd-alf]: https://github.com/nix-darwin/nix-darwin/blob/ff0e21a414d4547183396da1b44a66f2577b65e9/modules/system/defaults/alf.nix
[nd-state]: https://github.com/nix-darwin/nix-darwin/blob/ff0e21a414d4547183396da1b44a66f2577b65e9/modules/system/version.nix#L34
[nd-changelog]: https://github.com/nix-darwin/nix-darwin/blob/ff0e21a414d4547183396da1b44a66f2577b65e9/CHANGELOG
[nd-etc]: https://github.com/nix-darwin/nix-darwin/blob/ff0e21a414d4547183396da1b44a66f2577b65e9/modules/system/etc.nix#L82
[nd-hb-78]: https://github.com/nix-darwin/nix-darwin/blob/ff0e21a414d4547183396da1b44a66f2577b65e9/modules/homebrew.nix#L78
[nd-hb-93]: https://github.com/nix-darwin/nix-darwin/blob/ff0e21a414d4547183396da1b44a66f2577b65e9/modules/homebrew.nix#L93
[nd-rebuild]: https://github.com/nix-darwin/nix-darwin/blob/ff0e21a414d4547183396da1b44a66f2577b65e9/pkgs/nix-tools/darwin-rebuild.sh#L19
[nd-custom]: https://github.com/nix-darwin/nix-darwin/blob/ff0e21a414d4547183396da1b44a66f2577b65e9/modules/system/defaults/CustomPreferences.nix#L22
[nd-149]: https://github.com/nix-darwin/nix-darwin/issues/149
[nd-554]: https://github.com/nix-darwin/nix-darwin/issues/554
[nd-658]: https://github.com/nix-darwin/nix-darwin/issues/658
[nd-1041]: https://github.com/nix-darwin/nix-darwin/issues/1041
[nd-1207]: https://github.com/nix-darwin/nix-darwin/issues/1207
[nd-1369]: https://github.com/nix-darwin/nix-darwin/issues/1369
[nd-1509]: https://github.com/nix-darwin/nix-darwin/issues/1509
[nd-1694]: https://github.com/nix-darwin/nix-darwin/issues/1694
[nd-1787]: https://github.com/nix-darwin/nix-darwin/issues/1787
[nd-1829]: https://github.com/nix-darwin/nix-darwin/issues/1829
[nd-1860]: https://github.com/nix-darwin/nix-darwin/issues/1860
[c-concepts]: https://github.com/twpayne/chezmoi/blob/745ad561962c3b47742e715f13bd2ff51c06e7dd/assets/chezmoi.io/docs/reference/concepts.md
[c-apply]: https://github.com/twpayne/chezmoi/blob/745ad561962c3b47742e715f13bd2ff51c06e7dd/assets/chezmoi.io/docs/reference/commands/apply.md
[c-arch]: https://github.com/twpayne/chezmoi/blob/745ad561962c3b47742e715f13bd2ff51c06e7dd/assets/chezmoi.io/docs/developer-guide/architecture.md?plain=1#L81-L84
[c-status]: https://github.com/twpayne/chezmoi/blob/745ad561962c3b47742e715f13bd2ff51c06e7dd/assets/chezmoi.io/docs/reference/commands/status.md
[c-merge]: https://github.com/twpayne/chezmoi/blob/745ad561962c3b47742e715f13bd2ff51c06e7dd/assets/chezmoi.io/docs/reference/commands/merge.md
[c-diff]: https://github.com/twpayne/chezmoi/blob/745ad561962c3b47742e715f13bd2ff51c06e7dd/assets/chezmoi.io/docs/reference/commands/diff.md
[c-verify]: https://github.com/twpayne/chezmoi/blob/745ad561962c3b47742e715f13bd2ff51c06e7dd/assets/chezmoi.io/docs/reference/commands/verify.md
[c-readd]: https://github.com/twpayne/chezmoi/blob/745ad561962c3b47742e715f13bd2ff51c06e7dd/assets/chezmoi.io/docs/reference/commands/re-add.md
[c-unmanaged]: https://github.com/twpayne/chezmoi/blob/745ad561962c3b47742e715f13bd2ff51c06e7dd/assets/chezmoi.io/docs/reference/commands/unmanaged.md
[c-machines]: https://github.com/twpayne/chezmoi/blob/745ad561962c3b47742e715f13bd2ff51c06e7dd/assets/chezmoi.io/docs/user-guide/manage-machine-to-machine-differences.md
[c-prompt]: https://github.com/twpayne/chezmoi/blob/745ad561962c3b47742e715f13bd2ff51c06e7dd/assets/chezmoi.io/docs/reference/templates/init-functions/promptStringOnce.md
[c-design]: https://github.com/twpayne/chezmoi/blob/745ad561962c3b47742e715f13bd2ff51c06e7dd/assets/chezmoi.io/docs/user-guide/frequently-asked-questions/design.md?plain=1#L287
[c-design-326]: https://github.com/twpayne/chezmoi/blob/745ad561962c3b47742e715f13bd2ff51c06e7dd/assets/chezmoi.io/docs/user-guide/frequently-asked-questions/design.md?plain=1#L326
[c-multi]: https://github.com/twpayne/chezmoi/blob/745ad561962c3b47742e715f13bd2ff51c06e7dd/assets/chezmoi.io/docs/user-guide/frequently-asked-questions/design.md?plain=1#L192
[c-attrs]: https://github.com/twpayne/chezmoi/blob/745ad561962c3b47742e715f13bd2ff51c06e7dd/assets/chezmoi.io/docs/reference/source-state-attributes.md
[c-targets]: https://github.com/twpayne/chezmoi/blob/745ad561962c3b47742e715f13bd2ff51c06e7dd/assets/chezmoi.io/docs/reference/target-types.md
[c-version]: https://github.com/twpayne/chezmoi/blob/745ad561962c3b47742e715f13bd2ff51c06e7dd/assets/chezmoi.io/docs/reference/special-files/chezmoiversion.md
[c-external]: https://github.com/twpayne/chezmoi/blob/745ad561962c3b47742e715f13bd2ff51c06e7dd/assets/chezmoi.io/docs/reference/special-files/chezmoiexternal-format.md
[c-quick]: https://github.com/twpayne/chezmoi/blob/745ad561962c3b47742e715f13bd2ff51c06e7dd/assets/chezmoi.io/docs/quick-start.md?plain=1#L194
[c-scripts]: https://github.com/twpayne/chezmoi/blob/745ad561962c3b47742e715f13bd2ff51c06e7dd/assets/chezmoi.io/docs/user-guide/use-scripts-to-perform-actions.md?plain=1#L23
[c-423]: https://github.com/twpayne/chezmoi/issues/423
[c-424]: https://github.com/twpayne/chezmoi/issues/424
[c-1348]: https://github.com/twpayne/chezmoi/issues/1348
[c-2162]: https://github.com/twpayne/chezmoi/issues/2162
[c-4361]: https://github.com/twpayne/chezmoi/issues/4361
[c-5128]: https://github.com/twpayne/chezmoi/issues/5128
[bb-285]: https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/docs/Brew-Bundle-and-Brewfile.md?plain=1#L285
[bb-303]: https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/docs/Brew-Bundle-and-Brewfile.md?plain=1#L303
[bb-173]: https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/docs/Brew-Bundle-and-Brewfile.md?plain=1#L173
[bb-dump]: https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/docs/Brew-Bundle-and-Brewfile.md#brew-bundle-dump
[bb-trusted]: https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/docs/Brew-Bundle-and-Brewfile.md#trusted
[bb-pin]: https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/docs/Brew-Bundle-and-Brewfile.md#advanced-brewfiles
[bb-22622]: https://github.com/Homebrew/brew/issues/22622
[bb-22753]: https://github.com/Homebrew/brew/issues/22753
[bb-22756]: https://github.com/Homebrew/brew/issues/22756
[bm-865]: https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/docs/Manpage.md?plain=1#L865
[bm-cleanup]: https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/docs/Manpage.md?plain=1#L666
[bt-trust]: https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/docs/Tap-Trust.md
[bt-trust-36]: https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/docs/Tap-Trust.md?plain=1#L36
[bt-trust-47]: https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/docs/Tap-Trust.md?plain=1#L47
[bt-teams-99]: https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/docs/Homebrew-for-Teams.md?plain=1#L99
[bt-teams-125]: https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/docs/Homebrew-for-Teams.md?plain=1#L125
[bt-teams-132]: https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/docs/Homebrew-for-Teams.md?plain=1#L132
[bd-life]: https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/docs/Deprecating-Disabling-and-Removing.md
[bd-cask]: https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/docs/Cask-Cookbook.md#stanza-deprecate--disable
[bd-rename]: https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/docs/Rename-A-Formula.md
[bd-api]: https://formulae.brew.sh/docs/api/
[bd-firefox]: https://formulae.brew.sh/api/cask/firefox.json
[bd-allcasks]: https://formulae.brew.sh/api/cask.json
[m-readme]: https://github.com/lra/mackup#link-mode
[m-2035]: https://github.com/lra/mackup/issues/2035
[m-1924]: https://github.com/lra/mackup/issues/1924
[i-does]: https://github.com/Installomator/Installomator/wiki/What-it-does
[i-label]: https://github.com/Installomator/Installomator/blob/main/fragments/labels/firefoxpkg.sh
[i-3162]: https://github.com/Installomator/Installomator/issues/3162
[i-2974]: https://github.com/Installomator/Installomator/issues/2974
[a-trust]: https://github.com/autopkg/autopkg/wiki/AutoPkg-and-recipe-parent-trust-info
[s-readme]: https://github.com/MikeMcQuaid/strap#readme
[p-readme]: https://github.com/geerlingguy/mac-dev-playbook#readme
[p-config]: https://github.com/geerlingguy/mac-dev-playbook/blob/master/default.config.yml

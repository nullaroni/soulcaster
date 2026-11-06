# Config format (prototype)

**Throwaway prototype answering [#11](https://github.com/nullaroni/soulcaster/issues/11).** It isn't merged, and no app reads these files yet. The Catalog stub is illustrative only. The prototype skill's click-through HTML demo was skipped, because this question is about a file format and nobody is present to drive one: the schema, examples and check script are the prototype.

## Conclusion

A Config is one self-contained JSON file with a version, a name and up to four optional sections. Every entry is a string in a sorted list or one key in a map, so each change is about one line in Git. Items and packages are named, settings get typed values, and the Dotfiles Repo is a repo plus its links.

```json
{
  "$schema": "https://raw.githubusercontent.com/nullaroni/soulcaster/main/schema/config-1.schema.json",
  "schemaVersion": 1,
  "name": "Work Mac",
  "items": ["gh", "gh-sign-in", "ghostty", "git-identity"],
  "packages": {
    "formulae": ["mobile-dev-inc/tap/maestro", "neovim"],
    "casks": ["azure-cli"],
    "taps": ["jetbrains/utils"]
  },
  "settings": {
    "dock.autohide": true,
    "dock.position": "left"
  },
  "dotfiles": {
    "repo": "your-github-username/dotfiles",
    "links": { "~/.zshrc": "zsh/zshrc" },
    "installScript": "script/setup"
  }
}
```

Only `schemaVersion` and `name` are required. The app writes lists one entry per line, as in [`examples/`](examples/); they're shortened here.

| Question | Decision |
|---|---|
| Shape | Flat and closed: any field the schema doesn't name is an error. That is what keeps out shell commands, trust grants, removal flags and includes. |
| Item IDs | Lowercase product names (`ghostty`) and `<surface>.<name>` for settings (`dock.autohide`). The Catalog handles renames and retirement, and an ID is never reused. |
| Dotfiles Repo | `repo`, a `links` map from Machine location to repo path, and one optional script path. |
| Versioning | Integer `schemaVersion`. An older app refuses a newer Config and changes nothing. An unknown field rejects the whole file. An unknown ID skips only that entry and is kept in the file. |
| Machine-local | Git identity, the Dotfiles Repo's clone location, approvals, the last-applied record and which Config this Machine follows. If Adopt could write it, it's in the Config. Everything else stays local and is never Adopted. |
| Removal | Not expressible. Dropping an entry only creates Drift, which a person resolves per Machine. |
| Includes | None. The common-plus-role split becomes two complete files. |
| Taps and scripts | Worked out from the file and the Catalog alone, so Import can list them for separate approval. |

Validation passes: all 5 valid examples are accepted and all 15 invalid ones refused. Every Brewfile entry is covered, and adding a package changes 1 to 3 lines. See [Validation](#validation). No real Run, Adopt or Import happened, and nothing was written to a Machine.

## 1. Shape

**One line per entry.** Lists hold strings, and maps hold one value per key. Standard pretty-printers keep these on one line each, so a person's editor and the app produce the same layout.

**Items and packages are separate lists.** `items` holds Catalog IDs: apps, CLIs and sign-ins. `packages` holds Homebrew names that the Catalog doesn't cover. A package has no options, which rules out `args`, `postinstall`, `restart_service` and `trusted`. Any option belongs on a reviewed Catalog Item.

**Settings hold plain values.** They're booleans, numbers, strings or lists of strings, using readable values such as `"list"` rather than `"Nlsv"`. The Catalog holds the `defaults` domain, the key, the type and the stored value, so a Config never names them. A home-folder path such as `NewWindowTargetPath` also stays out of the file: `finder.new-window-target` is just `"documents"`.

**Sections are optional.** The app omits empty ones.

**The app always writes the same form:** 2-space indent, final newline, fixed key order, and sorted `items`, `packages`, `settings` and `links`. A list that is a setting's value, such as `dock.apps`, keeps its order because the order means something. If a person's edits left the file in another form, the app commits the reformat separately first, so the Adopt commit stays small.

**Where Adopt writes.** Every change goes to the one file this Machine follows, and each kind of change has one destination:

| Change on the Machine | Destination |
|---|---|
| Package that a Catalog Item installs | `items` |
| Any other formula or cask | `packages.formulae` or `packages.casks`, as `owner/tap/name` if third-party |
| A setting the Catalog exposes | `settings` |
| Anything else | Not Adopted. Other apps show as Untracked. |

Adopt never writes `packages.taps` or `dotfiles`, because neither is in the map's list of what Sync covers. If a package is listed both ways, it installs once and the next reviewed write keeps only the Item.

**Measured size of a change** in `owner-work.json` (109 lines), from the [check script](#validation):

| Change | Lines changed |
|---|---|
| Add a package mid-list, add a setting, add a link, remove an Item | 1 |
| Change a setting | 2 (one removed, one added) |
| Add at the end of a list | 3, because JSON needs a comma on the previous line |

The 3-line case can't be avoided in JSON. The other hand-editing cost is having no comments and no trailing commas. In return, the owner gets the format they chose, editor completion through `$schema`, and validation errors that name the exact path.

## 2. Item IDs

**Naming**

- **Items** are the product name in lowercase kebab-case: `ghostty`, `claude-code`, `gh-sign-in`.
- **Settings** are `<surface>.<name>`, where the surface is what a person sees, not a `defaults` domain: `dock.autohide`, not `com.apple.dock.autohide`.
- **Booleans say what true means:** `dock.show-recents`, not `dock.hide-recents`.
- **Neither has a category or install method in it.** `tailscale` is the ID whether it arrives as the `tailscale-app` cask or something else. `tailscale` is also both an app and a CLI, so an `app.` or `cli.` prefix would force an early call that a later rename has to undo.

The grammars differ, so a setting ID in `items` fails the schema.

**Lifecycle**, all recorded in the Catalog:

| Catalog says | What the app does with a Config naming it |
|---|---|
| `renamedTo` | Treats it as the new ID with no Drift. The next reviewed write updates the file, and Import's preview shows the rename. Only for the same kind and value type. Otherwise the old ID is retired and a new one added. |
| `deprecated` | Still installs it, and shows the message and replacement. |
| `retired` | Skips it, reports the message and offers a replacement. It uninstalls nothing, keeps the entry in the file, and doesn't switch to the replacement on its own. |
| Nothing (unknown ID) | Skips and reports that entry only, keeps it in the file, and carries on. This covers typos and IDs from a newer Catalog. |

An ID stays in the Catalog as a record forever and is never reused. Approvals follow what an ID resolves to, not the ID, so a rename that lands on a different tap asks again.

## 3. Dotfiles Repo

- **`repo`** is `owner/repo` on GitHub or an `https://` URL. SSH isn't allowed because a bare Mac has no key yet. Credentials in the URL are refused.
- **`links`** maps a location under `~/` to a path inside the repo. Using a map means two links can't claim the same location, and each link stays one line. A list of `{source, target}` objects would take four lines each.
- **`installScript`** is one path with no arguments and no spaces. A file is never run because of its name, so a repo's `install.sh` stays unused unless the Config names it. **This changes today's behaviour:** `scripts/dotfiles.sh` runs `install.sh` automatically, instead of linking.
- **Links and scripts can both be present.** Links are applied first, then the script runs if it's approved. A failing script doesn't stop the Run.
- **No clone location.** It defaults to `~/Developer/dotfiles`, as it does today, and can be overridden locally. Otherwise a stranger's Config could choose where a repo lands, such as `~/Library/LaunchAgents`.
- **No branch or commit pin.** The app follows the default branch and shows only whether it's behind or has unpushed changes, as the map says.

## 4. Versioning

| Case | What the app does |
|---|---|
| `schemaVersion` is newer than the app knows | Reads only that field, then refuses. No Run, Apply or Adopt, and the file isn't rewritten. Says to update Soulcaster. |
| `schemaVersion` is older | Reads it through an in-memory upgrade. The file keeps its version until someone uses a feature that needs a newer one, and that bump shows in the reviewed diff. |
| Missing or not an integer | Rejects the whole file. |
| Unknown field at the current version | Rejects the whole file and changes nothing. The error names the field. |
| Unknown Item or setting ID, or a value the Catalog refuses | Skips and reports that entry, keeps it in the file, and continues. |

**Any new field means a new `schemaVersion`.** Ignoring unknown fields would make a typo like `setings` silently unmanage everything, and would let a future field slip past Import's preview. Because old apps never rewrite a Config with fields they don't know, there's nothing to preserve at the field level. Preservation is still needed for entries: an older app must keep IDs from a newer Catalog when it writes.

The cost is that using a new field forces every Machine following that Config to update the app first. Writing the lowest version that can express the file keeps this rare. The Catalog has no version in the Config, because a Config refers to entries only by ID.

The parser must be strict: plain RFC 8259 and **duplicate keys refused**, since parsers disagree on which value wins and Import's preview could show a different one from what the engine uses. A schema can't express this, so `duplicate-key.json` is caught by the strict parser.

## 5. What stays on the Machine

**The boundary:** if Adopt could write it, it's in the Config. Local records fill slots the Catalog declares and never add to or override Config entries. So Drift only ever compares one file with the Machine.

| Local | Why it isn't in the Config | What the format provides |
|---|---|---|
| Git identity and other Catalog-declared values | Personal, and committed Configs are often public | The `git-identity` Item says identity is wanted, and the Catalog declares the value names. Leaving the Item out means the app doesn't touch Git identity, so it's never required. `git.user.*` keys fail the schema. |
| Dotfiles Repo clone location | Differs per Machine, and is risky from a stranger | Link sources are relative to the repo root. |
| Which Config this Machine follows | One file, many Machines | A Config Repo plus a path. |
| Approvals | An author must never approve for an importer | Stable keys: `owner/tap`, `owner/tap/name` and, for a script, repo plus path, tied to the script's content hash. A change asks again. No approval field exists, and `trusted` fails the schema. |
| Last-applied record, including prior setting values | Differs per Machine, and is what lets Drift show which side changed | One spelling for every entry, which serves as its key: Item ID after following renames, `formula:<name>`, `cask:<name>`, setting ID, or link location. Package names can't be written two ways, because `homebrew/core/x` and uppercase are refused. |

The full design of these records isn't specified here. [#13](https://github.com/nullaroni/soulcaster/issues/13) and [#15](https://github.com/nullaroni/soulcaster/issues/15) own the behaviour around them.

## 6. Removal

A Config can't express removal. It has no `remove`, `absent`, `cleanup`, `exact` or `force`, and `null` is refused. Deleting a line is the only way to change what's listed, and it has one effect:

| Situation | Result |
|---|---|
| Never in the Config, never applied | Nothing. Other apps show as Untracked. |
| Dropped from the Config, was applied here, still on the Machine | Drift: "no longer in this Config". The person chooses to keep it and stop tracking, remove it from this Machine with explicit confirmation, or add it back. Until they choose, nothing happens. |
| Removed on the Machine, still in the Config | Drift. Apply reinstalls it, or Adopt deletes the line. |
| Setting dropped from the Config | The value stays as it is. The app can offer the recorded prior value. |
| Link dropped from the Config | The link stays until a person confirms. |
| This Machine switches to another Config | Same as dropped, entry by entry. Nothing is removed. |

Other Machines learn of a removal the same way, by comparing the file with their own last-applied record. No tombstone is needed, so a shared Config can't trigger an uninstall. A Machine with no record of the entry sees nothing.

**One related case to show clearly in Import's preview:** a list-valued setting is a whole value. `dock.apps` replaces the Dock's contents, as `dockutil --remove all` does today, so a tile missing from that list leaves the Dock. The app itself stays installed.

## 7. No includes

A Machine follows one file, and a Config doesn't reference other Configs.

With `extends`, Adopt has no single right answer. A new package could go in the role file or the base. A package removed from a Machine that the base lists would need an override meaning "not this one", which is the removal field that section 6 rules out. Import would also have to fetch and preview several files, any of which could change after approval. [chezmoi's notes](https://github.com/twpayne/chezmoi/blob/745ad561962c3b47742e715f13bd2ff51c06e7dd/assets/chezmoi.io/docs/user-guide/frequently-asked-questions/design.md?plain=1#L192) and [Homebrew's on layered Brewfiles](https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/docs/Homebrew-for-Teams.md?plain=1#L132) point the same way.

So today's common-plus-role split becomes two complete files. `owner-personal.json` and `owner-work.json` differ by 7 lines out of about 110. A change meant for both must be made in both, which is the real cost. If that hurts in practice, the fix belongs in the app, such as "copy this change to my other Config", and leaves the format alone.

**Config Repo layout:** Configs live in `configs/*.json`. Importing `owner/repo` lists those files, and importing a URL can name any one file.

## 8. Taps and install scripts

Import can compute what needs separate approval without running anything:

- **Taps:** the union of `owner/tap` from third-party names in `packages`, entries in `packages.taps`, and the taps of Catalog Items in `items`. Third-party names must be fully qualified, so a bare name always means official Homebrew. The engine has to enforce that, because Homebrew may also find a bare name in a tap already on the Machine (not tested).
- **Catalog Items count too.** `aerospace` comes from a reviewed Catalog entry but still loads `nikitabobko/tap`, and tap code runs on the Machine whoever suggested it.
- **Script:** `dotfiles.installScript` with its `repo`. The app reads the script to show and hash it, without running it.

The check script prints this list for each example. For `owner-work.json` it finds all four Brewfile taps, even though only `jetbrains/utils` is written as a tap. How the approvals look is [#15](https://github.com/nullaroni/soulcaster/issues/15). The same approvals also apply when a later Adopt on another Machine brings in a new tap, not just at Import.

## Translating the owner's setup

[`owner-personal.json`](examples/owner-personal.json) and [`owner-work.json`](examples/owner-work.json) cover every Brewfile entry, checked by script against the Brewfiles.

| Today | In the Configs |
|---|---|
| `Brewfile.*` | Catalog-covered ones in `items`, the rest in `packages` |
| `kotlin.sh`, `kotlin-lsp.sh` | Items `kotlin` and `kotlin-lsp`. The `curl \| sh` and the pinned `262.7569.0` become Catalog recipes, as ADR 0001 requires. |
| `android.sh` | Item `android-sdk`, plus typed settings `android.platform`, `android.build-tools` and `android.emulator` in place of raw `sdkmanager` arguments such as `platforms;android-35`. `platform-tools` and `cmdline-tools` are part of the recipe. |
| `dock.sh`, `finder.sh`, `keyboard.sh`, `macos.sh`, `menu-bar.sh` | `settings`. Eight hot-corner keys become four settings, because each corner and its modifier are written together. |
| `dotfiles.sh` | `dotfiles`, with a placeholder repo |
| `git.sh`, except identity | `git.*` settings. Identity becomes the `git-identity` Item and local values. |

**Not in a Config, on purpose:**

| Today | Where it goes |
|---|---|
| Homebrew, Command Line Tools | Always installed, per the map |
| `docs/manual-steps.md`, `gh auth login` | Needs-You Queue, from the Catalog's `needsYou`. Only `gh-sign-in` is a Config choice. |
| `paths.sh` | Item recipes or the Dotfiles Repo |
| Restarts of Dock and Finder, offer to restart the Mac | Catalog apply steps and app behaviour |
| Weather's menu bar position (`100`), `yes \| sdkmanager --licenses` | Catalog recipe. The licenses are listed as a Needs-You step in the stub, because a Config shouldn't accept them for someone. |
| `mkdir ~/Developer` | Part of the clone and of the sidebar favorites recipe |

**Problems found in the current setup.** These were kept as they are, because a translation shouldn't quietly fix them:

- **`azure-cli` is no longer a formula.** homebrew-core [moved it to a cask](https://github.com/Homebrew/homebrew-core/blob/main/tap_migrations.json) on 2026-10-06, and `Brewfile.work` still says `brew`. It's listed under `casks`, which is the only change from the Brewfile. I didn't run `brew` to see how the old entry behaves.
- **`brew "junie"` is really `jetbrains/junie/junie`.** There's no such formula in homebrew-core. A bare name means official Homebrew in this format, so the stub's `junie` Item uses the full name.
- **`tap "jetbrains/utils"` looks unused.** Nothing in the Brewfiles comes from it. It does hold `kotlin-lsp.rb`, which the Brewfile's comment says is installed another way. It's the only entry in `packages.taps`. Dropping it would save a needless tap approval.
- **Already known to be broken, from [the settings research](https://github.com/nullaroni/soulcaster/blob/research/macos-settings/docs/research/macos-settings.md):** `Launchpad.app` in `dock.apps`, `sfltool` behind `finder.sidebar-favorites`, most of `menu-bar.extras`, and hot corners writing `0` where nix-darwin documents `1`. The Weather and Spotlight keys are unconfirmed.
- **The owner's real Dotfiles Repo wasn't available.** If it has an `install.sh`, today's script runs that and skips the links. The Config would then need `"installScript": "install.sh"`, which I couldn't check.

## Left out deliberately

| Left out | Why |
|---|---|
| Includes, conditionals, templates | Section 7. Each is a way for the preview to be wrong. |
| Version pins | Homebrew offers no real lock. A pin an Item needs, as `kotlin-lsp` does, lives in the Catalog. |
| Raw `defaults` writes | Untyped and can't be checked. A shared Config could set something sensitive. Propose a Catalog entry instead. |
| Mac App Store apps as packages | They need an account and the Needs-You Queue, so they're Catalog Items, as `xcode` is. |
| Custom tap URLs | No current user. Can arrive with a new `schemaVersion`. |
| `description`, `notes`, comments | Prose belongs in the Config Repo's README. A stranger's text in the preview is also a spoofing risk. |
| `importedFrom` | See below. |
| Branch or commit pin for the Dotfiles Repo | Section 3. |

## Rejected alternatives

| Alternative | Why not |
|---|---|
| One `items` list holding settings as `{id, value}` | Multi-line entries and larger diffs, with no gain. |
| A tombstone list such as `removed: [...]` | A shared file could then cause uninstalls, which is what [nix-darwin #1860](https://github.com/nix-darwin/nix-darwin/issues/1860) shows going wrong. |
| Tolerating unknown fields | Section 4. |
| Semver as `schemaVersion` | Only "can I read this?" matters, and an integer answers it. |
| Package options as objects | Reintroduces Brewfile behaviour and breaks one line per entry. |

**Where this departs from [the prior-art research](https://github.com/nullaroni/soulcaster/blob/research/prior-art/docs/research/prior-art.md):**

| It recommended | This format | Reason |
|---|---|---|
| `$schema` required | Optional, and the app always writes it | It only helps editors, so a hand-written file shouldn't fail without it. |
| Namespaced IDs such as `app.firefox` | `firefox` | Section 2. The research's real concern, UI names in IDs, is still met. |
| `importedFrom` with a commit | Left out, kept in the Machine's local record | Import is a copy and doesn't subscribe, so nothing in the file uses it. An author could also fill it in to look official. |
| `taps` only for custom URLs | `taps` only for taps with no named package | `jetbrains/utils` needs somewhere to go, and custom URLs have no user yet. |
| Preserve unrecognised data on write | Kept for entries, unnecessary for fields | Section 4. |

## Open risks

1. **Links can run code too, and the map doesn't cover that.** ADR 0001 and the map name taps and install scripts. But linking a stranger's `~/.zshrc` or a `~/Library/LaunchAgents` plist also runs their code, with no script involved, and the repo's contents can change later without the Config changing. The format lists every link target, so the preview can show them. **[#15](https://github.com/nullaroni/soulcaster/issues/15) should treat Importing someone else's Dotfiles Repo as a third approval.** This extends ADR 0001 and doesn't contradict it. I deliberately left out a blocklist in the schema, since these targets have legitimate uses and any list would be incomplete.
2. **The secret check is best effort.** The schema refuses secret-shaped key names and common token prefixes, but it can't recognise an arbitrary secret in a setting value or in `name`. The real protection is that there's no free-form field and values only reach typed Catalog settings. An unknown setting is kept in the file, so a mistyped key could still hold a secret until someone notices it in the report.
3. **Rust is untested.** The engine will be Rust, and these patterns use lookahead, which the plain `regex` crate doesn't support. Rust isn't installed here, so a validator must be confirmed at build time. Dialects differ: Python's `jsonschema` accepts `"ripgrep\n"`, and Ajv refuses it. JSON Schema specifies ECMA-262, so Ajv is right.
4. **Package names have no lifecycle in the Catalog.** `azure-cli` moving from formula to cask is exactly this. Items absorb such moves, but a name in `packages` doesn't. The engine has to handle Homebrew's renames and tap migrations and show the fix as a reviewed change. That detection is unbuilt and unproven.
5. **Adopt has to separate wanted formulae from dependencies,** or every dependency looks like Drift. Homebrew's "installed on request" data should do it, but that wasn't tested.
6. **`git.*` and `android.*` go beyond the map's "macOS settings".** They fit "settings values", but each needs a read path for Drift, or it's a value that can only be written.
7. **Setting IDs become public API at first release.** The names in the stub were picked for the examples. Review them before they ship, and every `store` value in the stub is copied from the scripts and research, untested.
8. **`$schema` points at a file that doesn't exist yet.** The schema has to be published on `main` at `schema/config-1.schema.json`, and that path must then stay put. Until then, editors can't fetch it. The app is unaffected.
9. **Comparing whole-list settings needs care.** The research notes that Dock entries are nested dictionaries, so `dock.apps` needs the Catalog to define how paths map to them.

## Validation

Run for real on 2026-11-06 with Python 3.9.6, `jsonschema` 4.25.1, Node 26.13.0 and Ajv 8.17.1. All tooling stayed in a temp directory. Run these from the repo root. They extract the scripts from this README, so what ran is what's shown:

```sh
TMP="$(mktemp -d)"
README=docs/prototypes/config-format/README.md
awk '/^```python$/{f=1;next} /^```$/{f=0} f' "$README" > "$TMP/check.py"
awk '/^```js$/{f=1;next} /^```$/{f=0} f' "$README" > "$TMP/check-ajv.mjs"

python3 -m venv "$TMP/venv"
"$TMP/venv/bin/pip" install --quiet --no-cache-dir jsonschema==4.25.1
"$TMP/venv/bin/python" "$TMP/check.py" docs/prototypes/config-format

npm_config_cache="$TMP/npm-cache" npm install --silent --no-audit --no-fund --prefix "$TMP/ajv" ajv@8.17.1
node "$TMP/check-ajv.mjs" "$TMP/ajv" docs/prototypes/config-format

rm -rf "$TMP"
```

Both validators exit 0. The check script covers the schema itself, each example, 35 edge cases, resolving against the Catalog stub, Brewfile coverage and Adopt diffs.

Last line of the check script's output: `result: all checks passed`.

<details>
<summary>Full output of the check script</summary>

```text
schema: valid against the 2020-12 metaschema

== valid examples: must parse strictly, pass the schema and already be in canonical form
PASS developer.json  (48 lines, canonical: True)
PASS lifecycle.json  (16 lines, canonical: True)
PASS minimal.json  (7 lines, canonical: True)
PASS owner-personal.json  (112 lines, canonical: True)
PASS owner-work.json  (109 lines, canonical: True)

== invalid examples: each must be refused
PASS credential-in-repo-url.json: refused at dotfiles/repo: 'https://someone:placeholder@github.com/example/dotfiles' does not match '^(?!.*/\\.{1,2}(?:/|$))(?:[A-Za-z0-9
PASS duplicate-key.json: refused by the strict parser: duplicate key ['name']
PASS git-identity-in-settings.json: refused at settings: 'git.user.email' should not be valid under {'pattern': '(?:^|[.-])(?:tokens?|secrets?|passwords?|passwd|passph
PASS include-another-config.json: refused at (top level): Additional properties are not allowed ('extends' was unexpected)
PASS link-escapes-home.json: refused at dotfiles/links: '/etc/zshrc' does not match '^~/(?!\\.{1,2}(?:/|$))(?!.*/\\.{1,2}(?:/|$))[A-Za-z0-9._+@ -]+(?:/[A-Za-z0-9._+@ 
PASS null-setting.json: refused at settings/dock.autohide: None is not valid under any of the given schemas
PASS package-injection.json: refused at packages/formulae/0: '--force' does not match '^(?!homebrew/)(?:[a-z0-9][a-z0-9._-]*/[a-z0-9][a-z0-9._-]*/)?[a-z0-9][a-z0-9._+@-]*$
PASS removal-field.json: refused at (top level): Additional properties are not allowed ('remove' was unexpected)
PASS script-with-arguments.json: refused at dotfiles/installScript: 'install.sh --yes && curl https://example.com' does not match '^(?!-)(?!\\.{1,2}(?:/|$))(?!.*/\\.{1,2}(?:/|$))
PASS secret-shaped-key.json: refused at settings: 'github.token' should not be valid under {'pattern': '(?:^|[.-])(?:tokens?|secrets?|passwords?|passwd|passphra
PASS secret-shaped-value.json: refused at settings/screenshots.location: 'ghp_notARealToken' is not valid under any of the given schemas
PASS setting-in-items.json: refused at items/0: 'dock.autohide' does not match '^[a-z0-9]+(?:-[a-z0-9]+)*$'
PASS shell-command-field.json: refused at (top level): Additional properties are not allowed ('postInstall' was unexpected)
PASS trust-grant.json: refused at packages: Additional properties are not allowed ('trusted' was unexpected)
PASS wrong-schema-version.json: refused at schemaVersion: 1 was expected

== edge cases: small Configs that must be refused or accepted
PASS refuses a schemaVersion written as a string
PASS refuses an empty name
PASS refuses a name with a terminal escape
PASS refuses a formula starting with a dash
PASS refuses a formula with a space
PASS refuses an uppercase formula
PASS refuses a second spelling of a core formula
PASS refuses a package with options
PASS refuses a version pin field
PASS refuses an official tap in taps
PASS refuses a tap written as a URL
PASS refuses a plural secret key
PASS refuses a nested object as a value
PASS refuses an email as a value
PASS refuses a home-folder path as a value
PASS refuses a private key as a value
PASS refuses a repo over ssh
PASS refuses a repo over http
PASS refuses a repo that climbs with ..
PASS refuses dotfiles with no repo
PASS refuses a link target using $HOME
PASS refuses a link target climbing out of home
PASS refuses a link source climbing out of the repo
PASS refuses a link source starting with a dash
PASS refuses an install script climbing out of the repo
PASS refuses an install script starting with a dash
PASS refuses an install script path with a space, which could be an argument
PASS accepts a formula with @ and +
PASS accepts a third-party cask
PASS accepts a tap repo name with a dot
PASS accepts a link target with spaces
PASS accepts a hidden link source
PASS accepts a repo URL ending in .git
PASS accepts a setting holding a dotted version string
PASS accepts a Config with nothing but its version and name

== resolving against the Catalog stub
developer.json: 7 formulae, 7 casks, 1 recipes, 5 settings, 4 links
  separate approval: taps ['anomalyco/tap'], install script https://git.example.com/dev/dotfiles: script/setup
  local values asked for: ['git.user.email', 'git.user.name']
lifecycle.json: 2 formulae, 0 casks, 0 recipes, 3 settings, 0 links
  separate approval: taps [], install script None
  local values asked for: []
  note: deprecated Item example-deprecated-tool: still installed, replacement offered: ghostty
  note: unknown Item example-from-a-newer-catalog: skipped and kept in the file
  note: renamed Item example-old-name -> gh: resolved, rewritten at the next reviewed write
  note: retired Item example-retired-tool: skipped and kept, nothing uninstalled. The vendor shut this down. Replacement offered: ghostty
  note: bad value for dock.autohide: expected a boolean: skipped and kept in the file
  note: unknown setting example.from-a-newer-catalog: skipped and kept in the file
minimal.json: 0 formulae, 1 casks, 0 recipes, 0 settings, 0 links
  separate approval: taps [], install script None
  local values asked for: []
owner-personal.json: 16 formulae, 5 casks, 3 recipes, 33 settings, 7 links
  separate approval: taps ['jetbrains/junie', 'jetbrains/utils', 'mobile-dev-inc/tap', 'nikitabobko/tap'], install script None
  local values asked for: ['git.user.email', 'git.user.name']
owner-work.json: 16 formulae, 5 casks, 3 recipes, 33 settings, 7 links
  separate approval: taps ['jetbrains/junie', 'jetbrains/utils', 'mobile-dev-inc/tap', 'nikitabobko/tap'], install script None
  local values asked for: ['git.user.email', 'git.user.name']

== Brewfile coverage: every entry must resolve from the translated Configs
personal: 25 entries, missing none
work: 25 entries, missing none
scripts/kotlin.sh: Item kotlin in both Configs: True
scripts/kotlin-lsp.sh: Item kotlin-lsp in both Configs: True
scripts/android.sh: Item android-sdk in both Configs: True

== Adopt: lines changed when the app rewrites the file canonically
add a cask in the middle of its list: 1 changed lines
    +       "firefox",
add a cask at the end of its list: 3 changed lines
    -       "linearmouse"
    +       "linearmouse",
    +       "zed"
change one setting: 2 changed lines
    -     "dock.autohide": true,
    +     "dock.autohide": false,
add one setting: 1 changed lines
    +     "dock.tile-size": 48,
remove one Item: 1 changed lines
    -     "junie",
add one link: 1 changed lines
    +       "~/.config/jj": "jj",

result: all checks passed
```

</details>

Ajv's output, using ECMA-262 regular expressions. It shows 14 invalid files, not 15, because `duplicate-key.json` is a test of the parser rather than the schema:

```text
examples: 5/5 behaved as expected (accepted)
examples/invalid: 14/14 behaved as expected (refused)
"ripgrep\n" as a formula: refused
```

**Package name patterns against real data.** One-off, run with Homebrew's published lists from 2026-11-06, [`formula.json`](https://formulae.brew.sh/api/formula.json) and [`cask.json`](https://formulae.brew.sh/api/cask.json), testing every name against `formulaName` and `caskName`:

```text
formula names: 8704, refused: 0
cask names: 7816, refused: 0
```

### Check script

<details>
<summary><code>check.py</code></summary>

```python
#!/usr/bin/env python3
"""PROTOTYPE: validates the Config examples, resolves them against the Catalog stub and measures Adopt diffs."""
import difflib, json, pathlib, re, sys
from jsonschema import Draft202012Validator

root = pathlib.Path(sys.argv[1]).resolve()  # docs/prototypes/config-format
repo = root.parents[2]
schema = json.loads((root / "config.schema.json").read_text())
Draft202012Validator.check_schema(schema)
validator = Draft202012Validator(schema)
catalog = json.loads((root / "catalog-stub.json").read_text())["items"]
print("schema: valid against the 2020-12 metaschema")

def no_dupes(pairs):
    keys = [k for k, _ in pairs]
    dupes = {k for k in keys if keys.count(k) > 1}
    if dupes:
        raise ValueError(f"duplicate key {sorted(dupes)}")
    return dict(pairs)

def strict_load(path):
    return json.loads(path.read_text(), object_pairs_hook=no_dupes)

ORDER = ["$schema", "schemaVersion", "name", "items", "packages", "settings", "dotfiles"]
def canonical(c):
    out = {}
    for k in ORDER:
        if k not in c:
            continue
        v = c[k]
        if k == "items":
            v = sorted(v)
        elif k == "packages":
            v = {s: sorted(v[s]) for s in ("formulae", "casks", "taps") if v.get(s)}
        elif k == "settings":
            v = dict(sorted(v.items()))
        elif k == "dotfiles":
            v = {f: (dict(sorted(v[f].items())) if f == "links" else v[f])
                 for f in ("repo", "links", "installScript") if f in v}
        if v != [] and v != {}:
            out[k] = v
    return json.dumps(out, indent=2, ensure_ascii=False) + "\n"

def tap_of(name):
    return "/".join(name.split("/")[:2]) if name.count("/") == 2 else None

def follow(entry_id):
    seen = [entry_id]
    while "renamedTo" in catalog.get(seen[-1], {}):
        seen.append(catalog[seen[-1]]["renamedTo"])
    return seen[-1], len(seen) > 1

def value_error(entry, value):
    kind = entry["type"]
    values = value if kind == "string-list" else [value]
    if kind == "boolean" and not isinstance(value, bool):
        return "expected a boolean"
    if kind == "integer" and (isinstance(value, bool) or not isinstance(value, int)):
        return "expected an integer"
    if kind in ("string", "string-list"):
        if kind == "string-list" and not isinstance(value, list):
            return "expected a list"
        for v in values:
            if not isinstance(v, str):
                return "expected a string"
            if "values" in entry and v not in entry["values"]:
                return f"{v!r} isn't an allowed value"
            if "pattern" in entry and not re.search(entry["pattern"], v):
                return f"{v!r} doesn't match"
    return None

def resolve(c):
    """Pure function of Config and Catalog. It reads nothing from the Machine or the network."""
    r = dict(formulae=set(), casks=set(), recipes=set(), localValues=set(), needsYou=[], taps={}, notes=[])
    def want(kind, name, via):
        r[kind].add(name)
        tap = tap_of(name)
        if tap:
            r["taps"].setdefault(tap, []).append(via)
    for raw in c.get("items", []):
        entry_id, renamed = follow(raw)
        e = catalog.get(entry_id)
        if e is None:
            r["notes"].append(f"unknown Item {raw}: skipped and kept in the file")
            continue
        if renamed:
            r["notes"].append(f"renamed Item {raw} -> {entry_id}: resolved, rewritten at the next reviewed write")
        if "retired" in e:
            r["notes"].append(f"retired Item {raw}: skipped and kept, nothing uninstalled. {e['retired']['message']} Replacement offered: {e['retired']['replacedBy']}")
            continue
        if "deprecated" in e:
            r["notes"].append(f"deprecated Item {raw}: still installed, replacement offered: {e['deprecated']['replacedBy']}")
        install = e.get("install", {})
        if "formula" in install:
            want("formulae", install["formula"], f"Item {entry_id}")
        if "cask" in install:
            want("casks", install["cask"], f"Item {entry_id}")
        if "recipe" in install:
            r["recipes"].add(install["recipe"])
        r["localValues"].update(e.get("localValues", []))
        r["needsYou"].extend(e.get("needsYou", []))
    pk = c.get("packages", {})
    for kind in ("formulae", "casks"):
        for name in pk.get(kind, []):
            if name in r[kind]:
                r["notes"].append(f"{name} is both an Item and a package: installed once, written as the Item")
            want(kind, name, f"package {name}")
    for tap in pk.get("taps", []):
        r["taps"].setdefault(tap, []).append("listed in packages.taps")
    for raw, value in c.get("settings", {}).items():
        entry_id, _ = follow(raw)
        e = catalog.get(entry_id)
        if e is None:
            r["notes"].append(f"unknown setting {raw}: skipped and kept in the file")
        elif e.get("kind") != "setting":
            r["notes"].append(f"{raw} isn't a setting: skipped and kept in the file")
        else:
            err = value_error(e, value)
            if err:
                r["notes"].append(f"bad value for {raw}: {err}: skipped and kept in the file")
    d = c.get("dotfiles", {})
    r["script"] = f"{d['repo']}: {d['installScript']}" if "installScript" in d else None
    return r

failures = 0
print("\n== valid examples: must parse strictly, pass the schema and already be in canonical form")
valid = sorted(root.glob("examples/*.json"))
loaded = {}
for path in valid:
    c = strict_load(path)
    errors = list(validator.iter_errors(c))
    canon = canonical(c) == path.read_text()
    ok = not errors and canon
    failures += not ok
    loaded[path.name] = c
    print(f"{'PASS' if ok else 'FAIL'} {path.name}  ({len(path.read_text().splitlines())} lines, canonical: {canon})")
    for e in errors:
        print("     ", e.message)

print("\n== invalid examples: each must be refused")
for path in sorted(root.glob("examples/invalid/*.json")):
    try:
        c = strict_load(path)
    except ValueError as e:
        print(f"PASS {path.name}: refused by the strict parser: {e}")
        continue
    errors = list(validator.iter_errors(c))
    failures += not errors
    print(f"{'PASS' if errors else 'FAIL (accepted)'} {path.name}: refused at {'/'.join(map(str, errors[0].absolute_path)) or '(top level)' if errors else ''}: {errors[0].message[:110] if errors else ''}")

print("\n== edge cases: small Configs that must be refused or accepted")
def probe(want_ok, label, **fields):
    global failures
    ok = validator.is_valid({"schemaVersion": 1, "name": "probe", **fields})
    failures += ok != want_ok
    print(f"{'PASS' if ok == want_ok else 'FAIL'} {'accepts' if want_ok else 'refuses'} {label}")
def pkg(name): return {"packages": {"formulae": [name]}}
def df(**d): return {"dotfiles": {"repo": "example/dotfiles", **d}}
probe(False, "a schemaVersion written as a string", **{"schemaVersion": "1"})
probe(False, "an empty name", **{"name": ""})
probe(False, "a name with a terminal escape", **{"name": "ok\u001b[2Jsafe"})
probe(False, "a formula starting with a dash", **pkg("-x"))
probe(False, "a formula with a space", **pkg("rip grep"))
probe(False, "an uppercase formula", **pkg("RipGrep"))
probe(False, "a second spelling of a core formula", **pkg("homebrew/core/ripgrep"))
probe(False, "a package with options", **{"packages": {"formulae": [{"name": "ripgrep", "args": ["--HEAD"]}]}})
probe(False, "a version pin field", **{"packages": {"formulae": ["ripgrep"], "versions": {"ripgrep": "14"}}})
probe(False, "an official tap in taps", **{"packages": {"taps": ["homebrew/cask"]}})
probe(False, "a tap written as a URL", **{"packages": {"taps": ["https://example.com/tap.git"]}})
probe(False, "a plural secret key", **{"settings": {"github.tokens": ["placeholder"]}})
probe(False, "a nested object as a value", **{"settings": {"dock.autohide": {"enabled": True}}})
probe(False, "an email as a value", **{"settings": {"example.owner": "you@example.com"}})
probe(False, "a home-folder path as a value", **{"settings": {"finder.path": "file:///Users/someone/Documents/"}})
probe(False, "a private key as a value", **{"settings": {"example.key": "-----BEGIN OPENSSH PRIVATE KEY-----"}})
probe(False, "a repo over ssh", **{"dotfiles": {"repo": "git@github.com:example/dotfiles.git"}})
probe(False, "a repo over http", **{"dotfiles": {"repo": "http://github.com/example/dotfiles"}})
probe(False, "a repo that climbs with ..", **{"dotfiles": {"repo": "example/.."}})
probe(False, "dotfiles with no repo", **{"dotfiles": {"links": {"~/.zshrc": "zsh/zshrc"}}})
probe(False, "a link target using $HOME", **df(links={"$HOME/.zshrc": "zsh/zshrc"}))
probe(False, "a link target climbing out of home", **df(links={"~/a/../../etc/zshrc": "zsh/zshrc"}))
probe(False, "a link source climbing out of the repo", **df(links={"~/.zshrc": "zsh/../../secret"}))
probe(False, "a link source starting with a dash", **df(links={"~/.zshrc": "-x"}))
probe(False, "an install script climbing out of the repo", **df(installScript="../install.sh"))
probe(False, "an install script starting with a dash", **df(installScript="--upload-pack=x"))
probe(False, "an install script path with a space, which could be an argument", **df(installScript="install.sh --yes"))
probe(True, "a formula with @ and +", **{"packages": {"formulae": ["openssl@3", "bonnie++"]}})
probe(True, "a third-party cask", **{"packages": {"casks": ["nikitabobko/tap/aerospace"]}})
probe(True, "a tap repo name with a dot", **{"packages": {"taps": ["example/homebrew.tools"]}})
probe(True, "a link target with spaces", **df(links={"~/Library/Application Support/Code/User/settings.json": "vscode/settings.json"}))
probe(True, "a hidden link source", **df(links={"~/.zshrc": ".zshrc"}))
probe(True, "a repo URL ending in .git", **{"dotfiles": {"repo": "https://git.example.com/dev/dotfiles.git"}})
probe(True, "a setting holding a dotted version string", **{"settings": {"android.build-tools": "35.0.0"}})
probe(True, "a Config with nothing but its version and name")

print("\n== resolving against the Catalog stub")
for name, c in loaded.items():
    r = resolve(c)
    print(f"{name}: {len(r['formulae'])} formulae, {len(r['casks'])} casks, {len(r['recipes'])} recipes, {len(c.get('settings', {}))} settings, {len(c.get('dotfiles', {}).get('links', {}))} links")
    print(f"  separate approval: taps {sorted(r['taps'])}, install script {r['script']}")
    print(f"  local values asked for: {sorted(r['localValues'])}")
    for note in r["notes"]:
        print("  note:", note)

print("\n== Brewfile coverage: every entry must resolve from the translated Configs")
MOVED = {"azure-cli": "casks"}  # moved from formula to cask in homebrew-core on 2026-10-06
for role, name in (("personal", "owner-personal.json"), ("work", "owner-work.json")):
    r = resolve(loaded[name])
    missing, total = [], 0
    for file in ("common", role):
        for line in (repo / f"brew/Brewfile.{file}").read_text().splitlines():
            m = re.match(r'(tap|brew|cask) "([^"]+)"', line)
            if not m:
                continue
            total += 1
            kind, token = m.groups()
            if kind == "tap":
                found = token in r["taps"]
            else:
                bucket = MOVED.get(token) or ("formulae" if kind == "brew" else "casks")
                found = any(t == token or t.endswith("/" + token) for t in r[bucket])
            if not found:
                missing.append(line)
    failures += bool(missing)
    print(f"{role}: {total} entries, missing {missing or 'none'}")
for item, script in (("kotlin", "scripts/kotlin.sh"), ("kotlin-lsp", "scripts/kotlin-lsp.sh"), ("android-sdk", "scripts/android.sh")):
    present = all(item in loaded[n]["items"] for n in ("owner-personal.json", "owner-work.json"))
    failures += not present
    print(f"{script}: Item {item} in both Configs: {present}")

print("\n== Adopt: lines changed when the app rewrites the file canonically")
def measure(label, mutate):
    before = canonical(loaded["owner-work.json"])
    c = json.loads(before)
    mutate(c)
    after = canonical(c)
    diff = [l for l in difflib.ndiff(before.splitlines(), after.splitlines()) if l[:2] in ("+ ", "- ")]
    print(f"{label}: {len(diff)} changed lines")
    for l in diff:
        print("   ", l)
measure("add a cask in the middle of its list", lambda c: c["packages"]["casks"].append("firefox"))
measure("add a cask at the end of its list", lambda c: c["packages"]["casks"].append("zed"))
measure("change one setting", lambda c: c["settings"].update({"dock.autohide": False}))
measure("add one setting", lambda c: c["settings"].update({"dock.tile-size": 48}))
measure("remove one Item", lambda c: c["items"].remove("junie"))
measure("add one link", lambda c: c["dotfiles"]["links"].update({"~/.config/jj": "jj"}))

print(f"\nresult: {'FAILED, ' + str(failures) + ' problem(s)' if failures else 'all checks passed'}")
sys.exit(1 if failures else 0)
```

</details>

<details>
<summary><code>check-ajv.mjs</code></summary>

```js
// PROTOTYPE: second validator, using ECMA-262 regular expressions as JSON Schema specifies.
import { createRequire } from "node:module";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
const require = createRequire(process.argv[2] + "/");
const Ajv2020 = require("ajv/dist/2020").default;
const root = process.argv[3];
const schema = JSON.parse(readFileSync(join(root, "config.schema.json"), "utf8"));
const validate = new Ajv2020({ strict: true, allErrors: true }).compile(schema);
let failures = 0;
for (const dir of ["examples", "examples/invalid"]) {
  const wantValid = dir === "examples";
  let pass = 0, total = 0;
  for (const f of readdirSync(join(root, dir)).filter((n) => n.endsWith(".json"))) {
    if (f === "duplicate-key.json") continue; // the strict parser's job, and JSON.parse keeps the last value
    total++;
    const ok = validate(JSON.parse(readFileSync(join(root, dir, f), "utf8")));
    if (ok === wantValid) pass++; else { failures++; console.log("UNEXPECTED", dir, f, validate.errors?.[0]?.message); }
  }
  console.log(`${dir}: ${pass}/${total} behaved as expected (${wantValid ? "accepted" : "refused"})`);
}
const nl = validate({ schemaVersion: 1, name: "probe", packages: { formulae: ["ripgrep\n"] } });
console.log(`"ripgrep\\n" as a formula: ${nl ? "accepted" : "refused"}`);
process.exit(failures ? 1 : 0);
```

</details>

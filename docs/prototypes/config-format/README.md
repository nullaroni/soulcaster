# Config format (prototype)

**Throwaway prototype answering [#11](https://github.com/nullaroni/soulcaster/issues/11).** It isn't merged, and no app reads these files yet. The Catalog stub is illustrative only. The prototype skill's click-through HTML demo was skipped, because this question is about a file format and nobody is present to drive one: the schema, examples and check scripts are the prototype.

An [independent review](REVIEW.md) of the first draft found 8 blocking problems and 14 smaller ones. This version fixes them, and [Review findings](#review-findings) says where each one went and how far it's proven.

## Conclusion

A Config is one self-contained JSON file with a version, a name and up to four optional sections. Every entry is a string in a sorted list or one key in a map, so each change is about one line in Git. Items and packages are named, settings get typed values, and the Dotfiles Repo is a repo plus its links.

```json
{
  "$schema": "https://raw.githubusercontent.com/nullaroni/soulcaster/main/schema/config-1.schema.json",
  "schemaVersion": 1,
  "name": "Work Mac",
  "items": ["ghostty", "git-identity"],
  "packages": {
    "formulae": ["mobile-dev-inc/tap/maestro", "neovim"],
    "casks": ["azure-cli"]
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
| Item IDs | Lowercase product names (`ghostty`) and `<surface>.<name>` for settings (`dock.autohide`). The Catalog handles renames and retirement, and an ID is never reused. Old IDs resolve at read time forever. |
| Always installed | `gh`, Homebrew and Command Line Tools can't be selected or omitted. An entry naming one is ignored with a note. |
| Dotfiles Repo | `repo`, a `links` map from Machine location to repo path, and one optional script path. **A Dotfiles Repo is code**, so the whole block needs approval, tied to the repo and a commit. |
| Versioning | Integer `schemaVersion`, frozen once published. An older app refuses a newer Config and changes nothing. An unknown field rejects the whole file. An unknown ID, a value the Catalog refuses, or a lint finding skips only that entry and keeps it in the file. |
| Machine-local | Git identity, the Dotfiles Repo's clone location, approvals, the last-applied record and which Config this Machine follows. Local records may suppress an entry, as "unavailable here", but never add or change one. The `sudo` password is stored nowhere, and no field can hold it. |
| Removal | Not expressible. Dropping an entry only creates Drift, which a person resolves per Machine. |
| Includes | None. The common-plus-role split becomes two complete files. |
| Approvals | Worked out from the file and the Catalog alone, so Import can list them: third-party taps, the install script and the Dotfiles Repo. |

Validation passes on all three validators: Python's `jsonschema`, Ajv and Rust's `jsonschema` with plain `regex`. They give the same answer on 5 valid examples, 16 lint examples, the invalid examples, 118 shared cases and 40,000 fuzz cases. See [Validation](#validation). **No real Run, Adopt or Import happened, and nothing was written to a Machine.** Rules the schema can't express are tested as a small Python model, not an engine. [Unproven](#what-remains-unproven) lists what that leaves open.

## 1. Shape

**One line per entry.** Lists hold strings, and maps hold one value per key. Standard pretty-printers keep these on one line each, so a person's editor and the app produce the same layout.

**Items and packages are separate lists.** `items` holds Catalog IDs: apps, CLIs and Machine-value Items such as `git-identity`. `packages` holds Homebrew names that the Catalog doesn't cover. A package has no options, which rules out `args`, `postinstall`, `restart_service` and `trusted`. Any option belongs on a reviewed Catalog Item.

**Settings hold plain values.** They're booleans, numbers, strings or lists of strings, using readable values such as `"list"` rather than `"Nlsv"`. The Catalog holds the read and write path, the type, the bounds and the stored value, so a Config never names them. A home-folder path such as `NewWindowTargetPath` also stays out of the file: `finder.new-window-target` is just `"documents"`.

**Sections are optional.** The app omits empty ones.

**The app always writes the same form:** 2-space indent, final newline, fixed key order, and sorted `items`, `packages`, `settings` and `links`. A list that is a setting's value, such as `dock.apps`, keeps its order because the order means something. `$schema` is written when the app creates a file. It's kept as it is otherwise, and isn't added to a file that has none, so a reviewed write stays small. If a person's edits left the file in another form, the app commits the reformat separately first, so the Adopt commit stays small.

**Where Adopt writes.** Every change goes to the one file this Machine follows, and each kind of change has one destination:

| Change on the Machine | Destination |
|---|---|
| Package that a Catalog Item installs, by its `coversPackages` | `items` |
| Any other formula or cask | `packages.formulae` or `packages.casks`, as `owner/tap/name` if third-party |
| A setting the Catalog exposes | `settings` |
| Anything else | Not Adopted. Other apps show as Untracked. |

Adopt never writes `dotfiles`, because it isn't in the map's list of what Sync covers. Every Adopt result is validated by the schema, the Catalog and the lint stage **before it's committed**. A value with no valid spelling, such as a Dock tile outside the app folders, is marked not adoptable, so Adopt can't make a shared Config unusable.

**What a reviewed write changes.** The change itself, plus dropping entries that name an always-installed Item. It never rewrites renames, aliases or package-to-Item moves. Those wait for **Tidy**, an explicit separate action, for the reason in [section 2](#2-item-ids).

**Measured size of a change** in `owner-work.json` (102 lines), from the [check script](#validation):

| Change | Lines changed |
|---|---|
| Add a package mid-list, add a setting, add a link, remove an Item | 1 |
| Change a setting | 2 (one removed, one added) |
| Add at the end of a list | 3, because JSON needs a comma on the previous line |

The 3-line case can't be avoided in JSON. The other hand-editing cost is having no comments and no trailing commas. In return, the owner gets the format they chose, editor completion through `$schema`, and validation errors that name the exact path.

## 2. Item IDs

**Naming**

- **Items** are the product name in lowercase kebab-case: `ghostty`, `claude-code`, `git-identity`.
- **Settings** are `<surface>.<name>`, where the surface is what a person sees, not a `defaults` domain: `dock.autohide`, not `com.apple.dock.autohide`.
- **Booleans say what true means:** `dock.show-recents`, not `dock.hide-recents`.
- **Neither has a category or install method in it.** `tailscale` is the ID whether it arrives as the `tailscale-app` cask or something else. `tailscale` is also both an app and a CLI, so an `app.` or `cli.` prefix would force an early call that a later rename has to undo.

The grammars differ, so a setting ID in `items` fails the schema.

**Always-installed Items.** `gh`, `homebrew` and `command-line-tools` are marked `base` in the Catalog. A Config can neither select nor omit them, because a Run needs them whatever it says:

- An entry naming one, by Item ID or by a package name it covers, is **ignored with a note and dropped at the next reviewed write**. It doesn't reject the file, because this list can change between releases.
- They're excluded from Drift, Adopt, removal and Import's choices.
- A third-party package with the same name, such as `evil/tap/gh`, gets a warning and stays its own package.

There is no sign-in kind of Item. GitHub sign-in is a Run-level ask, Tailscale is install-only, and Xcode's Apple Account is a Needs-You step on `xcode`.

**Lifecycle**, all recorded in the Catalog:

| Catalog says | What the app does with a Config naming it |
|---|---|
| `renamedTo` | Treats it as the new ID at read time, forever, with no Drift. **The file isn't rewritten** until a person chooses Tidy. Only for the same kind and value type. Otherwise the old ID is retired and a new one added. |
| `deprecated` | Still installs it, and shows the message and replacement. |
| `retired` | Skips it, reports the message and offers a replacement. It uninstalls nothing, keeps the entry in the file, and doesn't switch to the replacement on its own. **The Catalog keeps its install and detection data forever,** so a leftover package isn't mistaken for something to Adopt. |
| Nothing (unknown ID) | Skips and reports that entry only, keeps it in the file, and carries on. This covers typos and IDs from a newer Catalog. A close match is suggested before the Run: `ghosty` shows "did you mean `ghostty`". |

An ID stays in the Catalog as a record forever and is never reused. Approvals follow what an ID resolves to, not the ID, so a rename that lands on a different tap asks again.

**Different Machines will run different Catalogs,** because the Catalog ships inside each release and updates are the user's choice. So:

- **While a Config holds an ID this app doesn't know, there's no "no longer in this Config" removal offer and no package Adopt.** The app says to update instead. The file might name that package through the ID it can't read.
- **Nothing rewrites automatically.** If one app rewrote `tool-old` to `tool-new`, or `ripgrep` from `packages` to `items`, the other would read that as a removal and rewrite it back, forever.
- **An ID shown as unknown at Import that later starts resolving** is treated as a new selection and reviewed again. It isn't installed just because the app updated.

**Item and package overlap.** A Catalog entry can declare two lists, using the same `formula:` and `cask:` spellings as record keys:

- **`coversPackages`** lists packages that mean this Item. A covered package in `packages` installs once, through the Item, and Tidy can move it. `junie` covers both `cask:junie` and `formula:jetbrains/junie/junie`.
- **`conflictsWith`** lists packages that can't sit beside it, such as `tailscale` with the `tailscale` formula, whose pre-install dialog can stall a Run. This is checked per entry before a Run. Neither side installs until a person picks one, and the rest carries on.

**Two spellings of one setting:** if `dock.old-name` renames to `dock.new-name` and a file has both with different values, both are skipped and reported. With the same value, it applies once. The Catalog lint refuses rename cycles, and the app also stops on one rather than hanging.

**A setting can have `requires`.** `dock.apps` needs `dockutil`, so selecting it pulls `dockutil` in, shown in the review with why, and any tap that brings still needs approval.

## 3. Dotfiles Repo

- **`repo`** is `owner/repo` on GitHub or an `https://` URL, with a port if needed. SSH isn't allowed because a bare Mac has no key yet. Credentials in the URL are refused.
- **`links`** maps a location under `~/` to a path inside the repo. Each link stays one line, where a list of `{source, target}` objects would take four. Non-ASCII letters, parentheses and `~` are allowed inside a segment, for real locations such as `com~apple~CloudDocs` and `Übersicht`. Shell metacharacters aren't.
- **`installScript`** is one path with no arguments and no spaces. A file is never run because of its name, so a repo's `install.sh` stays unused unless the Config names it. **This changes today's behaviour:** `scripts/dotfiles.sh` runs `install.sh` automatically, instead of linking.
- **Links and scripts can both be present.** Links are applied first, then the script runs if it's approved. A failing script doesn't stop the Run.
- **No clone location.** It defaults to `~/Developer/dotfiles`, as it does today, and can be overridden locally. Otherwise a stranger's Config could choose where a repo lands, such as `~/Library/LaunchAgents`.
- **No branch or commit pin in the file.** The pin is the approval, in the local record. See [section 8](#8-what-needs-approval). The app shows whether the repo is behind or has unpushed changes, as the map says.

**Link safety.** A map can't hold the same key twice, but that doesn't stop two links claiming one location. So:

| Refused | By | Why |
|---|---|---|
| `.` and `..` segments, `.git` segments in any case, spaces at a segment's edge, control and invisible characters, repo paths starting with `-` | Schema | Can't be right on any Machine |
| Names equal after case-folding and Unicode normalization, such as `~/.zshrc` with `~/.ZSHRC` | Lint, per link | This Mac's volume treats them as one file, so the second replaces the first (checked on this Mac) |
| A link inside another link | Lint, per link | The child would land inside the Dotfiles Repo |
| Anything at, above or inside the clone location | Lint, per link | It would replace the repo, or write into it |
| Protected folders: `~/Library`, `~/Documents`, `~/Desktop`, `~/Downloads`, `~/Applications`, `~/.ssh`, `~/.config`, `~/Library/LaunchAgents` and similar | Lint, per link | Backup-then-replace would move all of a user's data aside. It's a data-loss list, not a trust list, and files inside these folders can still be linked. |
| Repo symlinks that resolve outside the repo | Engine, tested in `check.py` with real symlinks | A link must end up at the file the preview showed |

A refused link is skipped and reported, and the rest of the Config carries on.

**A value a Catalog Item writes never goes through a link into the repo.** Git identity or a `git.*` setting written through a linked `~/.gitconfig` would land in the Dotfiles Repo, one push from publication, as the review reproduced and `check.py` does again. So the engine checks where the file really resolves first. If it's in the repo, that entry is reported as unavailable here and not written. The same goes for a linked `defaults` plist.

## 4. Versioning

| Case | What the app does |
|---|---|
| `schemaVersion` is newer than the app knows | Reads only that field, then refuses. No Run, Apply or Adopt, and the file isn't rewritten. Says to update Soulcaster. |
| `schemaVersion` is older | Reads it through an in-memory upgrade. The file keeps its version until someone uses a feature that needs a newer one, and that bump shows in the reviewed diff. |
| Missing or not an integer | Rejects the whole file. |
| Unknown field at the current version | Rejects the whole file and changes nothing. The error names the field. |
| Unknown Item or setting ID, a value the Catalog refuses, or a lint finding | Skips and reports that entry, keeps it in the file, and continues. |

**Any new field means a new `schemaVersion`.** Ignoring unknown fields would make a typo like `setings` silently unmanage everything, and would let a future field slip past Import's preview. Because old apps never rewrite a Config with fields they don't know, there's nothing to preserve at the field level. Preservation is still needed for entries: an older app must keep IDs from a newer Catalog when it writes.

**A schema version freezes once published.** Tightening a rule later would make a Config that's valid today refuse on every Machine. So rules that may change between releases stay out of the schema and in the lint stage below. This version's structure, grammar and size limits are in the schema, and nothing else.

The cost is that using a new field forces every Machine following that Config to update the app first. Writing the lowest version that can express the file keeps this rare. The Catalog has no version in the Config, because a Config refers to entries only by ID.

**The parser must be strict:** plain RFC 8259, a file of at most 1 MiB, duplicate keys refused at any depth, no `NaN` or `Infinity`, and no number too large for a double, so `1e999` is refused. Parsers disagree on which duplicate wins, so Import's preview could show a different value from what the engine uses. Stock `serde_json` accepts `{"schemaVersion":99,"schemaVersion":1}` and keeps 1, so the engine needs the small custom visitor in [`check/rust/src/main.rs`](check/rust/src/main.rs). A schema can't express any of this.

**Size limits:** 500 items, 1,000 each of formulae and casks, 500 settings, 500 links, 200 strings in a list value, and 512 characters for `repo`. The largest Config these allow validates in 0.12 s in Python and 0.007 s in Ajv. The real reason for them is that one hostile line can't be buried in a preview of half a million.

**Secret checks are a lint stage,** because a whole-file schema rule hit real values: the locale `sk-SK`, `screensaver.require-password` and `ssh.auth-sock`. It's best effort, and it can't be otherwise. The real guard is that there's no free-form field, values only reach typed Catalog settings, and every write is scanned.

| When | What a lint finding does |
|---|---|
| An app write, such as Adopt | **Hard stop**, for what that write adds or changes. Entries already in the file don't block it, so a name from a newer Catalog can't stop an unrelated Adopt. |
| Reading a Config | **Skips that entry**, reports it and keeps it in the file. The rest applies. |
| Import | The same as reading. The file is someone else's, and the preview shows the finding. |

The lint looks for token prefixes, JWTs and private keys in values, `name`, link paths and the repo, plus email addresses and `/Users/` paths in values and `name`, and secret-looking names on setting IDs the Catalog doesn't know. A Catalog setting named `screensaver.require-password` passes, because it's typed and reviewed.

**Text from a Config is untrusted.** `name` and setting strings refuse control, bidi and invisible characters. Even so, "Verified safe by Soulcaster" is a legal `name`, and an unknown setting's value is a 256-character string nobody reviewed. So the app always shows them as quoted data from the file's author, never as its own words. That's an engine rule, and it isn't tested here.

## 5. What stays on the Machine

**The boundary:** if Adopt could write it, it's in the Config. **Local records may suppress an entry but never add or change one,** and they fill slots the Catalog declares. So Drift only ever compares one file with the Machine.

| Local | Why it isn't in the Config | What the format provides |
|---|---|---|
| Git identity and other Catalog-declared values | Personal, and committed Configs are often public | The `git-identity` Item says identity is wanted, and the Catalog declares the value names. Leaving the Item out means the app doesn't touch Git identity, so it's never required. Values are filled **without a third up-front ask**, in this order: existing `git config`, the local file, the GitHub account, then the Needs-You Queue. A Run never prompts for them. |
| Dotfiles Repo clone location | Differs per Machine, and is risky from a stranger | Link sources are relative to the repo root. |
| Which Config this Machine follows | One file, many Machines | A Config Repo plus a path. |
| Approvals | An author must never approve for an importer | Stable keys: `owner/tap`, `owner/tap/name` and, for the `dotfiles` block, the repo plus a commit SHA. No approval field exists, and `trusted`, `approved` and `commit` fail the schema. |
| Last-applied record, including prior setting values | Differs per Machine, and is what lets Drift show which side changed | One key for every entry: Item ID after following renames, `formula:<name>` after resolving aliases, `cask:<name>`, setting ID, or link location. A package that gains an Item migrates to the Item's key at read time, so it doesn't read as dropped. |
| Unavailable here, per entry with a reason | This Machine can't apply it, and others can | See below. |
| The `sudo` password | Held in memory for part of a Run | Nothing. No field holds it, and it's never written to disk. |

**Unavailable here.** Some entries can't apply on some Machines: `Launchpad.app` isn't on macOS 27, a setting may be managed by an admin, and a Machine may have no Apple Account for `xcode`. Without a state for this, Apply can never clear the Drift, and Adopt would delete the entry for every Machine. So the last-applied record holds a per-entry **unavailable here** result with a reason. It's set by the engine, or by a person from the queue.

- It's shown apart from Drift, with no Apply or Adopt.
- **List settings compare only on entries available here.** A missing Dock tile doesn't make the list differ, and Adopt keeps that tile in the file for other Machines, in place.

The Item lifecycle maps this onto its `skipped` state. The full design of these records is in that draft's section 7, and [#13](https://github.com/nullaroni/soulcaster/issues/13) and [#15](https://github.com/nullaroni/soulcaster/issues/15) own the behaviour around them.

**Package names have one written form, not one identity.** The schema refuses `homebrew/core/x`, `owner/homebrew-tap/x` and uppercase. It can't refuse aliases: 281 aliases on 250 formulae in live Homebrew data are valid names, and `nvim` means `neovim`. So the engine resolves aliases from Homebrew's metadata at read time, and Tidy can rewrite them.

## 6. Removal

A Config can't express removal. It has no `remove`, `absent`, `cleanup`, `exact` or `force`, and `null` is refused. Deleting a line is the only way to change what's listed, and it has one effect:

| Situation | Result |
|---|---|
| Never in the Config, never applied | Nothing. Other apps show as Untracked. |
| Dropped from the Config, was applied here, still on the Machine | Drift: "no longer in this Config". The person chooses to keep it and stop tracking, remove it from this Machine with explicit confirmation, or add it back. Until they choose, nothing happens. **Not offered while the file holds an ID this app doesn't know, or for an always-installed Item.** |
| Removed on the Machine, still in the Config | Drift. Apply reinstalls it, or Adopt deletes the line, unless it's a link, which only Apply can fix. |
| Setting dropped from the Config | The value stays as it is. The app can offer the recorded prior value. |
| Link dropped from the Config | The link stays until a person confirms. |
| This Machine switches to another Config | Same as dropped, entry by entry. Nothing is removed. |

Other Machines learn of a removal the same way, by comparing the file with their own last-applied record. No tombstone is needed, so a shared Config can't trigger an uninstall. A Machine with no record of the entry sees nothing.

**One related case to show clearly in Import's preview:** a list-valued setting is a whole value. `dock.apps` replaces the Dock's contents, as `dockutil --remove all` does today, so a tile missing from that list leaves the Dock. The app itself stays installed.

## 7. No includes

A Machine follows one file, and a Config doesn't reference other Configs.

With `extends`, Adopt has no single right answer. A new package could go in the role file or the base. A package removed from a Machine that the base lists would need an override meaning "not this one", which is the removal field that section 6 rules out. Import would also have to fetch and preview several files, any of which could change after approval. [chezmoi's notes](https://github.com/twpayne/chezmoi/blob/745ad561962c3b47742e715f13bd2ff51c06e7dd/assets/chezmoi.io/docs/user-guide/frequently-asked-questions/design.md?plain=1#L192) and [Homebrew's on layered Brewfiles](https://github.com/Homebrew/brew/blob/88f401fb862a12a0a5a201f189de570efc2a9ff1/docs/Homebrew-for-Teams.md?plain=1#L132) point the same way.

So today's common-plus-role split becomes two complete files. `owner-personal.json` and `owner-work.json` differ by 7 lines out of about 105. A change meant for both must be made in both, which is the real cost. If that hurts in practice, the fix belongs in the app, such as "copy this change to my other Config", and leaves the format alone.

**Config Repo layout:** Configs live in `configs/*.json`. Importing `owner/repo` lists those files, and importing a URL can name any one file.

## 8. What needs approval

Import can compute all of it without running anything. There are three separate approvals:

| Approval | What it covers | Starts |
|---|---|---|
| **Taps** | `owner/tap` from third-party names in `packages`, and the taps of Catalog Items, including ones pulled in by `requires`. `aerospace` comes from a reviewed Catalog entry but still loads `nikitabobko/tap`, and tap code runs on the Machine whoever suggested it. | Off |
| **Install script** | `dotfiles.installScript`, shown in full | Off |
| **The Dotfiles Repo** | The whole `dotfiles` block: the repo and every link. | **Off, unless the repo belongs to the signed-in GitHub account** |

**Why the Dotfiles Repo needs its own.** A Dotfiles Repo is code. With a sandboxed `HOME`, `check.py` shows a linked `~/.gitconfig` running a program on a plain `git status`, and silently redirecting a clone, and a linked `~/.zshenv` running on any `zsh -c`. No script is involved, and a repo's contents can change without the Config changing. A hostile Config with no taps and no script would otherwise have nothing to approve. ADR 0001's decision stands, because a Config still carries no commands, but its Consequences need the amendment proposed with this work.

**The approval is the repo plus a commit SHA,** in the local record. This replaces hashing the script, which missed anything the script sources.

- The engine never pulls past that commit without a new preview, which shows what changed since. A person's own `git pull` isn't the engine's to stop, so links follow what's on disk, and the script won't run until its commit is approved.
- An own repo is matched by its owner, case-insensitively, with GitHub's own answer for where it lives now, so a transferred repo doesn't still count. A URL on another host is never "own".

**The engine's guards, for whatever is already on the Machine:**

- **Link only after the `sudo` helper is gone,** in the User Stage.
- **The engine's git runs with `GIT_CONFIG_GLOBAL=/dev/null` and `GIT_CONFIG_NOSYSTEM=1`, and its shells with `zsh -f`.** `check.py` shows these stopping the marker. It reads and writes the user's Git config with `git config --file`, which doesn't run anything in it.

Third-party names must be fully qualified, so a bare name always means official Homebrew. The engine has to enforce that, because Homebrew may also find a bare name in a tap already on the Machine (not tested). The check script prints these lists for each example. How the approvals look is [#15](https://github.com/nullaroni/soulcaster/issues/15). The same approvals also apply when a later Adopt on another Machine brings in a new tap, not just at Import.

I deliberately left out a blocklist of dangerous link targets. `~/.zshrc` is both the most common link and a way to run code, so a list can't separate them. The approval does that job.

## Translating the owner's setup

[`owner-personal.json`](examples/owner-personal.json) and [`owner-work.json`](examples/owner-work.json) cover every Brewfile entry, checked by script against the Brewfiles.

| Today | In the Configs |
|---|---|
| `Brewfile.*` | Catalog-covered ones in `items`, the rest in `packages` |
| `brew "gh"` | Nothing. It's always installed. |
| `kotlin.sh`, `kotlin-lsp.sh` | Items `kotlin` and `kotlin-lsp`. `kotlin` uses the official formula in place of `curl \| sh`, and `kotlin-lsp` uses the Item lifecycle's pinned download, as ADR 0001 requires. |
| `android.sh` | Items `android-sdk` and `android-emulator`. The Catalog pins the platform and build tools, because going from 35 to 36 leaves both installed, so one value can't describe the Machine. `android.emulator: false` would also have been removal by another name. |
| `dock.sh`, `finder.sh`, `keyboard.sh`, `macos.sh`, `menu-bar.sh` | `settings`. Eight hot-corner keys become four settings, because each corner and its modifier are written together. `dock.apps` pulls in `dockutil`, listed as an Item here. |
| `dotfiles.sh` | `dotfiles`, with a placeholder repo |
| `git.sh`, except identity | `git.*` settings. Identity becomes the `git-identity` Item and local values. |

`git.*` stays even though it goes past the map's "macOS settings" wording. It's typed and readable, its keys are a closed list with none that run programs, and it follows the write-through rule. The map should say "settings the Catalog exposes".

**Not in a Config, on purpose:**

| Today | Where it goes |
|---|---|
| Homebrew, Command Line Tools, `gh` | Always installed, per the map |
| `docs/manual-steps.md`, `gh auth login` | The Needs-You Queue, from Catalog recipes. GitHub sign-in is a Run-level ask. |
| `paths.sh` | Item recipes or the Dotfiles Repo |
| Restarts of Dock and Finder, offer to restart the Mac | Catalog effects and app behaviour |
| Weather's menu bar position (`100`), `yes \| sdkmanager --licenses` | Catalog recipe. Licenses are a queue step, because a Config shouldn't accept them for someone. |
| `mkdir ~/Developer` | Part of the clone and of the sidebar favorites recipe |

**Problems found in the current setup.** These were kept as they are, except where noted, because a translation shouldn't quietly fix them:

- **`azure-cli` is no longer a formula.** homebrew-core [moved it to a cask](https://github.com/Homebrew/homebrew-core/blob/main/tap_migrations.json) on 2026-10-06, and `Brewfile.work` still says `brew`. It's listed under `casks`. Confirmed in live data. I didn't run `brew` to see how the old entry behaves.
- **`brew "junie"` means `jetbrains/junie/junie`,** which has no homebrew-core formula behind it. **Changed:** live data on 2026-11-06 shows an official `junie` cask at 3959.5, from JetBrains' own GitHub releases, while the tap formula is still at 3110.7 from the same release asset, wrapped in a shim that copies itself into `~/.local/bin`. The stub uses the cask, so `junie` needs no tap approval. Neither was installed.
- **`tap "jetbrains/utils"` looks unused.** Nothing in the Brewfiles comes from it. It does hold `kotlin-lsp.rb`, which the Brewfile's comment says is installed another way. **Changed:** `packages.taps` is gone from version 1, because this was its only entry, Adopt couldn't write it, and it asked for approval with no purpose. A tap with no named package can return with a new `schemaVersion`.
- **Already known to be broken, from [the settings research](https://github.com/nullaroni/soulcaster/blob/research/macos-settings/docs/research/macos-settings.md):** `Launchpad.app` in `dock.apps`, now reported as unavailable here. `finder.sidebar-favorites` and most of `menu-bar.extras` are marked `broken`, so they're skipped. Hot corners write `0` today, where nix-darwin documents `1`. **Changed:** `none` encodes to `1` in the stub. The Weather and Spotlight keys are unconfirmed.
- **The owner's real Dotfiles Repo wasn't available.** If it has an `install.sh`, today's script runs that and skips the links. The Config would then need `"installScript": "install.sh"`, which I couldn't check.

## Left out deliberately

| Left out | Why |
|---|---|
| Includes, conditionals, templates | Section 7. Each is a way for the preview to be wrong. |
| Version pins | Homebrew offers no real lock. A pin an Item needs, as `kotlin-lsp` does, lives in the Catalog. |
| Raw `defaults` writes | Untyped and can't be checked. A shared Config could set something sensitive. Propose a Catalog entry instead. |
| Mac App Store apps as packages | They need an account and the Needs-You Queue, so they're Catalog Items, as `xcode` is. |
| `packages.taps` and custom tap URLs | No current user. Can arrive with a new `schemaVersion`. |
| `android.*` settings | Install parameters, not settings. Section "Translating". |
| `description`, `notes`, comments | Prose belongs in the Config Repo's README. A stranger's text in the preview is also a spoofing risk. |
| `importedFrom` | See below. |
| Branch or commit pin in the file | Section 3. An author can't approve for an importer. |

## Rejected alternatives

| Alternative | Why not |
|---|---|
| One `items` list holding settings as `{id, value}` | Multi-line entries and larger diffs, with no gain. |
| A tombstone list such as `removed: [...]` | A shared file could then cause uninstalls, which is what [nix-darwin #1860](https://github.com/nix-darwin/nix-darwin/issues/1860) shows going wrong. |
| Tolerating unknown fields | Section 4. |
| Semver as `schemaVersion` | Only "can I read this?" matters, and an integer answers it. |
| Package options as objects | Reintroduces Brewfile behaviour and breaks one line per entry. |
| Secret checks in the schema | They refused real values, and a later prefix would break valid files. Section 4. |
| Rewriting old IDs at the next reviewed write | Two app versions rewrite each other forever. Section 2. |
| A blocklist of dangerous link targets | Section 8. |

**Where this departs from [the prior-art research](https://github.com/nullaroni/soulcaster/blob/research/prior-art/docs/research/prior-art.md):**

| It recommended | This format | Reason |
|---|---|---|
| `$schema` required | Optional and any string. The app writes it on new files and never fetches it. | It only helps editors, so a hand-written file, a local path or a renamed repo shouldn't refuse a file. |
| Namespaced IDs such as `app.firefox` | `firefox` | Section 2. The research's real concern, UI names in IDs, is still met, and the two grammars keep Items and settings apart. |
| `importedFrom` with a commit | Left out, kept in the Machine's local record | Import is a copy and doesn't subscribe, so nothing in the file uses it. An author could also fill it in to look official. The cost is that provenance stays on one Machine, and adding the field later needs a new `schemaVersion`. |
| `taps` only for custom URLs | No `taps` field | Section "Translating". |
| Preserve unrecognised data on write | Kept for entries, unnecessary for fields | Section 4. |

## Review findings

Each row says how far the fix is proven. **Schema** means Python, Ajv and Rust all ran it. **Lint** means a rule in `check.py` with tests, which the engine must port. **Modelled** means tested in `check.py`'s small model of Drift and Adopt, with no app. **Real** means real `git`, `zsh` or symlinks in a temp folder. **Engine** means stated here and untested.

| # | Finding | Fix | Proven by |
|---|---|---|---|
| 1 | Links run a stranger's code | Third approval on the whole block, repo plus commit in the local record, guards on git and zsh, link after the helper is gone | Approval defaults: Modelled. Guards: Real. No pull past the commit, link order: **Engine** |
| 2 | Link targets move user data or corrupt each other | Section 3's table | Schema, Lint, Real for repo symlinks. Backup behaviour: **Engine** |
| 3 | `gh` creates false Drift | Always-installed Items, section 2 | Modelled |
| 4 | Version skew | Old IDs resolve forever, Tidy is explicit, no removal or package Adopt while an ID is unknown | Modelled, including 6 rounds of two app versions |
| 5 | Adopt can break a shared file | Lint stage, frozen schema, every Adopt validated | Schema for what no longer refuses, Lint and Modelled for the rest |
| 6 | No state for "can't apply here" | Unavailable here, section 5 | Modelled. Detecting a missing tile: **Engine** |
| 7 | Git identity written into the repo | Write-through rule, section 3, removed from `developer.json` | Lint and Real. The engine's check before writing: **Engine** |
| 8 | Sign-in Items | Deleted, with Xcode's account kept as a Needs-You step | Catalog lint and Modelled |
| 9 | Lookahead in patterns | Moved to `not: {pattern}`, and all compile under plain `regex` | Schema, Rust |
| 10, 11 | Duplicate keys, `NaN`, `1e999` | Strict parsers | Python and Rust, 18 parser cases |
| 12, 13 | Item and package overlap, migration | `coversPackages`, `conflictsWith`, key migration | Lint and Modelled |
| 14 | Retired Items lose data | Detection data kept forever | Lint and Modelled |
| 15 | "One spelling" | `homebrew-` tap segments and `.rb` names refused, aliases resolved, claim corrected | Schema, Modelled with one real alias |
| 16 | Rename collision and cycle | Both skipped and reported, cycle linted and stopped | Lint and Modelled |
| 17 | Settings with no Item | `requires` on settings, `android.*` dropped | Modelled |
| 18 | `name` and unknown settings as a notes channel | Control, bidi and invisible characters refused. Showing them as data: **Engine** | Schema |
| 19 | No size limits | 1 MiB before parsing, plus schema caps | Schema, Rust and Python parsers |
| 20 | Stub values | Bounds and read paths, required by the Item lifecycle's schema too | Lint, plus that schema |
| 21 | `junie` cask | Checked live and adopted | Live data, not installed |
| 22 | README consistency | Section 6's row, `$schema`, empty `links` | `check.py`, and the README's example is now validated |

Judgment calls, all accepted: Git identity filled without a third ask, `packages.taps` dropped, `git.*` kept and `android.*` dropped, IDs that later resolve reviewed again, typo suggestions before a Run, a looser path character set, any string for `$schema`, and unprefixed IDs with no `importedFrom`.

## What remains unproven

1. **No engine exists.** Every "Lint", "Modelled" and "Engine" rule has to be built in Rust. The model is small and reads no real Machine, so it shows the rules hang together, not that they work on a Mac.
2. **Homebrew's own git calls.** With a linked `~/.gitconfig`, does `brew` read it during a later Run's Admin Stage, with `SUDO_ASKPASS` visible? Neither the review nor this work established it, because testing it would change Homebrew's tap folder. It's [spike 15](https://github.com/nullaroni/soulcaster/blob/prototype/item-lifecycle/docs/prototypes/item-lifecycle/README.md#required-spikes) in the Item lifecycle. This is the most important open question from finding 1.
3. **`GIT_CONFIG_NOSYSTEM=1`** was set in the tests, but no real system Git config was tried, because that needs `sudo`.
4. **The secret lint is best effort.** It has no way to spot a bare password, or a token in a format it doesn't know. An unknown setting is kept in the file, so a mistyped key could still hold a secret until someone notices it in the report.
5. **The lists in the stub are judgment calls:** the protected folders, the setting bounds (Apple doesn't publish valid values, so `key-repeat` 1 to 120 and `initial-key-repeat` 10 to 120 only have to include the owner's values and refuse the review's), the hot-corner codes, which come from nix-darwin's documentation, and every `stores` value. The recipes pass the lifecycle schema, but none has run.
6. **Package names have no lifecycle in the Catalog.** `azure-cli` moving from formula to cask is exactly this. Items absorb such moves, but a name in `packages` doesn't. The engine has to handle Homebrew's renames and tap migrations and show the fix as a reviewed change. That detection is unbuilt.
7. **Adopt has to separate wanted formulae from dependencies,** or every dependency looks like Drift. Homebrew's "installed on request" data should do it, but that wasn't tested.
8. **The always-installed list can change between releases.** If `foo` joined it, a newer app would drop `foo` at a reviewed write, and an older Machine that installed it would see "no longer in this Config". No removal happens without confirmation, and this should be rare, but the skew rule doesn't catch it.
9. **Setting IDs become public API at first release.** The names in the stub were picked for the examples. Review them before they ship.
10. **`$schema` points at a file that doesn't exist yet.** The schema has to be published on `main` at `schema/config-1.schema.json`. Until then, editors can't fetch it. The app is unaffected, and a stale value no longer refuses a file.
11. **Comparing `dock.apps` needs care.** Dock entries are nested dictionaries, so the `dock-tiles` store has to define how paths map to them.
12. **Repo symlinks are checked at one commit,** and the engine must recheck at each newly approved one.

## Validation

Run for real on 2026-11-06 with Python 3.9.6, `jsonschema` 4.25.1, Node 26.13.0, Ajv 8.17.1, Rust 1.99.1 and the crates `jsonschema` 0.60.2 and `regex` 1.13.1. All tooling stayed in a temp directory. Run these from the repo root:

```sh
set -o pipefail
TMP="$(mktemp -d)"
ROOT=docs/prototypes/config-format
git show origin/prototype/item-lifecycle:docs/prototypes/item-lifecycle/recipe.schema.json > "$TMP/recipe.schema.json"

python3 -m venv "$TMP/venv"
"$TMP/venv/bin/pip" install --quiet --no-cache-dir jsonschema==4.25.1
"$TMP/venv/bin/python" "$ROOT/check/check.py" "$ROOT" --recipe-schema "$TMP/recipe.schema.json" --corpus "$TMP/corpus.json" | tee "$TMP/python.txt"

npm_config_cache="$TMP/npm-cache" npm install --silent --no-audit --no-fund --prefix "$TMP/ajv" ajv@8.17.1
node "$ROOT/check/check-ajv.mjs" "$TMP/ajv" "$ROOT" "$TMP/corpus.json" | tee "$TMP/ajv.txt"

cp -R "$ROOT/check/rust" "$TMP/rust"
(cd "$TMP/rust" && CARGO_HOME="$TMP/cargo" CARGO_TARGET_DIR="$TMP/target" PATH="$HOME/.cargo/bin:$PATH" cargo build --quiet --release)
"$TMP/target/release/config-check" "$ROOT" "$TMP/corpus.json" "$TMP/recipe.schema.json" | tee "$TMP/rust.txt"

# All three must give the same verdicts, so this must print 1.
grep -h '^fuzz: 40000' "$TMP/python.txt" "$TMP/ajv.txt" "$TMP/rust.txt" | sort -u | wc -l

rm -rf "$TMP"
```

The scripts are [`check/check.py`](check/check.py), [`check/check-ajv.mjs`](check/check-ajv.mjs) and [`check/rust/`](check/rust/), and [`conformance.json`](conformance.json) holds the shared cases. All three exit 0, and the digest check printed `1`. Last line of the check script's output: `result: all checks passed`.

**The three validators now agree, including where they used to disagree.** Python's `$` matches before a trailing newline, so it accepted `"ripgrep\n"` while the others refused. Every string now also has to pass `safeText`, which refuses control characters, so that difference is gone. Rust's plain `regex` compiles every pattern in this schema, the stub and the Item lifecycle's recipe schema. The app should pin the crate and run the shared cases in CI.

<details>
<summary>Full output of the check script</summary>

```text
schema: valid against the 2020-12 metaschema
PASS schema: none of its 14 patterns uses lookaround

== valid examples: must parse strictly, pass the schema and already be in canonical form
PASS developer.json  (44 lines, canonical: True)
PASS lifecycle.json  (18 lines, canonical: True)
PASS minimal.json  (7 lines, canonical: True)
PASS owner-personal.json  (105 lines, canonical: True)
PASS owner-work.json  (102 lines, canonical: True)
PASS README's example Config passes the schema

== invalid examples: each must be refused
PASS credential-in-repo-url.json: refused at dotfiles/repo
PASS dotfiles-clone-location.json: refused at dotfiles
PASS dotfiles-self-approval.json: refused at dotfiles
PASS duplicate-key.json: refused by the strict parser: duplicate key ['name']
PASS duplicate-schema-version.json: refused by the strict parser: duplicate key ['schemaVersion']
PASS include-another-config.json: refused at (top level)
PASS link-dot-git-segment.json: refused at dotfiles/links
PASS link-escapes-home.json: refused at dotfiles/links
PASS link-leading-space-segment.json: refused at dotfiles/links
PASS link-source-dot-git.json: refused at dotfiles/links/~/.zshrc
PASS link-trailing-space.json: refused at dotfiles/links
PASS name-with-bidi-override.json: refused at name
PASS newline-in-package.json: refused at packages/formulae/0
PASS null-setting.json: refused at settings/dock.autohide
PASS number-infinity.json: refused by the strict parser: Infinity isn't JSON
PASS number-nan.json: refused by the strict parser: NaN isn't JSON
PASS number-overflow.json: refused by the strict parser: number out of range: 1e999
PASS package-injection.json: refused at packages/formulae/0
PASS package-shaped-like-ruby-file.json: refused at packages/formulae/0
PASS packages-taps.json: refused at packages
PASS removal-field.json: refused at (top level)
PASS script-in-dot-git.json: refused at dotfiles/installScript
PASS script-with-arguments.json: refused at dotfiles/installScript
PASS setting-in-items.json: refused at items/0
PASS shell-command-field.json: refused at (top level)
PASS tap-with-homebrew-prefix.json: refused at packages/formulae/0
PASS trust-grant.json: refused at packages
PASS unavailable-in-the-file.json: refused at (top level)
PASS value-with-zero-width.json: refused at settings/example.note
PASS wrong-schema-version.json: refused at schemaVersion

== conformance: shared with Ajv and Rust
PASS 118 schema and size cases behaved as expected
PASS 18 parser cases behaved as expected
fuzz: 40000 cases, 16855 accepted, sha256 303dffa59c1cceeef7c4dcdba1e2589e5f221ccfa5d3ce0e9df7d90e3f269923

== lint examples: pass the schema, then the rules flag exactly these
PASS 16 lint examples, all with expected results
PASS base-items-listed.json: resolve notes only
PASS email-value.json: email-value
PASS git-identity-in-settings.json: machine-value
PASS home-folder-value.json: home-path-value
PASS item-package-conflict.json: resolve notes only
PASS link-ancestor-pair.json: link-overlap
PASS link-at-clone-location.json: link-clone, link-clone, link-clone, link-overlap, link-overlap, link-overlap
PASS link-case-duplicate.json: link-duplicate
PASS link-protected-folder.json: link-protected, link-protected
PASS link-unicode-duplicate.json: link-duplicate
PASS rename-collision.json: resolve notes only
PASS secret-shaped-key.json: secret-key
PASS secret-shaped-value.json: secret-value
PASS sign-in-item.json: resolve notes only
PASS token-in-name.json: secret-value
PASS write-through-link.json: writes-through-link, writes-through-link

== Catalog stub: lint and mutations
PASS 54 entries pass Catalog lint
PASS 52 recipes pass the Item lifecycle's recipe schema, without the five fields this draft owns
PASS refuses a rename cycle: loop-a: rename cycle
PASS refuses a rename to nowhere: gone: renamedTo names an unknown ID
PASS refuses a rename from an Item to a setting: was-item: a rename can't change an Item into a setting
PASS refuses a fourth base Item: base Items must be exactly Command Line Tools, Homebrew and gh, got ['command-line-tools', 'gh', 'ghostty', 'homebrew']
PASS refuses a sign-in kind: gh-sign-in: there is no sign-in kind
PASS refuses an integer setting with no bounds: keyboard.key-repeat: an integer setting needs bounds
PASS refuses a setting with no read path: dock.autohide: a setting needs a read path
PASS refuses a pattern with lookahead: git.default-branch: a pattern can't use lookaround
PASS refuses a retired Item with no detection data: example-retired-tool: a retired Item keeps its detection data
PASS refuses a package covered by two Items: formula:opencode is covered by more than one Item: ['ghostty', 'opencode']
PASS refuses a step's package missing from coversPackages: ghostty: coversPackages must list cask:ghostty, which its steps install
PASS refuses a badly spelled conflict: tailscale: conflictsWith entry tailscale needs formula: or cask:
PASS refuses a replacement that doesn't exist: example-retired-tool: retired.replacedBy names an unknown ID
PASS refuses a requirement that doesn't exist: dock.apps: unknown requirement not-there

== rules the schema can't express, modelled
PASS base: minimal.json doesn't want gh, and gh on the Machine is no Drift
PASS base: dropping gh from a Config offers no removal
PASS base: gh is never offered to Adopt
PASS base: entries naming gh are ignored with a note
PASS base: they're dropped at the next reviewed write
PASS base: a third-party gh warns and is still its own package
PASS base: Import shows no choice for them
PASS sign-in: the kind and both IDs are gone
PASS sign-in: Xcode's Apple Account stays a Needs-You step
PASS sign-in: Tailscale is install-only, with a note and no queue step
PASS skew: an older app reading a renamed ID offers no removal and no Adopt, only an update
PASS skew: a newer app reading the old ID sees no Drift
PASS skew: a reviewed write doesn't rewrite the old ID
PASS skew: rewriting is the explicit Tidy action
PASS skew: a package gaining an Item no longer rewrites back and forth (6 rounds, 1 form)
PASS skew: the record key migrates, so it doesn't read as dropped
PASS skew: an older app holding the new Item offers no package Adopt
PASS skew: an unknown ID is skipped and kept
PASS skew: an ID unknown at Import that later resolves is held for review, and not installed
PASS typos: a suggestion appears before a Run
PASS lint: sk-SK and Catalog settings with secret-looking names pass
PASS lint: flags 'ghp_abcdefghijklmnop'
PASS lint: flags ' ghp_abcdefghijklmno'
PASS lint: flags 'Bearer abcdefghijklm'
PASS lint: flags 'sk_live_abcdefghijkl'
PASS lint: flags 'npm_abcdefghijkl'
PASS lint: flags 'hf_abcdefghijkl'
PASS lint: flags 'eyJhbGciOiJIUzI1.eyJ'
PASS lint: flags '-----BEGIN OPENSSH P'
PASS lint: flags 'AKIAABCDEFGHIJKLMNOP'
PASS lint: flags a key named sudo.pass
PASS lint: flags a token in a link source
PASS lint: an app write adding a token is a hard stop
PASS lint: an unrelated write isn't blocked by what's already in the file
PASS lint: on read, the flagged entry is reported and the rest of the file still applies
PASS Adopt: a home-folder Dock tile isn't adoptable, so the file stays valid
PASS Adopt: the same tile written with ~/ is adoptable
PASS Adopt: a tile with no valid spelling isn't adoptable
PASS Adopt: an out-of-range value isn't adoptable
PASS Adopt: a normal value is adoptable
PASS unavailable: a missing Dock tile is reported apart from Drift, with no Adopt
PASS unavailable: Adopt on a list keeps the tile for other Machines
PASS unavailable: xcode on a Machine with no Apple Account shows no Drift and no Adopt
PASS unavailable: it lives only in the local record
PASS unavailable: 300 random records only ever suppress entries, and never add one
PASS unavailable: a broken setting isn't applied
PASS unavailable: a real hot corner now has a spelling
PASS write-through: developer.json no longer combines git-identity with a linked ~/.gitconfig
PASS write-through: a linked defaults plist is reported too
PASS overlap: a covered package installs once, through the Item
PASS rename collision: the same value applies once
PASS rename cycle: it terminates, skips the entry and reports it
PASS requires: dock.apps pulls in dockutil, shown with why
PASS requires: a tap pulled in by a setting still needs approval
PASS retired: skipped, and kept in the file
PASS retired: its leftover package isn't offered to Adopt
PASS retired: an entry in the file isn't read as dropped
PASS retired: a retired setting gets the right note
PASS alias: nvim and neovim are one package
PASS alias: nvim shows no Drift against neovim
PASS alias: only Tidy rewrites it
PASS stub: keyboard.key-repeat refuses -5
PASS stub: keyboard.initial-key-repeat refuses 1
PASS stub: git.excludes-file refuses '~/../../../etc/passwd'
PASS stub: the owner's real values still pass
PASS canonical: empty links are dropped
PASS canonical: the app doesn't add $schema to a file that has none
PASS Import: you/dotfiles with sign-in you starts on
PASS Import: You/dotfiles with sign-in you starts on
PASS Import: https://github.com/you/dotfiles with sign-in you starts on
PASS Import: stranger/dotfiles with sign-in you starts off
PASS Import: https://github.com.evil.example/you/dotfiles with sign-in you starts off
PASS Import: https://git.example.com/you/dotfiles with sign-in you starts off
PASS Import: you/dotfiles with sign-in None starts off
PASS Import: a hostile Dotfiles block needs approval, even with no taps or script
PASS Git identity: existing config, then the local file, then GitHub
PASS Git identity: a blank value goes to the queue, with no prompt

== real git and zsh, with a sandboxed HOME
PASS real git: a linked ~/.gitconfig runs core.fsmonitor on plain git status
PASS real git: GIT_CONFIG_GLOBAL=/dev/null and GIT_CONFIG_NOSYSTEM=1 stop it
PASS real git: a linked url.insteadOf silently redirects a clone
PASS real git: the guard keeps the URL the preview showed
PASS real zsh: a linked ~/.zshenv runs on zsh -c
PASS real zsh: zsh -f doesn't run it
PASS real git: writing a linked config changes the file in the repo
PASS writes-through-link: detects that link
PASS writes-through-link: ignores a plain file
PASS repo symlinks: only the ones that leave the repo are refused

== resolving against the Catalog stub
developer.json: 6 formulae, 7 casks, 1 recipes, 5 settings, 3 links
  separate approval: taps [], dotfiles https://git.example.com/dev/dotfiles with script script/setup
  local values filled: ['git.user.email', 'git.user.name']
lifecycle.json: 1 formulae, 1 casks, 0 recipes, 5 settings, 0 links
  separate approval: taps [], dotfiles None
  local values filled: []
  note: deprecated example-deprecated-tool: still installed, replacement offered: ghostty
  note: unknown ID example-from-a-newer-catalog: skipped and kept in the file.
  note: renamed example-old-name -> ghostty: resolved, and the file isn't rewritten
  note: retired example-retired-tool: skipped and kept, nothing uninstalled. The vendor shut this down. Replacement offered: ghostty
  note: bad value for dock.autohide: expected a boolean: skipped and kept in the file
  note: unknown ID example.from-a-newer-catalog: skipped and kept in the file.
  note: renamed example.old-setting -> dock.show-recents: resolved, and the file isn't rewritten
  note: retired example.retired-setting: skipped and kept, nothing uninstalled. macOS removed this. Replacement offered: dock.show-recents
minimal.json: 0 formulae, 1 casks, 0 recipes, 0 settings, 0 links
  separate approval: taps [], dotfiles None
  local values filled: []
owner-personal.json: 16 formulae, 7 casks, 4 recipes, 30 settings, 7 links
  separate approval: taps ['mobile-dev-inc/tap', 'nikitabobko/tap'], dotfiles your-github-username/dotfiles with script None
  local values filled: ['git.user.email', 'git.user.name']
  pulled in: {'openjdk': 'android-sdk'}
  note: finder.sidebar-favorites doesn't work on this macOS: unavailable here. Skipped: sfltool add-item no longer works on macOS 27.
  note: menu-bar.extras doesn't work on this macOS: unavailable here. Skipped: most of these paths no longer exist on macOS 27.
owner-work.json: 16 formulae, 7 casks, 4 recipes, 30 settings, 7 links
  separate approval: taps ['mobile-dev-inc/tap', 'nikitabobko/tap'], dotfiles your-github-username/dotfiles with script None
  local values filled: ['git.user.email', 'git.user.name']
  pulled in: {'openjdk': 'android-sdk'}
  note: finder.sidebar-favorites doesn't work on this macOS: unavailable here. Skipped: sfltool add-item no longer works on macOS 27.
  note: menu-bar.extras doesn't work on this macOS: unavailable here. Skipped: most of these paths no longer exist on macOS 27.

== Brewfile coverage: every entry must resolve from the translated Configs
PASS personal: 25 entries, missing none
PASS work: 25 entries, missing none
PASS scripts/kotlin.sh: Item kotlin in both Configs
PASS scripts/kotlin-lsp.sh: Item kotlin-lsp in both Configs
PASS scripts/android.sh: Item android-sdk in both Configs
PASS scripts/android.sh: Item android-emulator in both Configs
PASS no android.* settings remain

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

Ajv's output, using ECMA-262 regular expressions. It skips the 3 invalid examples only the parser can catch, because `JSON.parse` keeps the last duplicate and turns `1e999` into `Infinity`:

```text
examples: 5/5 behaved as expected (accepted)
examples/invalid: 27/27 behaved as expected (refused)
examples/lint: 16/16 behaved as expected (accepted)
conformance: 118/118 behaved as expected
parser cases: 18 skipped, because JSON.parse isn't the engine's parser
fuzz: 40000 cases, 16855 accepted, sha256 303dffa59c1cceeef7c4dcdba1e2589e5f221ccfa5d3ce0e9df7d90e3f269923
```

Rust's output, with the strict parser in front of the plain `regex` engine:

```text
plain regex: 14/14 patterns in config.schema.json compile
plain regex: 4/4 patterns in catalog-stub.json compile
plain regex: 20/20 patterns in recipe.schema.json compile
examples: 5/5 behaved as expected (accepted)
examples/invalid: 30/30 behaved as expected (refused)
examples/lint: 16/16 behaved as expected (accepted)
conformance: 118/118 behaved as expected
parser cases: 18/18 behaved as expected
duplicate schemaVersion: stock serde_json accepts, strict parser refuses
fuzz: 40000 cases, 16855 accepted, sha256 303dffa59c1cceeef7c4dcdba1e2589e5f221ccfa5d3ce0e9df7d90e3f269923
fuzz: plain regex and fancy-regex differ on 0 cases
```

**Package name patterns against real data.** One-off, with Homebrew's published lists from 2026-11-06, [`formula.json`](https://formulae.brew.sh/api/formula.json) and [`cask.json`](https://formulae.brew.sh/api/cask.json):

```text
formula names: 8704, refused: 0
aliases: 281 on 250 formulae, refused: 0
cask names: 7818, refused: 0
```

**Do these checks catch anything?** One-off: I made each change below to a temp copy and reran all three validators. Every one was caught.

```text
drop safeText from formulaName (Python's $ should diverge): caught by Python; fuzz digests differ
put a lookahead back (plain regex must refuse): caught by Python, Rust
allow .git segments: caught by Python, Ajv, Rust
remove the items cap: caught by Python, Ajv, Rust
put ~/.gitconfig back in developer.json (write-through + order): caught by Python
allow a removal offer for base Items: caught by Python
allow dropped offers while holding an unknown ID: caught by Python
allow package Adopt while holding an unknown ID: caught by Python
don't say to update the app: caught by Python
no lint on writes: caught by Python
compare whole lists: caught by Python
remove the git guard: caught by Python
compare link names exactly: caught by Python
loosen the key-repeat bound: caught by Python
```

**Names on this Mac's volume.** One-off, in a temp folder: a file created as NFC `Ü` is found under NFD `Ü`, and `.zshrc` is found as `.ZSHRC`. So `fold()` in `check.py` matches this volume. A case-sensitive volume would refuse a few links it could hold, which is the safe direction.

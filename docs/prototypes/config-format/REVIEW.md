Independent review of the Config format at commit `359f879`, saved verbatim apart from a removed local scratchpad path.

# Review of the Config format, commit `359f879`

## Verdict: accept with the listed fixes, no redesign

The file's shape holds up: `items`, `packages`, `settings`, `dotfiles`, no includes, and removal that can't be expressed. Eight blocking findings need fixing first. Most are rule, example, stub or ADR changes, and the rest are small schema pattern edits.

**How to read "reproduced":** no app exists, so nothing here ran a real Run, Adopt or Import.
- **Reproduced:** ran real validators, `git`, `zsh` and `brew`, or the designer's own `resolve()` and `canonical()`.
- **Modelled:** those same functions plus my reading of the README's Drift rules, so it's strong logic rather than observed app behaviour.

## Blocking findings, most severe first (all reproduced or modelled)

### 1. Links run a stranger's code with no approval
- **Location:** README section 8 and the "Taps and scripts" row, ADR 0001 Consequences.
- **Failure:** a Config with links to `~/.zshenv`, `~/.gitconfig`, `~/.ssh/authorized_keys`, a LaunchAgent and `~/.local/bin/sudo` passes all three validators. `resolve()` reports `taps [], script None`, so nothing asks for approval.
- **No shell needed:** with a sandboxed `HOME`, the linked files acted as soon as ordinary commands ran:
  - `git status` ran a linked `core.fsmonitor`.
  - `git clone` ran a `core.hooksPath` hook, with `SUDO_ASKPASS` visible to it.
  - `url.insteadOf` sent a clone of `https://github.com/acme/tap` to an attacker's repo, while `origin` still showed the GitHub URL.
  - `zsh -c true` ran a linked `.zshenv`, also with `SUDO_ASKPASS` visible.
- **Script approval gap:** the approval hashes one file, so a helper the script sources can change without asking again. This is from reading the rule, not a test.
- **Not tested:** Homebrew's own git calls. It isolates global git config in some places, but I didn't establish whether tap clones are covered.
- **Fix:**
  - Amend ADR 0001: a Dotfiles Repo is code. The Config format itself stays as it is.
  - Make the whole `dotfiles` block a third approval, off by default on Import, unless the repo belongs to the signed-in GitHub account.
  - Tie that approval to repo plus commit SHA in the local record, and never pull past that commit without a new preview. This replaces the script hash.
  - Link only after the `sudo` helper is gone.
  - Run the engine's git with `GIT_CONFIG_GLOBAL=/dev/null` and `GIT_CONFIG_NOSYSTEM=1`, and its shells with `zsh -f`. Both stopped the marker in testing (`git status` and `zsh -c`).

### 2. Link targets can move user data or corrupt each other
- **Location:** `$defs/linkTarget`, and the README 3 claim that "two links can't claim the same location".
- **Accepted by the schema:** `~/Library`, `~/Documents`, `~/.ssh`, `~/Developer`, `~/Developer/dotfiles`, parent-and-child pairs, `~/.zshrc` with `~/.ZSHRC`, and `~/.zshrc ` with a trailing space.
- **Failure:** on this Mac's case-insensitive volume, the second link replaced the first. With overlapping links, the child landed inside the Dotfiles Repo. Backup-then-replace would also move all of `~/Documents` aside.
- **Fix:** add a per-link check in the engine. Refuse:
  - case-folded duplicates and ancestor/descendant pairs;
  - anything at or above the clone location;
  - a short list of protected folders (`~/Library`, `~/Documents`, `~/Desktop`, `~/Downloads`, `~/Applications`, `~/.ssh` and similar).

  This list guards against data loss, so it isn't the trust blocklist the designer rejected. Also refuse leading or trailing spaces in path segments, `.git` segments, and repo symlinks that resolve outside the repo.

### 3. `gh` creates false Drift and a removal offer
- **Location:** `owner-personal.json`, `owner-work.json`, `developer.json`, stub `gh`, README "Not in a Config" table.
- **Failure:**
  - `minimal.json` doesn't want `gh`, but every Machine has it. Under Adopt's first row, it shows as Drift everywhere.
  - Dropping `gh` from a Config that lists it triggers the "no longer in this Config" removal offer for a provisioning tool.
  - `packages.formulae: ["gh"]` and `evil/tap/gh` both validate.
  - The stub's `formula: gh` also conflicts with the #12 decision to download `gh` without Homebrew. The result is either "missing" or a second copy.
- **Fix:**
  - Mark `gh`, Homebrew and Command Line Tools as always installed in the Catalog. A Config can neither select nor omit them.
  - Ignore any entry naming one, with a note, and drop it at the next reviewed write. Don't reject the whole file, because this list can change between releases.
  - Exclude them from Drift, Adopt, removal and Import's choices, matching by Item ID and package name. Warn on third-party packages with the same name.
  - Remove them from the examples. `check.py` must then count `brew "gh"` as covered: removing them today fails with `missing ['brew "gh"']`.

### 4. Catalog version skew causes false removal offers and an endless rewrite loop
- **Location:** README section 2 rename row, and section 1 "keeps only the Item".
- **Why it's common:** the Catalog ships inside each release and updates are user-chosen, so Machines will routinely run different Catalogs.
- **Failure (modelled):**
  - **Rename:** a newer app rewrites `tool-old` to `tool-new`. An older Machine then sees an unknown ID, reads `tool-old` as dropped and offers removal. It also sees `tool` as extra and offers to Adopt it into `packages`.
  - **Package gains an Item:** the two apps rewrite `ripgrep` between `items` and `packages` on every round. I ran three rounds and it never settled.
- **Fix:** resolve old IDs at read time forever, and rewrite only as an explicit separate action. While a Config holds any ID this app doesn't know, offer no "dropped" removal and no package Adopt, and say to update the app instead.

### 5. Adopt can make a shared Config unusable on every Machine
- **Location:** `$defs/settingString` and `settingId`, README section 4.
- **Failure:** `dock.apps` containing `/Users/me/Applications/Foo.app`, or `screenshots.location` of `/Users/me/Pictures`, fails the schema. If Adopt commits that, every Machine following the file refuses it.
- **False positives:** the same whole-file refusal hits the locale `sk-SK`, `screensaver.require-password`, `safari.autofill-passwords` and `ssh.auth-sock`.
- **Later risk:** adding a token prefix in a later release would break Configs that are valid today.
- **Fix:**
  - Move the secret checks out of the schema into a lint stage: a hard stop on app writes, and a per-entry skip on read.
  - Freeze each schema version once published.
  - Validate every Adopt result before committing it. Mark values with no valid spelling as not adoptable.

### 6. There's no state for "this Machine can't apply this"
- **Location:** README section 5 ("never add to or override") and section 6.
- **Failure:** the owner's `dock.apps` lists `Launchpad.app`, which isn't on this Mac (macOS 27.2 has `Apps.app`).
  - The whole-list comparison would never match, so Apply can't clear the Drift.
  - Adopt would delete the tile for every Machine.
  - The same applies to `xcode` on a Machine with no Apple Account.
  - A real hot corner has no spelling, because the stub only defines `none`.
- **Fix:** add a per-entry "unavailable here" result, with a reason, to the last-applied record. Show it apart from Drift and offer no Adopt. Amend the boundary so local records may suppress an entry but never add or change one. Compare list settings only on the tiles available on that Machine.

### 7. `developer.json` writes Git identity into the Dotfiles Repo
- **Location:** `developer.json` combines the `git-identity` Item with the link `~/.gitconfig: git/gitconfig`.
- **Failure:** `git config --global` wrote the name and email through the symlink, leaving `git/gitconfig` modified in the repo and one push away from publication. With today's order, where `git.sh` runs before `dotfiles.sh`, identity would instead end up in the backup. `git.*` settings have the same collision.
- **Fix:** add a rule that a value a Catalog Item writes never goes through a link into the repo. Report that entry instead. Remove the combination from the example.

### 8. Sign-in Items contradict the owner's decisions
- **Location:** `gh-sign-in` and `tailscale-sign-in` in the examples and stub, schema lines 26 and 87, README lines 14, 51, 84 and 196, GLOSSARY line 18.
- **Failure:**
  - GitHub sign-in always happens, so selecting it means nothing and omitting it falsely suggests a skip.
  - `tailscale-sign-in` contradicts install-only, and resolves alone to a queue step with nothing installed.
  - The stub's Tailscale `needsYou` also looks stale.
- **Fix:** delete the sign-in kind and remove "sign-in" from the glossary's Item definition. Keep Xcode's Apple Account as a Needs-You step on `xcode`. #12 should decide the Tailscale system-extension step.

## Non-blocking findings (reproduced or modelled; fix or hand off)

| # | Location | Failure | Fix |
|---|---|---|---|
| 9 | Regex patterns | See question 2 below | Rewrite before publishing, since the schema freezes then |
| 10 | Duplicate keys | Stock `serde_json` accepts `{"schemaVersion":99,"schemaVersion":1}` and keeps 1, so the preview could differ from what the engine reads | Use a custom visitor. I tested one of about 30 lines and it refuses these |
| 11 | `check.py` `strict_load` | Accepts `NaN`, `Infinity` and `1e999`, despite the README's "plain RFC 8259" | Add `parse_constant` and a finite check |
| 12 | Item and package overlap | Only exact names match. `opencode` with `anomalyco/tap/opencode-v2` installs both, which the research says conflict. `tailscale` with the formula hits a pre-install dialog that can stall a Run. `developer.json` itself uses the vendor formula although a Catalog Item exists | Add Catalog `coversPackages` and `conflictsWith`, and check per entry before a Run |
| 13 | Package gains an Item | With no duplicate listed, nothing defines the migration. The record key changing from `formula:x` to the Item ID reads as dropped | Migrate the key, and make the rewrite an explicit action, as in finding 4 |
| 14 | Retired Items | The stub keeps only a message, so the app can't tell it's installed. The leftover package then reads as extra, and Adopt offers to add it to `packages`. A retired setting also gets the wrong "isn't a setting" note | Keep install and detection data forever |
| 15 | "One spelling" (README 5) | False: 281 aliases on 250 formulae pass, so `nvim` would show permanent two-way Drift against `neovim`. `jetbrains/homebrew-utils` also passes, and Homebrew normalises it to `jetbrains/utils` | Refuse tap segments starting `homebrew-`, resolve aliases from metadata, correct the claim |
| 16 | Rename collision | `dock.hide` and `dock.autohide` with different values gives no note. A rename cycle hangs `follow()` | Skip both and report; lint the Catalog for cycles |
| 17 | Settings with no Item | `android.*`, and `dock.apps` without `dockutil`, resolve silently. That means either a hidden install, against the selection rule, or a failed setting | Add `requires` on settings and report it per entry |
| 18 | `name` and unknown settings | U+202E, U+009B, zero-width characters, U+2028 and "Verified safe by Soulcaster" all pass. Unknown setting values become a 256-character notes channel, the very thing the README leaves out `description` to avoid | Tighten the pattern (I tested an ECMA class) and show these as untrusted data |
| 19 | No size limits | 500,000 items pass: Rust took 15 ms and Python 4 s. Ajv took 0.67 s at 20,000, and I stopped it after 3.5 CPU minutes at 500,000. The real risk is burying one line in a preview | Add a size cap before parsing, `maxItems`, `maxProperties` and a `repo` `maxLength` |
| 20 | Stub values | `keyboard.key-repeat: -5` and `keyboard.initial-key-repeat: 1` pass, as does `git.excludes-file: ~/../../../etc/passwd`. Ten settings have no `store` field and no separate read path | #12 should require bounds and a read path |
| 21 | `junie` (README line 205) | Official Homebrew now has a JetBrains `junie` cask, so the stub's third-party tap may be an unnecessary approval | Check it and prefer the cask if it's equivalent |
| 22 | README consistency | Section 6's "Adopt deletes the line" conflicts with section 1's "Adopt never writes `dotfiles` or `taps`". `minimal.json` counts as canonical without `$schema`, though the app "always writes it". Empty `links` also survive `canonical()` | Align the text and the code |

## My view on the six questions

**1. Links:** ADR 0001's decision stands, because a Config still carries no commands. Its Consequences are incomplete, because `dotfiles` points at mutable code. The approvals are currently bypassable. The fix is in finding 1, and the schema shouldn't try to judge trust.

**2. Rust and regex:** the schema isn't unsafe, and Python is the outlier.
- Plain `regex` can't compile 7 of the 13 patterns, and it fails loudly at build rather than silently.
- `jsonschema` 0.60.2 with its default `fancy-regex` matched Ajv on all 240,000 fuzz inputs, and every disagreement in the 94-case set was a parse-stage difference.
- Python accepts a trailing newline in all 11 pattern types tested, including link targets and `installScript`. That goes wider than the README's one example, but Rust and Ajv refuse them, so the app fails closed.
- **Change anyway:** move the lookaheads into `not: {pattern}`. That rewrite compiled under plain `regex` and showed 0 differences across the same fuzz and the 94 cases.
- Pin the crate, run a shared conformance set on Rust and Ajv in CI, and state that Python's `$` doesn't conform.

**3. Departures from the research:**
- **Unprefixed IDs:** justified. The two grammars already separate Items from settings.
- **Optional `$schema`:** justified, but the `const` also refuses a local path or a tagged URL. Document that, or relax it. A `raw.githubusercontent.com/.../main` URL also embeds the repo name, which has already changed once.
- **No `importedFrom`:** justified for a copy, and the local record suits it. The stated cost is real, though: provenance stays on one Machine, and adding the field later forces a version bump.

**4. `git.*` and `android.*`:** keep `git.*`, drop `android.*`.
- `git.*` is typed, readable and has no code-running keys. It needs the collision rule from finding 7, and the map's wording should change to "settings the Catalog exposes".
- `android.*` holds install parameters, not settings. Going from platform 35 to 36 leaves both installed, so one value can't describe the Machine. `android.emulator: false` would also be removal by another name. Use Catalog recipes or Items instead.

**5. Secret detection:** acceptable only as a best-effort aid and never as a whole-file rule.
- **Bypassed by:** a leading space, `Bearer`, `sk_live_`, JWTs, `npm_`, `hf_`, a bare password, `sudo.pass`, `git.email`, and a token in `name` or a link source.
- **The real guard:** typed fields and scanning at write time. See finding 5.

**6. Unknown field versus unknown ID:** coherent in principle, because a field needs new app code and an ID only needs Catalog data.
- **File damage:** an older app's Adopt doesn't damage the file. Unknown entries and list order survive, and `canonical()` is stable.
- **Meaning damage:** this is the real problem, and it comes through version skew (finding 4). Two gaps remain: schema value checks that act as whole-file failures (finding 5), and Adopt overwriting a value this app refuses to read. Don't offer Adopt on such an entry.

## Judgment calls (not reproduced)

- **`git-identity` adds a third up-front ask (blocking-level conflict with the map).** The map says a Run asks for only two things. Fill the values in this order: existing `git config`, the local file, the GitHub account, then the Needs-You Queue. Never prompt during a Run. The owner may want to confirm this.
- **Drop `packages.taps` from version 1.** Its only use looks dead, Adopt can't write it, and it asks for approval with no purpose.
- **Re-review IDs that start resolving.** An ID shown as "unknown, skipped" at Import can become a real Item after an app update. Treat it as a new selection.
- **Show typo suggestions before a Run.** `ghosty` only skips and reports, which suits automation but not a person who then walks away.
- **Loosen the path character set.** These real values reject the whole file: `com~apple~CloudDocs`, `Übersicht`, folders with parentheses, and repo URLs with ports.

## Checked and found sound

- **Owner's later decisions:** nothing in the format contradicts the `sudo` rule. There's no field for the password, and `sudo` and `sudo.password` are refused. Add one line saying it's stored nowhere. The Catalog-in-release decision fits too, because the app never fetches anything; its consequence is finding 4.
- **Shape:** the closed-object design refuses shell commands, trust grants, removal fields and includes. Package injection, credentials in URLs, SSH repos and path climbing are also refused.
- **Package names:** `.rb`-shaped names pass the schema, but Homebrew 7.0.12 refused to load one, so its body didn't run. Refusing them in the schema is still cheap defence in depth.
- **Translation:** Dock, Finder, keyboard, menu bar, macOS, git, Android and link values all match the scripts, including list order. The exceptions are the items above and the known-dead keys the README already flags.
- **Check script:** it isn't vacuous. Removing or misfiling an entry, and reordering keys, all made it fail.
- **`azure-cli`:** the move from formula to cask is confirmed in live Homebrew data.

## Validation rerun

| Check | Result |
|---|---|
| Python `check.py` | Exit 0, output byte-identical to the README, `all checks passed` |
| Ajv | Exit 0, 5/5 accepted, 14/14 refused, `"ripgrep\n"` refused |
| README counts | 5 valid, 15 invalid and 35 edge cases all confirmed |
| Homebrew name patterns, live data | 0 of 8,704 formulae and 0 of 7,819 casks refused (the README had 7,816 casks) |
| Rust `jsonschema` 0.60.2 (new) | 5/5 accepted, 14/14 refused. Stock `serde_json` accepts `duplicate-key.json` |

The README's claims hold. The gaps are in what it didn't test: the owner's later decisions, Rust, version skew and links as code.

I changed nothing in the repo: the worktree is clean, with no commits, pushes, PRs or issue comments. Tooling stayed in the scratchpad, and I've removed the heavy parts. The one `brew` command used an isolated home and cache. The test scripts are in a local scratchpad directory.

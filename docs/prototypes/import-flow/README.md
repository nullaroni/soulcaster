# Import preview (prototype)

**Throwaway prototype answering [#15](https://github.com/nullaroni/soulcaster/issues/15).** It is not the app. It fetches nothing, installs nothing, and changes nothing. The question was what someone sees and approves when they Import a Config. One flow is decided here, because the owner asked for the open tickets to be finished and the calls to be made. An agent decided it.

The rules live in [`check/import_model.py`](check/import_model.py). [`check/check.py`](check/check.py) runs them on [`samples/`](samples/) through the Config format's own strict parser, `resolve()` and lint, pinned to `origin/prototype/config-format` at `4ea7e9b`. The Rune app in [`app/`](app/) only draws `app/plans.json`, which that check writes. What an approval is allowed to run comes from the [Item lifecycle](https://github.com/nullaroni/soulcaster/blob/prototype/item-lifecycle/docs/prototypes/item-lifecycle/README.md): third-party taps, Dotfiles links and the install script run in the User Stage, after the Admin Stage has discarded the password.

## Answer

Import by `owner/repo` or URL makes a local copy of one Config at one commit. Reading it runs nothing from it. The copy does not follow its source.

Before a Run, one review page shows everything the file would install or change, and asks up front for each thing that can run someone else's code. Those are separate, and none implies another:

- each third-party Homebrew tap
- the Dotfiles Repo, meaning the repo and every link in the file
- the Dotfiles install script, only after the repo is on, and only if Soulcaster can show the script

A safe Config can go straight to Start Run. Catalog Items, official Homebrew and settings start selected. Taps, a stranger's Dotfiles Repo and every install script start off. A Config that looks built to mislead starts with nothing selected, says so in a warning, and offers Close as the primary button.

## From a pasted link to a Run

1. **Paste** `owner/repo`, or a link to a repo or to one file. The paste stays on screen if the read fails.
2. **Read that one commit.** A repo link lists `configs/*.json`. A file link opens that file. A Machine follows one Config, so several files means picking one. The list shows each file's name, how many things it would install, how many settings it would change, and how many parts need approval.
3. **Refuse the whole file, or review it.** A file that fails the strict parser or the schema is refused in full. Nothing is kept. A file that passes is reviewed even when some entries will be skipped.
4. **Review, on one page.** The footer says what a Run from the current boxes would do, before the button is pressed. Cancel drops the copy. Keep a copy saves it and records no approvals. Start Run is the only way approvals are written, and they are written on this Machine only.
5. **Two questions, then the Run.** The Mac password, only because this plan has admin work, and GitHub sign-in in the browser. The order on that screen is the Item lifecycle's order: Admin Stage, discard the password, User Stage, and only then the taps, links and script that were turned on. Back returns to the same boxes.

Pressing Start Run on a clean Config, without touching a box, installs its Catalog Items, official Homebrew packages and settings. It does not tap, clone or run a script. Pressing it on a hostile Config is impossible until something is selected, and selecting nothing is how that page opens.

## What the review shows

The name in the file is shown as data, in quotes, with the line that Soulcaster doesn't verify or endorse any Config. Letters that mix alphabets are also written out as code points, so the name on screen is not the only form the person sees. The source is the repo, the path and the commit. Nothing has been installed yet.

Four counts sit under that: what would install, what settings would change, how many parts need approval, and how many entries won't apply. A blocked tap is not counted as needing approval, because no box is offered for it.

**Can run someone else's code.** This block is absent when the file has no tap and no Dotfiles Repo, and the page says everything comes from the Catalog or official Homebrew. Otherwise each part is its own box, drawn with a rule on the left. The rule is the only mark. There is no score and no icon. The rule is quiet while the box is off and amber while it is on.

Each tap is grouped, not buried in the package list. The card names the account that wrote the formula, says Homebrew will run that code as the user after the password is gone, says a later update of an already approved package runs the newest formula with no new question, and points at the formula on GitHub. It also says which line of which list the package came from, so a tap on line 272 of 480 is at the top of this block. A Catalog Item that loads a tap, such as AeroSpace and `nikitabobko/tap`, is the same kind of card. Leaving the box off skips those packages and the Run continues.

A package whose short name is an always-installed Item, such as `evil-tools/tap/gh` beside `gh`, is not a card with a box. It is labeled not offered. It stays in the copy and never installs.

The Dotfiles Repo card lists every link that lint allowed, as `location ← path in the repo`, in monospace. A location that runs by itself gets a note beside it: `~/.zshenv` runs for every zsh, `~/.gitconfig` can run programs during git, `~/Library/LaunchAgents/` starts a program at login, and the same kind of note for SSH and a fake `sudo`. A location with no note is still part of the approval. The note explains. It does not refuse. Existing files are backed up before they are replaced. Leaving the box off means the repo is not downloaded.

The script is a second box, off, and it stays disabled until the repo is on. The card shows every line of the script. Turning the repo off turns the script off. A script Soulcaster cannot show cannot be ticked.

**Will install.** Catalog Items, then official formulae, then official casks. Each group is one box, on, unless a signal turned the page off or the list is longer than 100 names. A long list shows the first 14 names and a control to show the rest, and it starts off. Items pulled in by `requires` are named with the reason. Command Line Tools, Homebrew and `gh` are a line of their own: every Run installs them, and the file cannot select them.

**Will change settings.** A table of the setting, the value on this Machine now, the value after, and when it takes effect. A list such as the Dock says it replaces the whole Dock and names what leaves. A setting that would write through a linked file says it is skipped if the Dotfiles Repo is turned on. The group starts on, unless a signal turned the page off.

**What you'll be asked.** License acceptance starts off. A Config cannot accept a license. Leave it off and that Item waits in the Needs-You Queue. The password and GitHub sign-in are named here and collected on the next screen, not in the middle of the Run. Other Needs-You steps are listed for after the Run, including ones that apply only if a tap was approved.

**Won't apply.** Unknown IDs, retired Items, always-installed names, values the Catalog refuses, personal values, names that look like secrets, and links lint refused. A refused link is one that would replace a protected folder, land on the same location as another link, or write into the Dotfiles Repo's own folder. These stay in the copy. The value of a secret-shaped or unknown setting is not shown. The rest of the file carries on.

## A risky Config and a safe one

The difference is the first screenful, not a badge the person learns to ignore.

| | Safe (`samples/clean.json`) | Risky (`samples/hostile.json`) |
|---|---|---|
| Opening | No warning. "9 to install, 6 settings to change, 0 need your approval." | A red block: "This file looks built to mislead you, so everything starts off," then each reason in words. |
| Boxes | Catalog, official Homebrew and settings are on. There is nothing to approve. | Every box is off, including the safe groups. A Run would have nothing to do. |
| Primary button | Start Run, filled in. | Close without importing, filled in. Start Run is an outline and disabled until something is selected. |
| Third-party code | The approval block is the sentence "Nothing here needs approval." | Blocked names first, then each tap and the Dotfiles Repo as its own off box. The buried tap is in that list, not on line 272. |
| Counts | "to install", "settings to change". | "could install", "settings could change". |

A signal is one of: the name mixes alphabets, the name says Soulcaster verified or endorsed it, the account or a tap reads like Homebrew, GitHub, Apple or Soulcaster, a tap package shares a base Item's name, the Dotfiles host looks like GitHub and isn't, or a link location mixes alphabets. Finding one starts the page off. Finding none does not mean the file is safe, and the page never says it is.

A Config can be ordinary and still need approval. One tap, or a stranger's Dotfiles Repo, keeps the safe groups on and adds off boxes under "Can run someone else's code." The footer then reads "Start Run without them" and names what was left off. That is the click-through the rules allow: third-party code stays off, and the person is not asked to tick the safe parts to reach it.

Your own Dotfiles Repo is the exception that starts on. "Yours" means the signed-in GitHub login owns both the Config and the repo, on `github.com`, compared case-insensitively. A URL on another host is never yours, including a host that starts like `github.com`. Before sign-in, Soulcaster cannot know it is yours, so it starts off and offers sign-in. The script starts off even then.

## Private repos and failed fetches

GitHub is asked only to read. A failure keeps nothing and changes nothing, and the paste is still there.

| What happened | What the page says |
|---|---|
| No network | Soulcaster couldn't reach GitHub. Try again. |
| Repo or file not visible, not signed in | It doesn't exist, or it's private. GitHub gives the same answer for both, so Soulcaster can't tell which. Sign in, in the browser. The password is never typed into Soulcaster. |
| Still not visible once signed in | This account can't see it. Check the spelling, or ask the owner. Another GitHub account is the other way through. |
| Rate limited | GitHub is limiting this network. Sign in for a higher limit, or try again after the time GitHub gives. |
| A repo with no `configs/*.json` | No Configs there. Paste a link to the file if it lives somewhere else. |

A file that was fetched and then refused is a different page. The whole file is refused, not one field:

- characters in the name that change how text reads, shown written out as code points
- a field a Config can't have, including anything that would approve a tap or a repo for the importer, or carry a command
- the same key twice, because programs disagree on which one counts
- a newer `schemaVersion`, which asks for an update and does not call the file wrong

## After the copy exists

**Keep a copy** saves it on this Machine and can commit it to your Config Repo as a new file. The commit does not say where it came from. Approvals stay on this Machine. Saving does not approve a tap, and a stranger's Dotfiles Repo does not become yours because the Config now lives in your repo. On another Machine the same file asks again. An entry that looks like a secret stops the save until it is removed.

**A Run that left something out** says so once, at the end. An unapproved tap is not Drift and it is not in the Needs-You Queue. The report offers Review again. It does not nag.

**The source of the Config changes.** Soulcaster looks when you open the Config. The copy does not move unless you choose, and choosing updates the copy only. A new package from a tap you already approved starts off. A Dotfiles Repo that was not in the file before starts off. Changed settings and new official packages need no new approval and apply at the next Run's review. A line the source removed is "no longer in this Config", never an uninstall. A value you changed in your copy stays.

**The Dotfiles Repo moves.** This Machine stays at the approved commit. Soulcaster does not pull. A stranger's newer commit starts off, and the page shows the linked-file diff as text, including files that run when a terminal opens. Your own newer commit starts the move box on, and nothing moves until you press the button. The script box starts off either way: an approval at the old commit does not cover the new one. If the history was rewritten, there is no diff. The page says to treat it as a repo you have never seen, and both boxes start off. A transferred repo, or a different repo in the file, voids the old approval. So does a link added at the same commit.

## Decisions the owner may want to overrule

An agent made these because the owner asked for the tickets to be finished. **[security]** or **[product]** says why it is flagged.

1. **[security] A stranger's Config that names your Dotfiles Repo starts off.** ADR 0001 turns a repo on when it belongs to the signed-in account. This prototype also requires that account to own the Config, because the other person wrote the link list.
2. **[security] A third-party package that shares a base Item's name is not offered for approval.** The Config format warns and keeps that package as its own package. Here `evil-tools/tap/gh` can never be approved.
3. **[security] A tap approval is the package names, not the tap.** A new package from that tap starts off. Updating a package you already approved runs the newest formula with no new question, when you choose to update.
4. **[security] One signal turns every box off, including Catalog Items and official Homebrew, and Close is the primary button.** The signals are a short heuristic: mixed alphabets, a few claim words, and a rough folding of Homebrew, GitHub, Apple and Soulcaster. They will miss files and they will flag honest names.
5. **[product] Safe groups start selected.** Start Run on an ordinary Config installs them without a tick per Item. The map's Ninite line is "nothing is installed unless the user selects it." This treats the review plus Start Run as that selection, so the off-by-default boxes are only the ones that run someone else's code.
6. **[product] An official Homebrew list longer than 100 names starts off.** The owner's Configs are well under that. A long list is still official Homebrew.
7. **[security] A script whose text cannot be shown cannot be approved.** A Dotfiles Repo that is not on GitHub cannot be read until Git exists, and a bare Mac does not have Git yet, so that script stays off at Import.
8. **[security] Your own Dotfiles Repo starts on, so Start Run links those files without another click.** The script still starts off. Your own newer commit starts the move box on, and still does nothing until the button. Rewritten history starts off even when the repo is yours.
9. **[security] Link notes do not refuse a link.** `~/.zshrc` can be approved. Only the Config format's lint refusals are skipped.
10. **[security] Keep a copy records no approvals and writes no provenance into the file.** The next Machine asks again. A secret-shaped entry blocks the save.

## What was not verified

The check printed `result: all checks passed` against `origin/prototype/config-format` at `4ea7e9b`, including the straight-through clicks and 4,000 random box states. `cargo build` of [`app/`](app/) succeeded offline against the cached Rune rev `d03349e`. Headless PNGs were rendered for `paste`, `review-clean`, `review-hostile` and `failed-not-visible`. Nobody drove the window. No GitHub request, Run, install, clone, link or password prompt was performed. "Yours" compares login strings. It does not ask GitHub where a transferred repo lives now. The lookalike fold is not UTS #39, and the host check is the last two labels, not the public suffix list.

## How to look at it

From the repo root, with the Config format files available as a directory (schema, `catalog-stub.json`, and `check/check.py`):

```sh
TMP="$(mktemp -d)"
python3 -m venv "$TMP/venv"
"$TMP/venv/bin/pip" install --quiet --no-cache-dir jsonschema==4.25.1
"$TMP/venv/bin/python" docs/prototypes/import-flow/check/check.py \
  docs/prototypes/import-flow <config-format dir> --write-plans

source "$HOME/.cargo/env"
cargo run --manifest-path docs/prototypes/import-flow/app/Cargo.toml -- \
  --snap /tmp/review-hostile.png --scene review-hostile --size 980x1400 --scale 1 --theme light
```

`--scene` also accepts `paste`, `review-clean`, `failed-not-visible`, `review-one-tap`, `review-stranger`, `review-own`, `first-minute`, `update-config`, `update-dotfiles-stranger`, `update-dotfiles-own`, `update-dotfiles-rewritten` and `save-copy`. `cargo run` with no arguments opens the paste screen. The sample buttons are the prototype's stand-in for a fetch. The app has no network client.

# Sync when two Machines both change (prototype)

**Throwaway prototype answering [#13](https://github.com/nullaroni/soulcaster/issues/13).** It isn't merged, and it isn't the engine. [`model.py`](model.py) simulates two Machines against a local bare git repo and checks invariants I1–I14 after every action. Homebrew, GitHub's servers and the Mac's real settings are not involved. The click-through HTML demo from the prototype skill was skipped: nobody is here to drive one, and the question is which state changes are legal.

Nothing here was run on a real Mac. No app was installed or removed, no setting was written, no `sudo` was used, and nothing was sent to GitHub.

## Conclusion

Drift is one row per entry. The Machine Record's last-applied value stays on the Machine and is never Adopted. The app compares three things: the Config, the Machine, and that last-applied value. Whoever changed is the side that differs from last-applied. A missing line in the Config is not a removal. Another Machine hears about it as Drift, and nothing is uninstalled until that Machine confirms.

| Question | Decision |
|---|---|
| When Drift is checked | At launch, when the window is focused, hourly while the app is open, and when the person asks. A check changes nothing on the Machine and sends nothing. |
| What Adopt writes | The reviewed entries, and nothing else, into the one Config this Machine follows. A separate reformat commit comes first when a hand edit left the file in another form. The last-applied record updates only after GitHub has the result. |
| Two Machines, different entries | Both saves land, one after the other. Each Machine then sees the other's entry as new, and Apply settles it. |
| Same setting, two ways | One row. The person keeps this Mac's value, takes the Config's value, or leaves this Mac and stops being asked. No save happens until they choose. |
| Offline for weeks | The save waits on this Mac. Coming back sends nothing by itself. Entries the other Machine didn't touch go through; the one both changed is asked again. Nothing is uninstalled. |
| A failed push | The reviewed entry is tried again on top of whatever GitHub has now. The same entry, if someone else changed it, comes back as a row. The waiting change is kept until it is sent or discarded. |
| A removal | Deleting the line asks every other Machine that last-applied it. Apply all never includes it. Uninstalling takes a separate confirmation, and it affects only this Mac. |
| Words on screen | The sentences in [The words on screen](#the-words-on-screen). None of them is a Git word. |

## Both Machines Adopt different changes

Machine A installs Zed and Adopts it. Machine B installs ripgrep and Adopts it, without having looked since A saved. GitHub ends with both entries, as two ordinary saves, one on top of the other. A then sees ripgrep as new in the Config. B sees Zed as new. Apply on each Machine installs the one it doesn't have. Nobody is asked to pick a winner, because the entries are different.

A change to some other file in the Config Repo is kept too. Adopt only rewrites the Config this Machine follows.

## The same setting, changed two ways

Both Machines change Dock tile size, from the shared 48. A Adopts 36. B, which is still on 64, gets one row: the Config says 36, this Mac says 64, and the last-applied value was 48. Nothing is written while that row is open.

| B chooses | Config | A | B |
|---|---|---|---|
| Save 64 to the Config | 64 | 64, after Apply | 64 |
| Change this Mac to 36 | 36 | 36 | 36 |
| Leave this Mac as it is and don't ask again | 36 | 36 | 64, and this entry is unavailable here |

If B Adopts 64 without looking, and A has already saved 36, B's save is refused. The row comes back. A's 36 stays.

**Lists.** The Dock is a list, and order matters. If A removes Mail and B adds Zed, the shared apps are still in the same order, so the row also offers the combined list: A's removal and B's addition, Zed at the end. Choosing it writes that list to the Config and sets this Mac's Dock to it. The other Mac still Applies. If both Machines move Mail to a different slot, there is no honest combined order, so that choice is absent and the person picks one side. The tile is never doubled.

A tile this Mac can't show, such as Launchpad on an older system, is unavailable here. It is not Drift. Adopting the rest of the list puts that tile back in its old place, so other Machines keep it.

## A Machine offline for weeks

Adopt while GitHub can't be reached leaves the change on this Mac: "Saved on this Mac, but not on GitHub yet." The last-applied record does not move. The row still says this Mac changed. When the network returns, a check sends nothing. The person sends.

On that send, an entry the other Machine left alone is saved. An entry both sides changed is not saved, and comes back as a two-place row. Entries the other Machine added wait for Apply, including a third-party tap, which asks in the Needs-You Queue before anything from that tap is installed. An entry the other Machine dropped is "no longer in this Config", and the formula is still installed.

## A failed push

The save is built from the entries the person reviewed, then sent. If GitHub has moved, the app reads the new Config and tries the same entries again, up to three times.

- **Someone else added a different entry, or edited another file.** The reviewed entry still means what the person saw, so it is saved on top with no second question.
- **Someone else changed the same entry.** The save is not written. The row comes back: "{name} also changed in your Config, so this wasn't saved. Choose which one to keep."
- **This Mac's value changed after the person chose.** The save is not written. "{name} changed again on this Mac, so this wasn't saved. Review it again."
- **GitHub keeps moving, three times.** "Your Config keeps changing on GitHub, so this wasn't sent. Nothing is lost. Try again in a moment." The change is still waiting. A later Send finishes it.
- **No network.** The change waits. The last-applied record stays. A later check still sends nothing until the person sends or discards. Discard keeps the Mac as it is, and the row returns.
- **GitHub needs a sign-in.** The change waits. On a public Config Repo, a check still works. On a private one, the check pauses too. Neither is a broken Config.
- **GitHub refuses the account or the branch rules.** "GitHub didn't accept this." No second attempt. The change waits until the person sends again.
- **The app dies after GitHub has the save and before the Machine Record is updated.** The next Send sees the entry already there and writes nothing. The record then catches up.

A value the Config refuses, such as a home-folder path, is not offered. A public Config says, at the moment of saving, "This Config is public. Anyone can read what you save."

## How a removal reaches other Machines

The Config still can't say "remove". Adopt of something uninstalled on this Mac deletes that line, and the person is told: "Your other Macs will be asked what to do about {name}. Nothing is removed from them."

The other Machine sees a removal only because its last-applied record still has the entry and the Config no longer does. The row says the entry is no longer in this Config, it is still on this Mac, and nothing will be removed unless they say so. Checking does not uninstall it. Apply all does not include it, and writes nothing.

| They choose | This Mac | The Config | Other Machines |
|---|---|---|---|
| Keep it here and stop tracking it | Stays installed | Unchanged | Not asked again about this keep |
| Remove it from this Mac | Uninstalled, after the confirmation below | Unchanged | Not uninstalled |
| Put it back in your Config | Stays installed | The line returns | The Machine that dropped it sees a new entry and can decline |

The confirmation is its own step: "Remove {names} from this Mac? This uninstalls them. Your other Macs aren't affected."

A setting has no uninstall. The choices are to stop tracking it, or to put the value back in the Config. The value on the Mac stays until the person changes it.

Three cases never offer a removal:

- **No last-applied record for that entry.** A new Machine, or one whose Machine Record was lost, may see "on this Mac but not in your Config". It can Adopt or keep it on this Mac only. It cannot be offered an uninstall of something this record never saw applied.
- **An always-installed Item** (`gh`, Homebrew, Command Line Tools). It is never Drift. A reviewed write drops a line that names one.
- **The file holds an ID this version doesn't recognize.** There is no removal offer and no package Adopt until Soulcaster is updated. The row says to update. The line stays in the file. Only Tidy rewrites an old ID, and only on a version that knows the new one.

## The words on screen

These are the sentences the model checks, copied verbatim. `{name}` is the entry a person would recognize, `{value}` is the value being kept, `{n}` is the number of rows, `{when}` is when this Mac last read the Config, `{why}` is the reason, `{names}` is the list being uninstalled, `{path}` is the Config's path in the Config Repo, and `{new}` is where a renamed file appears to have gone.

Git words are refused in all of them: merge, conflict, commit, push, pull, rebase, branch, head, remote, origin, fetch, upstream, stash, sha, diverge, force, clone, checkout, ours, theirs, and fast-forward. "GitHub" stays, because that is the place the Config Repo lives.

| | |
|---|---|
| `status.ok` | Up to date with your Config. |
| `status.rows` | {n} differences to review. |
| `status.offline` | Can't reach GitHub right now. Showing your Config as of {when}. |
| `row.machine-moved` | {name} changed on this Mac. |
| `row.removed-here` | {name} was removed from this Mac, but your Config still lists it. |
| `row.config-moved` | {name} changed in your Config. |
| `row.new-in-config` | {name} is new in your Config. |
| `row.dropped` | {name} is no longer in this Config. It's still on this Mac, and nothing will be removed unless you say so. |
| `row.extra` | {name} is on this Mac but not in your Config. |
| `row.conflict` | {name} changed in two places, and they don't match. You choose which one to keep. |
| `row.held` | {name}: update Soulcaster to review this. Your Config lists something this version doesn't recognize. |
| `choice.adopt` | Save {value} to your Config |
| `choice.apply` | Change this Mac to {value} |
| `choice.combined` | Use both changes: {value} |
| `choice.not-here` | Leave this Mac as it is and don't ask again |
| `choice.keep` | Keep it here and stop tracking it |
| `choice.remove` | Remove it from this Mac |
| `choice.add-back` | Put it back in your Config |
| `choice.keep-here` | Keep it on this Mac only |
| `confirm.remove` | Remove {names} from this Mac? This uninstalls them. Your other Macs aren't affected. |
| `confirm.adopt-removal` | Your other Macs will be asked what to do about {name}. Nothing is removed from them. |
| `save.waiting` | Saved on this Mac, but not on GitHub yet. Your other Macs can't see this until it's sent. |
| `save.send` | Send to GitHub |
| `save.discard` | Discard, and keep this Mac as it is |
| `save.sign-in` | GitHub needs you to sign in again before this can be sent. Nothing is lost. |
| `save.denied` | GitHub didn't accept this. You may not be allowed to change this Config, or its rules may not allow direct changes. Nothing is lost. |
| `save.changed-elsewhere` | {name} also changed in your Config, so this wasn't saved. Choose which one to keep. |
| `save.changed-again` | {name} changed again on this Mac, so this wasn't saved. Review it again. |
| `save.not-adoptable` | {name} can't be saved to a Config because {why}. It stays on this Mac. |
| `save.busy` | Your Config keeps changing on GitHub, so this wasn't sent. Nothing is lost. Try again in a moment. |
| `save.public` | This Config is public. Anyone can read what you save. |
| `config.unreadable` | Your Config on GitHub can't be read: {why}. Nothing on this Mac has changed. Fix it on GitHub, or go back to the last version that worked. |
| `config.restore` | Go back to the last version that worked |
| `config.too-new` | This Config needs a newer version of Soulcaster. Nothing on this Mac has changed. |
| `config.missing` | {path} isn't on GitHub any more. Nothing on this Mac has changed, and nothing will be removed. |
| `config.moved` | It looks like it moved to {new}. Follow it there? |
| `config.put-back` | Put it back |
| `config.choose` | Choose another Config |
| `config.rewritten` | Someone replaced the history of this Config on GitHub. Soulcaster compared it entry by entry, and nothing on this Mac has changed. |
| `switch.waiting` | You have changes waiting to send to {path}. Send or discard them before you switch. |
| `dotfiles.behind` | Dotfiles Repo: updates are available on GitHub. |
| `dotfiles.unpushed` | Dotfiles Repo: this Mac has changes that aren't on GitHub. |
| `dotfiles.review` | Review the updates |
| `dotfiles.by-hand` | This Mac has its own changes, so Soulcaster won't update the Dotfiles Repo. Update it yourself in Terminal. |
| `dotfiles.script-held` | Your Dotfiles install script changed since you approved it, so it won't run until you review it. |

Apply all covers only "changed in your Config" and "new in your Config". A two-place row, a removal, and a change that happened on this Mac are always one entry at a time.

## What else the rows do

**Untracked.** An app installed outside the Catalog, and not named as a Homebrew package, is Untracked. It is not a Drift row. It is not Adopted, not Applied, and not uninstalled. `{name} is on this Mac but not in your Config.` is only for a Catalog Item or a package the Config doesn't list. Those can be saved into the Config, or kept on this Mac only.

**Unavailable here.** Shown apart from Drift, with no Adopt and no Apply. The engine sets it when this Machine can't apply the entry. The person sets it with "Leave this Mac as it is and don't ask again." List settings compare only the entries available here.

**A file that can't be read.** A trailing comma, a duplicate key, an unknown field or a null setting: the Machine stays on the last version it could read. Nothing looks like a removal. Apply and Adopt wait. The person can fix it on GitHub, or go back to that last version, which is one ordinary save of those bytes. A newer `schemaVersion` is different: "This Config needs a newer version of Soulcaster." Going back is not offered, because that would replace someone else's newer file. One bad setting value skips that entry only and stays in the file.

**A missing or moved file.** Writes stop. Nothing is removed. A rename offers "It looks like it moved to {new}. Follow it there?" and does not follow by itself. A deleted file can be put back. A renamed default branch is the same idea, in the same plain words.

**Switching Config.** Waiting saves have to be sent or discarded first. Switching uninstalls nothing. Each entry the old Config had and the new one doesn't is a removal row. Switching straight back, with the Machine Record intact, leaves nothing to review.

**Someone replaces history on GitHub.** The person is told once. The app compares entry by entry and changes nothing on the Mac. The next save goes on top of the replaced history. Soulcaster never replaces history itself. The old version stays readable on this Mac.

**Dotfiles.** Adopt never writes the Dotfiles Repo. The Config screen shows only whether updates are available, or this Mac has changes that aren't on GitHub, or both. Updating stops at the commit the person just reviewed. If this Mac has its own changes, Soulcaster does not update it. A person's own update past the approved commit holds the install script until they review it.

**Two app versions.** An Adopt never rewrites an ID this other version wouldn't recognize. While the file holds such an ID, the older app offers no removal and no package Adopt. Tidy is explicit. Six rounds of alternating saves do not fight over the spelling.

## Decisions the owner may want to overrule

Each is decided so the ticket can close. **[cost]** is a product call. **[security]** is a safety call.

1. **[cost] "Use both changes" also changes this Mac.** The combined Dock is written to the Config and applied here in the same step. The other Machine still Applies. The alternative is to save the list and leave this Mac until a separate Apply.
2. **[cost] "Don't ask again" hides later edits too.** Marking an entry unavailable here suppresses it on this Machine even if another Machine changes that entry again. It reappears only if the entry leaves the Config. The alternative is to hide it once, and ask again the next time the Config moves.
3. **[cost] A different entry needs no second question.** If the reviewed entry is untouched, it is saved on top of whatever else landed. The alternative is to ask whenever GitHub moved at all, including an edit to some other file in the Config Repo.
4. **[cost] Three tries, then the person sends again.** A Config that keeps changing stops the save instead of retrying until it lands. The change is not dropped.
5. **[cost] Soulcaster will not update a dirty Dotfiles Repo.** Even when GitHub only added other files. The person updates it in Terminal. The alternative is to update the files the Mac didn't touch.
6. **[cost] "Go back to the last version that worked" replaces the file for every Machine.** That is how a bad hand edit gets undone. A newer schema version is the exception, and only says to update Soulcaster. The alternative is to never write over a file this Machine couldn't read.
7. **[cost] Drift is checked hourly while the app is open,** plus at launch, on focus, and on demand. The hourly check is the part most likely to be the wrong rhythm.
8. **[security] The Config Repo never learns the Mac's name or the person's email.** Saves use the GitHub account's noreply address. This prototype hardcodes that address.
9. **[security] A public Config is a sentence, not a second confirmation.** "This Config is public. Anyone can read what you save." The alternative is to ask every time.
10. **[cost] A lost Machine Record never offers a removal.** Something on the Mac that the Config doesn't list can still be Adopted, which can put back an entry a person had already dropped. That is the direction that can't uninstall by surprise.

## What this actually ran

Run on this Mac with Python 3.9.6, Apple Git 2.54.0 and `jsonschema` 4.25.1. The passing run is:

```sh
python3 docs/prototypes/sync/model.py --readme docs/prototypes/sync/README.md --schema config.schema.json
```

`config.schema.json` is the one on `prototype/config-format`. Every save is checked with that schema and with the model's own closed-shape check. The run is the scenarios, 150 random schedules, and one mutation per invariant. Each mutation breaks one rule and must be caught by that rule.

Not run, and not claimed:

- No second physical Mac, and no Machine offline for real weeks.
- No push, fetch or sign-in against GitHub. Auth failures and refusals are recorded git sentences, classified locally. Those sentences need a recheck against a real GitHub response.
- No Homebrew install, uninstall or tap. Packages are flags in the model.
- No `defaults` write, and no Dock change.
- No Rune screen. The sentences above are the copy the screen has to use.
- The Dotfiles guards were run on a temp repo, not on a person's Dotfiles Repo.

## How to run

From the repo root:

```sh
python3 docs/prototypes/sync/model.py
python3 docs/prototypes/sync/model.py --readme docs/prototypes/sync/README.md
```

`--schema path/to/config.schema.json` also checks every save with the Config schema, when `jsonschema` is installed. `--seeds` defaults to 150.

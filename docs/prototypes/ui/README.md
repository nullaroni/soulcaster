# UI (prototype)

**Throwaway prototype answering [#16](https://github.com/nullaroni/soulcaster/issues/16).** It is not the app, and it is not merged. An agent decided the layout because the owner asked the open tickets finished. The owner can overrule any decision flagged below.

**Nothing here installs.** There is no `sudo`, no Homebrew, no settings write, no GitHub call and no network from the prototype. The window only plays a scripted Run. The strip along the bottom says so, and it is not part of the product.

Rune stays pinned at `d03349e25189ce1edc8c26ada6bf6d4aac72c826`. That rev compiled.

## Run it

From `docs/prototypes/ui/app`, after `source "$HOME/.cargo/env"`:

```sh
cargo run
cargo run -- --snap out.png --scene catalog-picked
cargo run -- --list-scenes
```

`--scene` is one of `catalog`, `catalog-picked`, `review`, `review-answered`, `review-nothing-needed`, `run`, `finish`, `finish-clean`, `drift`. `run` is paused mid-Run, with a failure already in the log and another Item still installing. `finish` is the Needs-You Queue after that Run.

Headless shots of the four main screens are in `snaps/` (`catalog.png`, `run.png`, `needs-you.png`, `drift.png`). They were taken with `--snap --scale 1 --theme light`.

## How the screens connect

One window. The title bar is the path a person walks, and Drift sits beside it.

1. **Catalog.** Checkboxes over the shipped Catalog, plus settings and a by-name Homebrew package. Base Items (Homebrew, `gh`, Command Line Tools) are locked on. Nothing else starts selected. Continue goes to Review.
2. **Review** ("Before the Run"). Password, GitHub, licenses, third-party taps and conflicts, once. The side panel names what the Run will do and which steps will land in the Needs-You Queue. Start Run is disabled until the asks that block it are done.
3. **Run.** Progress for the whole Run, then the Admin Stage (password held, then discarded) and the User Stage. One failed Item is named in the header and opened in the log. Everything else keeps going. You can leave this screen.
4. **Needs you.** Where a finished Run drops you. The Needs-You Queue is the list only a person can finish. Failures and Items blocked on them sit under it, with Try again. A clean Run offers Drift.
5. **Drift.** Its own screen, from the title bar, and closed again from the same button. Unavailable while a Run is in progress, because only one thing may change the Machine at a time. Each line is Later, Adopt or Apply. Nothing is written until the footer button. Unavailable here is a separate group, with no choice. Untracked apps are listed and left alone.

Going back to Catalog or Review is allowed until the Run starts. After that those two steps are history. Needs you and the Run stay reachable so a failure can open its log.

## What was decided

- **One design, not a set of options.** The question was what the main screens look like and how they connect. The answer is this window.
- **Glossary words are the labels.** Catalog, Run, Needs-You Queue, Drift, Adopt, Apply, Base Item, Admin Stage, User Stage, Untracked, Unavailable here. The lifecycle's state names stay in code; the screen says Installing, Failed, Needs you.
- **Checkboxes are the Catalog.** A count in the footer is the only summary. Search and "Your selection" filter the same list. Competing Items (three Node version managers, and so on) sit together with a warning, and a hard conflict is named on the row.
- **A failure does not stop the Run.** The header names it, the log shows the error, and the User Stage keeps installing. The Needs-You Queue is not a second interrupt in the middle.
- **Drift defaults to Later.** Adopt saves this Machine into the Config. Apply changes this Machine to match the Config. The line under the control says which of those this row actually does (save, drop from the Config, reinstall). Uninstalling something the Config no longer lists is **Remove…**, a separate confirmation, because a Config cannot ask a Machine to remove anything and absence is not removal.
- **One footer button commits every Drift line** that is not Later. Its label is Adopt, Apply, or Adopt and Apply, matching what was chosen.
- **The sample Drift is hardcoded.** No engine has compared a Config with a Machine. The rows cover the kinds the Config format and the Item lifecycle already named, including a three-way conflict that shows the last-applied value.

### The owner may want to overrule

- **Glossary words on a public screen.** Adopt, Apply, Admin Stage and User Stage are accurate and dense. "Save to Config", "Change this Mac", "Needs your password" and "Everything else" are the plainer versions this draft used first.
- **"Review" has no glossary term.** The map calls this "answer everything once". The step is labeled Review and the heading is "Before the Run".
- **Adopt and Apply share one commit button.** Two buttons would make the two directions harder to fire together by mistake.
- **Remove… is not labeled Apply**, even though uninstalling a dropped Item does make the Machine match the Config. It stays a confirmed exception.
- **Drift is unreachable during a Run.** A person who wants to Adopt a failure mid-Run cannot.
- **The Needs-You Queue and the failures share one screen** at the end of the Run, rather than the queue being its own window during the Run.
- **The shots use a filled "typical" selection** so the later screens have a Run to show. The real Catalog still starts with nothing selected. `catalog` is that empty state.

## What was only simulated

The whole engine. Installing, checking, verifying, settings writes, the Admin Stage password, discarding it, the GitHub device flow, third-party tap approval, license acceptance, progress, the log text, Drift detection, Adopt and Apply. Any password text except `wrong` is accepted. GitHub approves itself and no browser opens. The Catalog JSON is the launch Catalog from the catalog prototype, read locally. Selecting an Item does not install it.

## What Rune made hard

Pinned rev `d03349e25189ce1edc8c26ada6bf6d4aac72c826` of `anomalyco/rune` (`rune` and `rune-kit`). It built. Headless `--snap` and `--scene` use the same `view` as the window.

- **Virtual rows cannot see `Cx` or the overlay layer.** The theme is stashed in a static for rows built outside `view`. A Select inside a Catalog row cannot open, so the open menu is built beside the list, against the trigger's key.
- **A wrapped control inside a card was clipped**, not given a taller row. Drift's Adopt / Apply control has to stay on one line. The three-value row (Config, this Machine, last applied) only fits because those columns are narrow.
- **Long Catalog section names need to wrap.** A fixed 28-point sidebar row truncated them ("Containers and virtual m…"). The row grows and the label wraps.
- **The log, the Run list and the stage chips are custom.** rune-kit supplies Progress, Dialog, SegmentedControl, Toast and TextInput. It does not supply a log, a checklist or a multi-stage run. The log keeps a 20,000-line tail in memory and virtualizes the lines. A real Run would write the file and keep a tail like this.
- **No release to depend on.** The pin is a `dev` rev. The app will have to track that API. This prototype did not need a newer rev.

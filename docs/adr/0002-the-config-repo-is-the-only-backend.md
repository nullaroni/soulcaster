# The Config Repo is the only backend

Macs in a Fleet need to know about each other, and the first map ruled out a hosted service. We decided the person's private Config Repo on GitHub is the only place a Fleet lives. There is no server of ours, and no account beyond GitHub. This costs speed, because a Mac finds out about changes by reading GitHub rather than being told, and it ties the product to GitHub.

## The layout

Machine Types stay where they are, in `configs/<slug>.json`. Each Machine adds one file about itself, a Machine Entry, at `machines/<id>.json`. No existing file moves, so a Config Repo needs no migration.

A Machine Entry holds only these fields:

- `schemaVersion`
- `id`
- `name`, the Mac's computer name
- `machineType`, the name of the Machine Type it follows
- `lastSeen`, in seconds since 1970
- `differences`, how many Drift rows the last check found, left out when there hasn't been a check

## Consequences

**A Machine's ID is 16 random bytes, kept on that Mac.** Computer names change, and serial numbers identify hardware to anyone who reads the repo. A reinstall makes a new ID, so the old entry goes stale and has to be removed by hand.

**A Machine writes only its own file.** Two Macs saving at once touch different files, so they can't conflict over content. A push that loses the race goes again on top of what's there. Reading an entry whose `id` doesn't match its file name is refused.

**A Machine Entry is deliberately not the Machine Record.** The Machine Record is never sent anywhere, and that stays true. Nothing is added to an entry without changing this ADR. App lists and settings values stay out, because those belong to the Machine Type.

**A public Config Repo gets no Machine Entries.** A Machine Type can be shared on purpose, but a Fleet record describes someone's computers. The Mac carries on working, and isn't listed.

**An entry holds the latest state only.** A Mac that can't reach GitHub doesn't queue anything. It sends what's true now the next time something happens.

**Every entry update is a commit.** That's fine for a save or a Run. Regular check-ins will need a low rate so the repo's history doesn't fill with them.

**Write access to the repo is the security boundary.** Anyone who can write to it can change what Macs are told to have. On a Full auto Machine, that reaches admin rights. Securing the GitHub account is part of securing the Fleet, and the sync decision will set out what limits this.

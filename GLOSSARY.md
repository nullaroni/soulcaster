# Soulcaster

A desktop app that sets up a Mac from scratch for developers and keeps that Mac matched to a shareable description stored on GitHub.

## Language

### What gets provisioned

**Machine**:
A Mac that is being provisioned or kept in sync.
_Avoid_: Device, host, computer

**Catalog**:
The neutral list of everything the app knows how to install or configure.
_Avoid_: Brewfile, package list

**Item**:
One entry in the Catalog, such as an app, a CLI, a setting, or a set of Machine-local values.
_Avoid_: Package, module, component

**Base Item**:
An Item every Run needs, whatever the Config says: Homebrew, `gh` and Command Line Tools. A Config can't select or omit one.
_Avoid_: Core item, required item

**Recipe**:
The data describing how the engine installs, configures and checks an Item, chosen from step types built into the signed app.
_Avoid_: Script, formula

### Describing a Machine

**Config**:
A JSON file describing what a Machine should have. A Machine follows one Config at a time.
_Avoid_: Role, profile, preset, manifest

**Config Repo**:
A user's GitHub repository holding their Configs.
_Avoid_: Settings repo

**Dotfiles Repo**:
A user's separate Git repository holding configuration files for their tools.
_Avoid_: Config Repo

**Import**:
Making a local copy of a Config published by someone else.
_Avoid_: Install, clone, subscribe

**Machine Record**:
The private local file holding what a Machine remembers: last-applied values, approvals, Machine-local values and the queue. It's never Adopted or sent anywhere.
_Avoid_: State file, lockfile

### Provisioning

**Run**:
One pass of bringing a Machine in line with its Config.
_Avoid_: Bootstrap, job, session

**Needs-You Queue**:
The single list of steps during a Run that only a person can complete.
_Avoid_: Manual steps, prompts

**Admin Stage**:
The part of a Run that may use the `sudo` password. It ends by discarding the password, and third-party tap code never runs in it.
_Avoid_: Root phase, sudo phase

**User Stage**:
The part of a Run after the password is discarded. Third-party taps, Dotfiles links and install scripts run here.
_Avoid_: Normal phase

### Staying in sync

**Drift**:
A difference between a Config and the Machine that follows it.
_Avoid_: Out of sync, mismatch

**Adopt**:
Saving Drift from a Machine into its Config.
_Avoid_: Capture, sync up, push

**Apply**:
Changing a Machine to match its Config.
_Avoid_: Restore, sync down, pull

**Untracked**:
Describes an app on a Machine that was installed outside anything the app manages.
_Avoid_: Unknown, unmanaged

**Unavailable here**:
Describes a Config entry this Machine can't apply, recorded locally with a reason. It's shown apart from Drift and can't be Adopted.
_Avoid_: Disabled, excluded, ignored

**Tidy**:
An explicit action that rewrites a Config's old IDs, aliases and covered packages to their current forms. No other write does this.
_Avoid_: Migrate, normalize

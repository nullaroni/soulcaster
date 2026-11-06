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
One entry in the Catalog, such as an app, a CLI, a macOS setting, or a sign-in.
_Avoid_: Package, module, component

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

### Provisioning

**Run**:
One pass of bringing a Machine in line with its Config.
_Avoid_: Bootstrap, job, session

**Needs-You Queue**:
The single list of steps during a Run that only a person can complete.
_Avoid_: Manual steps, prompts

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

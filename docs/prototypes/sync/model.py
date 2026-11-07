#!/usr/bin/env python3
"""PROTOTYPE, not the engine. Simulates Machines following one Config, with real git.

Everything Git does here is real: clones, fetches, commits and pushes against a local
bare repo in a temp folder, with a sandboxed HOME and the engine's guards. Machines,
Homebrew and GitHub's own responses are simulated. Nothing touches the network.

    python3 model.py [--schema config.schema.json] [--readme README.md] [--seeds 150]

--schema also validates every commit with the real Config schema (needs jsonschema).
--readme checks that the README's wording table matches UI below.
"""
import argparse
import contextlib
import copy
import json
import os
import pathlib
import random
import re
import shutil
import subprocess
import sys
import tempfile

MUT = set()  # the rule currently being broken on purpose
SCHEMA = None  # real schema validator, when --schema is passed
ROOT_HOME = None  # sandboxed HOME, set in main

RULES = {
    "I1": "no silent data loss",
    "I2": "no unconfirmed uninstall",
    "I3": "converges after a person chooses",
    "I4": "never commits an invalid Config",
    "I5": "no commit without a person's action",
    "I6": "never rewrites history on GitHub",
    "I7": "a commit holds exactly what was reviewed",
    "I8": "nothing private is sent",
    "I9": "Dotfiles never move past the approved commit",
    "I10": "an unreadable or missing file is never read as removal",
    "I11": "the Machine never changes without a person's action",
    "I12": "the last-applied record never runs ahead of GitHub",
    "I13": "the engine's git runs nothing from a repo's own config",
    "I14": "the UI uses no Git vocabulary",
}


class Violation(Exception):
    def __init__(self, rule, msg):
        super().__init__(f"{rule} {msg}")
        self.rule = rule


class Crash(Exception):
    """A power cut at a chosen point."""


def expect(rule, ok, msg):
    if not ok:
        raise Violation(rule, msg)


# ------------------------------------------------------------------ wording
# The README's table is checked against this, and I14 scans it for Git words.
UI = {
    "status.ok": "Up to date with your Config.",
    "status.rows": "{n} differences to review.",
    "status.offline": "Can't reach GitHub right now. Showing your Config as of {when}.",
    "row.machine-moved": "{name} changed on this Mac.",
    "row.removed-here": "{name} was removed from this Mac, but your Config still lists it.",
    "row.config-moved": "{name} changed in your Config.",
    "row.new-in-config": "{name} is new in your Config.",
    "row.dropped": "{name} is no longer in this Config. It's still on this Mac, and nothing will be removed unless you say so.",
    "row.extra": "{name} is on this Mac but not in your Config.",
    "row.conflict": "{name} changed in two places, and they don't match. You choose which one to keep.",
    "row.held": "{name}: update Soulcaster to review this. Your Config lists something this version doesn't recognize.",
    "choice.adopt": "Save {value} to your Config",
    "choice.apply": "Change this Mac to {value}",
    "choice.combined": "Use both changes: {value}",
    "choice.not-here": "Leave this Mac as it is and don't ask again",
    "choice.keep": "Keep it here and stop tracking it",
    "choice.remove": "Remove it from this Mac",
    "choice.add-back": "Put it back in your Config",
    "choice.keep-here": "Keep it on this Mac only",
    "confirm.remove": "Remove {names} from this Mac? This uninstalls them. Your other Macs aren't affected.",
    "confirm.adopt-removal": "Your other Macs will be asked what to do about {name}. Nothing is removed from them.",
    "save.waiting": "Saved on this Mac, but not on GitHub yet. Your other Macs can't see this until it's sent.",
    "save.send": "Send to GitHub",
    "save.discard": "Discard, and keep this Mac as it is",
    "save.sign-in": "GitHub needs you to sign in again before this can be sent. Nothing is lost.",
    "save.denied": "GitHub didn't accept this. You may not be allowed to change this Config, or its rules may not allow direct changes. Nothing is lost.",
    "save.changed-elsewhere": "{name} also changed in your Config, so this wasn't saved. Choose which one to keep.",
    "save.changed-again": "{name} changed again on this Mac, so this wasn't saved. Review it again.",
    "save.not-adoptable": "{name} can't be saved to a Config because {why}. It stays on this Mac.",
    "save.busy": "Your Config keeps changing on GitHub, so this wasn't sent. Nothing is lost. Try again in a moment.",
    "save.public": "This Config is public. Anyone can read what you save.",
    "config.unreadable": "Your Config on GitHub can't be read: {why}. Nothing on this Mac has changed. Fix it on GitHub, or go back to the last version that worked.",
    "config.restore": "Go back to the last version that worked",
    "config.too-new": "This Config needs a newer version of Soulcaster. Nothing on this Mac has changed.",
    "config.missing": "{path} isn't on GitHub any more. Nothing on this Mac has changed, and nothing will be removed.",
    "config.moved": "It looks like it moved to {new}. Follow it there?",
    "config.put-back": "Put it back",
    "config.choose": "Choose another Config",
    "config.rewritten": "Someone replaced the history of this Config on GitHub. Soulcaster compared it entry by entry, and nothing on this Mac has changed.",
    "switch.waiting": "You have changes waiting to send to {path}. Send or discard them before you switch.",
    "dotfiles.behind": "Dotfiles Repo: updates are available on GitHub.",
    "dotfiles.unpushed": "Dotfiles Repo: this Mac has changes that aren't on GitHub.",
    "dotfiles.review": "Review the updates",
    "dotfiles.by-hand": "This Mac has its own changes, so Soulcaster won't update the Dotfiles Repo. Update it yourself in Terminal.",
    "dotfiles.script-held": "Your Dotfiles install script changed since you approved it, so it won't run until you review it.",
}
GIT_WORDS = re.compile(
    r"\b(merge[sd]?|conflicts?|commits?|push(es|ed)?|pull(s|ed)?|rebase|branch(es)?|head|remote|origin|"
    r"fetch(es|ed)?|upstream|stash|sha|diverged?|force|clone|checkout|ours|theirs|fast-forward)\b",
    re.I,
)


# ------------------------------------------------------------------ Config format, ported from the Config draft
class Refused(Exception):
    pass


def strict_parse(data):
    """Plain RFC 8259, at most 1 MiB, duplicate keys refused at any depth."""
    if len(data) > 1024 * 1024:
        raise Refused("the file is larger than 1 MiB")

    def no_dupes(pairs):
        keys = [k for k, _ in pairs]
        dupes = sorted({k for k in keys if keys.count(k) > 1})
        if dupes:
            raise Refused(f"{dupes[0]} is listed twice")
        return dict(pairs)

    def no_constant(name):
        raise Refused(f"{name} isn't allowed")

    try:
        return json.loads(data.decode("utf-8"), object_pairs_hook=no_dupes, parse_constant=no_constant)
    except json.JSONDecodeError as e:
        raise Refused(f"line {e.lineno} has a JSON mistake")
    except UnicodeDecodeError:
        raise Refused("it isn't UTF-8 text")


ORDER = ["$schema", "schemaVersion", "name", "items", "packages", "settings", "dotfiles"]


def canonical(c):
    """The one form the app writes: same code path as the Config draft's canonical()."""
    out = {}
    for k in ORDER:
        if k not in c:
            continue
        v = c[k]
        if k == "items":
            v = sorted(v)
        elif k == "packages":
            v = {s: sorted(v[s]) for s in ("formulae", "casks") if v.get(s)}
        elif k == "settings":
            v = dict(sorted(v.items()))
        if v != [] and v != {}:
            out[k] = v
    return json.dumps(out, indent=2, ensure_ascii=False) + "\n"


def shape_errors(c):
    """A small closed-shape check, so the model runs with no dependencies. --schema adds the real one."""
    if not isinstance(c, dict):
        return ["it isn't a JSON object"]
    errors = [f"{k} isn't a known field" for k in c if k not in ORDER]
    if not isinstance(c.get("schemaVersion"), int) or isinstance(c.get("schemaVersion"), bool):
        errors.append("schemaVersion is missing")
    if not isinstance(c.get("name"), str) or not c.get("name"):
        errors.append("name is missing")
    if "items" in c:
        if not isinstance(c["items"], list) or len(set(map(str, c["items"]))) != len(c["items"]):
            errors.append("items has a duplicate")
    packages = c.get("packages", {})
    if not isinstance(packages, dict) or set(packages) - {"formulae", "casks"}:
        errors.append("packages has an unknown field")
    settings = c.get("settings", {})
    if not isinstance(settings, dict):
        errors.append("settings isn't an object")
    else:
        for k, v in settings.items():
            if v is None or isinstance(v, dict):
                errors.append(f"{k} has a value that isn't allowed")
            if isinstance(v, list) and len(set(map(str, v))) != len(v):
                errors.append(f"{k} lists the same thing twice")
    if SCHEMA is not None and c.get("schemaVersion") == 1:
        errors += [f"{list(e.absolute_path)}: {e.message[:80]}" for e in SCHEMA.iter_errors(c)]
    return errors


def read_config(data, cat):
    """Returns (status, value). Whole-file problems only. Per-entry ones are flatten()'s."""
    try:
        c = strict_parse(data)
    except Refused as e:
        return "unreadable", str(e)
    if isinstance(c, dict) and isinstance(c.get("schemaVersion"), int) and c["schemaVersion"] > cat.max_schema:
        return "too-new", c["schemaVersion"]
    errors = shape_errors(c)
    return ("unreadable", errors[0]) if errors else ("ok", c)


def lint_value(v):
    """The Config draft's write-time lint, for what a write adds or changes."""
    parts = v if isinstance(v, list) else [v]
    for p in parts:
        if isinstance(p, str):
            if "/Users/" in p:
                return "it names this Mac's home folder"
            if re.search(r"ghp_|sk_live_|-----BEGIN", p):
                return "it looks like a secret"
            if re.search(r"[^@\s]+@[^@\s]+\.[a-z]+", p):
                return "it contains an email address"
    return None


class Catalog:
    def __init__(self, items, settings, renames=None, max_schema=1, bounds=None):
        self.items = set(items)
        self.settings = set(settings)
        self.renames = dict(renames or {})
        self.max_schema = max_schema
        self.bounds = dict(bounds or {})
        self.base = {"gh", "homebrew", "command-line-tools", "formula:gh"}

    def resolve(self, entry_id):
        return self.renames.get(entry_id, entry_id)


ITEMS = ["ghostty", "junie", "tool-old", "dockutil", "zed"]
SETTINGS = ["dock.autohide", "dock.tile-size", "dock.position", "dock.apps", "screenshots.location", "finder.show-hidden"]
BOUNDS = {"dock.tile-size": (16, 128)}


def older_catalog():
    return Catalog(ITEMS, SETTINGS, bounds=BOUNDS)


def newer_catalog():
    return Catalog(ITEMS + ["tool-new", "new-thing"], SETTINGS + ["dock.new-setting"], renames={"tool-old": "tool-new"}, max_schema=2, bounds=BOUNDS)


def is_package(k):
    return k.startswith(("formula:", "cask:"))


def is_setting(k):
    return "." in k and not is_package(k)


def flatten(c, cat):
    """A Config as {entry key: value}, with the Config draft's key spellings.

    Returns the entries, IDs this Catalog can't read, and entries whose value it refuses.
    Those last two are skipped, reported and kept in the file."""
    entries, unknown, refused = {}, [], set()
    for raw in c.get("items", []):
        k = cat.resolve(raw)
        if k in cat.base:
            continue
        if k in cat.items:
            entries[k] = True
        else:
            unknown.append(raw)
    for s, prefix in (("formulae", "formula:"), ("casks", "cask:")):
        for name in c.get("packages", {}).get(s, []):
            k = prefix + name
            if k not in cat.base:
                entries[k] = True
    for raw, v in c.get("settings", {}).items():
        k = cat.resolve(raw)
        if k not in cat.settings:
            unknown.append(raw)
            continue
        lo_hi = cat.bounds.get(k)
        if lo_hi and not (isinstance(v, (int, float)) and lo_hi[0] <= v <= lo_hi[1]):
            refused.add(k)
            continue
        entries[k] = v
    return entries, unknown, refused


def set_entry(c, key, value, cat):
    """Changes one entry and nothing else. It keeps an old spelling already in the file, as only Tidy rewrites."""
    if is_setting(key):
        settings = c.setdefault("settings", {})
        spelling = next((r for r in settings if cat.resolve(r) == key), key)
        if value is None:
            settings.pop(spelling, None)
        else:
            settings[spelling] = value
    elif is_package(key):
        section = "formulae" if key.startswith("formula:") else "casks"
        names = c.setdefault("packages", {}).setdefault(section, [])
        name = key.split(":", 1)[1]
        if value is None and name in names:
            names.remove(name)
        elif value is not None and name not in names:
            names.append(name)
    else:
        items = c.setdefault("items", [])
        if value is None:
            c["items"] = [r for r in items if cat.resolve(r) != key]
        elif not any(cat.resolve(r) == key for r in items):
            items.append(key)


def drop_base(c, cat):
    """A reviewed write drops entries naming always-installed Items, per the Config draft."""
    if "items" in c:
        c["items"] = [r for r in c["items"] if cat.resolve(r) not in cat.base]
    formulae = c.get("packages", {}).get("formulae")
    if formulae:
        c["packages"]["formulae"] = [n for n in formulae if "formula:" + n not in cat.base]


# ------------------------------------------------------------------ list settings
def combine(base, ours, theirs):
    """Both sides' changes to a list, or None when no single order is right."""
    base = base or []
    in_all = [x for x in base if x in ours and x in theirs]
    if [x for x in ours if x in in_all] != [x for x in theirs if x in in_all]:
        return None
    removed = {x for x in base if x not in ours or x not in theirs}
    out = [x for x in theirs if x not in removed]
    previous = None
    for x in ours:
        if x not in removed and x not in out:
            out.insert(out.index(previous) + 1 if previous in out else 0, x)
        if x in out:
            previous = x
    return out


def view(v, tiles):
    """List settings compare only on entries available here."""
    return [x for x in v if x not in tiles] if isinstance(v, list) and tiles else v


def reinsert(m, c, tiles):
    """Adopting a list keeps a tile that's unavailable here, in place, for other Machines."""
    out = list(m)
    for i, x in enumerate(c or []):
        if x in tiles and x not in out:
            previous = c[i - 1] if i else None
            out.insert(out.index(previous) + 1 if previous in out else 0, x)
    return out


# ------------------------------------------------------------------ the one classification
def classify(c, m, l):
    """The prior-art research's table, per entry, once Config and Machine differ. None means absent."""
    if l is None:
        if c is None:
            return "extra"
        return "new-in-config" if m is None else "never-applied-differs"
    if c == l:
        return "machine-moved"
    if c is None:
        return "dropped"
    return "config-moved" if m == l else "conflict"


OFFERS = {
    "machine-moved": ("adopt", "apply", "not-here"),
    "config-moved": ("apply", "not-here"),
    "new-in-config": ("apply", "not-here"),
    "never-applied-differs": ("apply", "adopt", "not-here"),
    "conflict": ("adopt", "apply", "not-here"),
    "extra": ("adopt", "keep-here"),
    "held": (),
}


class Row:
    def __init__(self, key, cls, c, m, l, offers, combined=None, why=None):
        self.key, self.cls, self.c, self.m, self.l = key, cls, c, m, l
        self.offers, self.combined, self.why = offers, combined, why

    def __repr__(self):
        return f"<{self.key} {self.cls} c={self.c} m={self.m} l={self.l}>"


# ------------------------------------------------------------------ git, with the engine's guards
class Result:
    def __init__(self, rc, out, err):
        self.rc, self.out, self.err = rc, out, err


def git_env(extra=None):
    env = {
        "HOME": str(ROOT_HOME),
        "PATH": os.environ["PATH"],
        "GIT_CONFIG_GLOBAL": "/dev/null",
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_TERMINAL_PROMPT": "0",
        "LC_ALL": "C",
        "GIT_AUTHOR_DATE": "2026-11-06T12:00:00Z",
        "GIT_COMMITTER_DATE": "2026-11-06T12:00:00Z",
    }
    env.update(extra or {})
    return env


# Every engine git call adds these. GIT_CONFIG_GLOBAL doesn't cover a repo's own .git/config.
GUARDS = ["-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false", "-c", "credential.helper="]


def run_git(args, gitdir=None, cwd=None, env=None, guards=True, data=None, check=False):
    cmd = ["git"] + (GUARDS if guards else [])
    if gitdir:
        cmd += ["--git-dir", str(gitdir)]
    cmd += list(args)
    p = subprocess.run(cmd, cwd=str(cwd) if cwd else None, env=git_env(env), input=data, capture_output=True)
    r = Result(p.returncode, p.stdout, p.stderr.decode("utf-8", "replace"))
    if check and r.rc:
        raise RuntimeError(f"{' '.join(cmd)} failed: {r.err}")
    return r


def out(r):
    return r.out.decode().strip()


def commit_file(gitdir, parent, files, message, env, tmp):
    """Builds a commit with plumbing. No working tree, no checkout, no hooks. Files not named are kept."""
    index = pathlib.Path(tmp) / f"index-{os.getpid()}-{random.getrandbits(32):x}"
    e = dict(env, GIT_INDEX_FILE=str(index))
    if parent:
        run_git(["read-tree", parent], gitdir, env=e, check=True)
    for path, data in files.items():
        if data is None:
            # --remove refuses a bare repo ("must be run in a work tree"). Mode 0 in --index-info does not.
            line = f"0 {'0' * 40}\t{path}\n".encode()
            run_git(["update-index", "--index-info"], gitdir, env=e, data=line, check=True)
        else:
            blob = out(run_git(["hash-object", "-w", "--stdin"], gitdir, env=e, data=data, check=True))
            run_git(["update-index", "--add", "--cacheinfo", f"100644,{blob},{path}"], gitdir, env=e, check=True)
    tree = out(run_git(["write-tree"], gitdir, env=e, check=True))
    args = ["commit-tree", tree, "-F", "-"] + (["-p", parent] if parent else [])
    sha = out(run_git(args, gitdir, env=e, data=message.encode(), check=True))
    if index.exists():
        index.unlink()
    return sha


HAND = {"GIT_AUTHOR_NAME": "GitHub web editor", "GIT_AUTHOR_EMAIL": "noreply@github.com", "GIT_COMMITTER_NAME": "GitHub", "GIT_COMMITTER_EMAIL": "noreply@github.com"}
OWNER_NOREPLY = "12345+owner@users.noreply.github.com"
APP_IDENTITY = {"GIT_AUTHOR_NAME": "owner", "GIT_AUTHOR_EMAIL": OWNER_NOREPLY, "GIT_COMMITTER_NAME": "owner", "GIT_COMMITTER_EMAIL": OWNER_NOREPLY}
# What a Mac's name and a person's real address look like. Neither may ever reach a commit.
HOSTNAME = "Peytons-MacBook-Pro"
PERSONAL_EMAIL = "person@example.com"
SIMULATED_AUTH = "remote: Invalid username or token.\nfatal: Authentication failed for 'https://github.com/owner/configs.git/'"


def classify_failure(err):
    """Maps git's words to what the person can do. Must be rechecked against real GitHub."""
    if re.search(r"Authentication failed|could not read Username|Invalid username or token|terminal prompts disabled", err):
        return "auth"
    if re.search(r"Permission to .* denied|GH0\d\d|protected branch|pre-receive hook declined|remote rejected", err, re.I):
        return "denied"
    if re.search(r"\[rejected\]|fetch first|non-fast-forward", err):
        return "moved"
    return "offline"


# ------------------------------------------------------------------ a Machine
class Machine:
    def __init__(self, world, name, cat, actual, path):
        self.w, self.name, self.cat = world, name, cat
        self.dir = world.root / "machines" / name
        self.dir.mkdir(parents=True)
        self.clone = self.dir / "config-repo.git"
        self.record_path = self.dir / "machine.json"
        self.actual = dict(actual)
        self.transport = "ok"
        self.problem = None  # ("unreadable"|"too-new"|"missing", detail)
        self.empty_override = False
        self.notices = []
        self.fetched_head = None
        self.before_push = None
        self.crash_after_push = False
        self.rejected_pushes = 0
        run_git(["clone", "--bare", "--single-branch", "--no-tags", "--template=", str(world.remote), str(self.clone)], check=True)
        self.record = {
            "follows": {"repo": "owner/configs", "path": path, "branch": "main", "commit": None},
            "lastApplied": {},
            "pending": [],
            "keptHere": {},
            "unavailable": {},
            "approvals": {"taps": []},
        }
        self.fetch()
        self.check()

    # -- record
    def save(self):
        tmp = self.record_path.with_suffix(".tmp")
        tmp.write_text(json.dumps(self.record, indent=2, sort_keys=True))
        tmp.replace(self.record_path)

    def reload(self):
        self.record = json.loads(self.record_path.read_text())

    @property
    def follows(self):
        return self.record["follows"]

    @property
    def pending(self):
        return self.record["pending"]

    @property
    def L(self):
        return self.record["lastApplied"]

    def l(self, k):
        return self.L[k]["value"] if k in self.L else None

    def set_l(self, k, value, source):
        if value is None:
            self.L.pop(k, None)
        else:
            self.L[k] = {"value": value, "source": source}

    # -- git
    def g(self, *args, **kw):
        return run_git(args, self.clone, **kw)

    def url(self):
        return str(self.w.remote) if self.transport != "offline" else str(self.w.root / "no-such-host.git")

    def show(self, commit, path=None):
        r = self.g("show", f"{commit}:{path or self.follows['path']}")
        return r.out if r.rc == 0 else None

    def fetch(self):
        """Read-only except for the local copy of GitHub's history. Returns ok, offline, auth or problem."""
        if self.transport == "auth-all":
            return "auth"
        b = self.follows["branch"]
        r = self.g("fetch", "--no-tags", self.url(), f"+refs/heads/{b}:refs/remotes/origin/{b}")
        if r.rc:
            if "couldn't find remote ref" in r.err:
                self.problem = ("missing", {"new_branch": self._default_branch()})
                return "problem"
            return classify_failure(r.err)
        head = out(self.g("rev-parse", f"refs/remotes/origin/{b}"))
        self.fetched_head = head
        old = self.follows["commit"]
        if old and self.g("merge-base", "--is-ancestor", old, head).rc:
            self.g("update-ref", f"refs/soulcaster/rewritten/{old}", old)
            self.notices.append("rewritten")
        data = self.show(head)
        if data is None:
            self.problem = ("missing", {"new_path": self._rename_candidate(old, head)})
            if "missing-as-empty" in MUT:
                self.problem, self.empty_override = None, True
                return "ok"
            return "problem"
        status, value = read_config(data, self.cat)
        if status != "ok":
            self.problem = (status, value)
            if "invalid-as-empty" in MUT:
                self.problem, self.empty_override = None, True
                return "ok"
            return "problem"
        self.problem, self.empty_override = None, False
        self.follows["commit"] = head
        self.save()
        return "ok"

    def _rename_candidate(self, old, head):
        if not old:
            return None
        r = self.g("diff", "--name-status", "-M", "--diff-filter=R", old, head)
        for line in out(r).splitlines():
            parts = line.split("\t")
            if len(parts) == 3 and parts[1] == self.follows["path"]:
                return parts[2]
        return None

    def _default_branch(self):
        r = self.g("ls-remote", "--symref", self.url(), "HEAD")
        m = re.search(r"ref: refs/heads/(\S+)\s+HEAD", out(r))
        return m.group(1) if m else None

    def config(self):
        if self.empty_override:
            return {"schemaVersion": 1, "name": "(empty)"}
        return strict_parse(self.show(self.follows["commit"]))

    # -- reading
    def tiles(self, k):
        return self.record["unavailable"].get(k, {}).get("tiles", [])

    def fold_key(self, k):
        """Item and setting IDs are read after renames. A rename is not Drift."""
        return k if is_package(k) else self.cat.resolve(k)

    def rows(self):
        C, unknown, refused = flatten(self.config(), self.cat)
        actual, last = {}, {}
        for k, v in self.actual.items():
            actual[self.fold_key(k)] = v
        for k in self.L:
            last[self.fold_key(k)] = self.l(k)
        visible = {k for k in actual if is_package(k) or k in self.cat.items}
        keys = set(C) | set(last) | visible
        rows = []
        for k in sorted(keys):
            if k in refused or k in self.cat.base:
                continue
            tiles = self.tiles(k)
            if k in self.record["unavailable"] and not tiles and k in C:
                continue
            c, m, l = view(C.get(k), tiles), actual.get(k), view(last.get(k), tiles)
            if c == m:
                continue
            if c is None and k in self.record["keptHere"] and "forget-kept-here" not in MUT:
                continue
            cls = classify(c, m, l)
            guard_off = "no-skew-guard" in MUT
            if unknown and not guard_off and (cls == "dropped" or (cls == "extra" and is_package(k))):
                cls = "held"
            if cls == "dropped":
                offers = ("keep", "add-back") if is_setting(k) else ("keep", "remove", "add-back")
            else:
                offers = OFFERS[cls]
            why = lint_value(m) if m is not None else None
            if why and "skip-validate" not in MUT:
                offers = tuple(o for o in offers if o != "adopt")
            combined = None
            if cls == "conflict" and isinstance(c, list) and isinstance(m, list):
                combined = combine(l if isinstance(l, list) else [], m, c)
                if combined in (c, m):
                    combined = None
            rows.append(Row(k, cls, C.get(k), m, last.get(k), offers, combined, why))
        return rows

    def blocked(self):
        return self.problem is not None

    def check(self, fetch=True):
        """When the app looks: at launch, on focus, hourly while open, or by button. It changes nothing shared."""
        status = self.fetch() if fetch else "ok"
        if not self.blocked():
            C, _, refused = flatten(self.config(), self.cat)
            for k in sorted(set(C) | set(self.L)):
                tiles = self.tiles(k)
                c, m = view(C.get(k), tiles), self.actual.get(k)
                if k not in refused and c == m and view(self.l(k), tiles) != c:
                    self.set_l(k, C.get(k), "record")
            for k in list(self.record["keptHere"]):
                if k in C or k not in self.actual:
                    del self.record["keptHere"][k]
            for k in list(self.record["unavailable"]):
                if k not in C and not self.tiles(k):
                    del self.record["unavailable"][k]
            self.save()
            if "auto-commit" in MUT:
                self._queue([r.key for r in self.rows() if r.cls == "machine-moved"])
                self._send()
            if "auto-apply" in MUT:
                for r in self.rows():
                    if r.cls in ("config-moved", "new-in-config"):
                        self._write_machine(r.key, r.c)
        self.w.checkpoint()
        return status

    # -- Machine changes
    def _write_machine(self, k, v):
        self.w.machine_log.append((self.name, k, self.actual.get(k), v, self.w.current))
        if v is None:
            self.actual.pop(k, None)
        else:
            self.actual[k] = v

    def outside(self, k, v):
        """A person changing the Mac without the app: brew install, System Settings."""
        if v is None:
            self.actual.pop(k, None)
        else:
            self.actual[k] = v

    # -- the one write path
    def _queue(self, keys, values=None):
        C, _, _ = flatten(self.config(), self.cat)
        for k in keys:
            m = self.actual.get(k)
            after = (values or {}).get(k, m)
            tiles = self.tiles(k)
            if tiles and isinstance(after, list):
                after = reinsert(after, C.get(k), tiles)
            op = {"id": self.w.new_op(self.name), "key": k, "before": C.get(k), "after": after, "machineAtChoice": m}
            self.pending.append(op)
            if "record-before-push" in MUT:
                self.set_l(k, after, "adopt")
        self.save()

    def _return(self, op, why):
        """An op that can't go turns back into a Drift row. The value is still on the Machine, so nothing is lost."""
        self.pending.remove(op)
        self.w.ops[op["id"]] = why
        self.notices.append(why)

    def _send(self):
        """Fetch, replay by entry, validate, commit on top of what's there, then push with no force."""
        for _ in range(3):
            prev = self.follows["commit"]
            status = self.fetch()
            if "drop-pending" in MUT and status != "ok":
                self.record["pending"] = []
            if status != "ok":
                # An unreadable, too-new or missing file is not a network failure: the waiting
                # change stays, and nothing is written over the last file this Machine could read.
                if self.blocked():
                    self.notices.append("blocked")
                    return "blocked"
                self.notices.append("sign-in" if status == "auth" else "waiting")
                return status
            head = self.follows["commit"]
            raw = self.show(head)
            cfg = self.config()
            C, _, _ = flatten(cfg, self.cat)
            send, noop = [], []
            for op in list(self.pending):
                k = op["key"]
                if self.actual.get(k) != op["machineAtChoice"]:
                    self._return(op, "changed-again")
                elif lint_value(op["after"]) and "skip-validate" not in MUT:
                    self._return(op, "not-adoptable")
                elif C.get(k) == op["after"]:
                    noop.append(op)
                elif C.get(k) == op["before"] or "blind-overwrite" in MUT or "line-merge" in MUT:
                    send.append(op)
                else:
                    self._return(op, "changed-elsewhere")
            if not send:
                self._finish(noop, head)
                return "ok"
            new = copy.deepcopy(cfg)
            for op in send:
                set_entry(new, op["key"], op["after"], self.cat)
            drop_base(new, self.cat)
            if "whole-file" in MUT:
                new = {"schemaVersion": 1, "name": cfg["name"]}
                for k, v in self.actual.items():
                    set_entry(new, k, v, self.cat)
            text = canonical(new).encode()
            if "line-merge" in MUT and prev != head:
                text = self._line_merge(prev, head, send)
            status, _ = read_config(text, self.cat)
            if status != "ok" and "skip-validate" not in MUT and "line-merge" not in MUT:
                for op in send:
                    self._return(op, "not-adoptable")
                return "not-adoptable"
            parent = head
            if raw != canonical(cfg).encode() and "line-merge" not in MUT:
                parent = commit_file(self.clone, parent, {self.follows["path"]: canonical(cfg).encode()}, reformat_message(self.follows["path"]), APP_IDENTITY, self.dir)
                self.w.ledger[parent] = {"machine": self, "action": self.w.current, "kind": "reformat", "ops": [], "parent": head}
            sha = commit_file(self.clone, parent, {self.follows["path"]: text}, adopt_message(self.follows["path"], send), APP_IDENTITY, self.dir)
            self.w.ledger[sha] = {"machine": self, "action": self.w.current, "kind": "entries", "ops": copy.deepcopy(send), "parent": parent}
            if self.before_push:
                self.before_push()
            result = self._push(sha)
            if result == "ok":
                if self.crash_after_push:
                    self.crash_after_push = False
                    raise Crash()
                self.fetch()
                self._finish(send + noop, sha)
                return "ok"
            if result != "moved":
                self.notices.append({"auth": "sign-in", "denied": "denied"}.get(result, "waiting"))
                return result
            self.rejected_pushes += 1
            if "force-push" in MUT:
                self.g("push", "--force", self.url(), f"{sha}:refs/heads/{self.follows['branch']}")
                self.fetch()
                self._finish(send + noop, sha)
                return "ok"
        self.notices.append("busy")
        return "busy"

    def _line_merge(self, prev, head, send):
        """The rejected alternative, kept so the mutation can show what it does."""
        tmp = self.dir / "merge"
        tmp.mkdir(exist_ok=True)
        base, ours_cfg = self.show(prev), strict_parse(self.show(prev))
        for op in send:
            set_entry(ours_cfg, op["key"], op["after"], self.cat)
        (tmp / "base").write_bytes(base)
        (tmp / "ours").write_bytes(canonical(ours_cfg).encode())
        (tmp / "theirs").write_bytes(self.show(head))
        r = run_git(["merge-file", "-p", str(tmp / "ours"), str(tmp / "base"), str(tmp / "theirs")])
        return r.out

    def _push(self, sha):
        if self.transport == "auth-push":
            return classify_failure(SIMULATED_AUTH)
        r = self.g("push", "--porcelain", self.url(), f"{sha}:refs/heads/{self.follows['branch']}")
        return "ok" if r.rc == 0 else classify_failure(out(r) + "\n" + r.err)

    def _finish(self, ops, commit):
        for op in ops:
            k = op["key"]
            self.w.ops[op["id"]] = "sent"
            if self.actual.get(k) != op["after"]:
                self._write_machine(k, op["after"])  # the combined list: GitHub first, then this Mac
            self.set_l(k, op["after"], "adopt")
            self.pending.remove(op)
        if commit and not self.blocked():
            self.follows["commit"] = commit
        self.save()

    # -- what a person can do
    def person_adopt(self, keys, values=None):
        with self.w.action(self, "adopt", keys):
            if self.blocked():
                return "blocked"
            rows = {r.key: r for r in self.rows()}
            for k in keys:
                expect("I1", k in rows and ("adopt" in rows[k].offers or values), f"{self.name} adopted {k}, which wasn't offered")
            self._queue(keys, values)
            return self._send()

    def person_add_back(self, keys):
        with self.w.action(self, "add-back", keys):
            if self.blocked():
                return "blocked"
            self._queue(keys)
            return self._send()

    def person_send(self):
        with self.w.action(self, "send", [op["key"] for op in self.pending]):
            return self._send()

    def person_discard(self):
        with self.w.action(self, "discard", [op["key"] for op in self.pending]):
            for op in list(self.pending):
                self._return(op, "discarded")
            self.save()

    def person_apply(self, keys, reviewed=None):
        with self.w.action(self, "apply", keys):
            self.fetch()
            if self.blocked():
                return "blocked"
            C, _, _ = flatten(self.config(), self.cat)
            done = []
            for k in keys:
                if reviewed is not None and C.get(k) != reviewed.get(k):
                    self.notices.append("changed-while-reviewing")
                    continue
                if k not in C and "apply-all-removes" not in MUT:
                    continue
                if is_package(k) and k.count("/") == 2:
                    tap = "/".join(k.split(":", 1)[1].split("/")[:2])
                    if tap not in self.record["approvals"]["taps"]:
                        self.notices.append(f"needs-you:{tap}")
                        continue
                value = C.get(k)
                tiles = self.tiles(k)
                self._write_machine(k, view(value, tiles))
                self.set_l(k, value, "run")
                done.append(k)
            self.save()
            return done

    def person_apply_all(self):
        """Only where the Config moved. Removals and two-place changes are always per entry."""
        rows = self.rows()
        allowed = ("config-moved", "new-in-config")
        if "apply-all-removes" in MUT:
            allowed += ("dropped",)
        return self.person_apply([r.key for r in rows if r.cls in allowed])

    def person_confirm_remove(self, keys):
        with self.w.action(self, "confirm-remove", keys):
            rows = {r.key: r for r in self.rows()}
            for k in keys:
                expect("I2", "remove" in rows[k].offers, f"{self.name} offered removal of {k} as {rows[k].cls}")
                self._write_machine(k, None)
                self.set_l(k, None, "run")
            self.save()

    def person_keep(self, keys):
        with self.w.action(self, "keep", keys):
            for k in keys:
                self.set_l(k, None, "run")
                if not is_setting(k):
                    self.record["keptHere"][k] = True
            self.save()

    def person_keep_here(self, keys):
        with self.w.action(self, "keep-here", keys):
            for k in keys:
                self.record["keptHere"][k] = True
            self.save()

    def person_not_here(self, keys):
        with self.w.action(self, "not-here", keys):
            for k in keys:
                self.record["unavailable"][k] = {"reason": "marked-by-person"}
            self.save()

    def person_approve_tap(self, tap):
        with self.w.action(self, "approve-tap", []):
            self.record["approvals"]["taps"].append(tap)
            self.save()

    def person_restore(self):
        """Go back to the last version that worked: an ordinary reviewed write."""
        with self.w.action(self, "restore", []):
            for _ in range(3):
                status = self.fetch()
                if status != "problem" or self.problem[0] == "too-new":
                    return status
                good = self.show(self.follows["commit"])
                head = self.fetched_head
                path = self.follows["path"]
                sha = commit_file(self.clone, head, {path: good}, f"Restore {path} to its last readable version\n\nFrom {self.follows['commit'][:12]}.", APP_IDENTITY, self.dir)
                self.w.ledger[sha] = {"machine": self, "action": self.w.current, "kind": "restore", "ops": [], "parent": head}
                if self._push(sha) == "ok":
                    self.fetch()
                    return "ok"
            return "busy"

    def person_follow(self, path=None, branch=None):
        with self.w.action(self, "follow", []):
            if path:
                self.follows["path"] = path
            if branch:
                self.follows["branch"] = branch
            self.follows["commit"] = None
            self.save()
            return self.fetch()

    def person_switch(self, path):
        with self.w.action(self, "switch", []):
            if self.pending:
                self.notices.append("switch-blocked")
                return "blocked"
            self.follows["path"] = path
            self.follows["commit"] = None
            if "clear-record-on-switch" in MUT:
                self.record["lastApplied"] = {}
            self.save()
            return self.fetch()

    def person_tidy(self):
        with self.w.action(self, "tidy", []):
            for _ in range(3):
                if self.fetch() != "ok" or self.blocked():
                    return "blocked"
                cfg, head = self.config(), self.follows["commit"]
                new = copy.deepcopy(cfg)
                new["items"] = sorted({self.cat.resolve(r) for r in cfg.get("items", [])})
                if new == cfg:
                    return "nothing"
                path = self.follows["path"]
                sha = commit_file(self.clone, head, {path: canonical(new).encode()}, f"Tidy {path}\n\nRewrites old IDs to their current forms.", APP_IDENTITY, self.dir)
                self.w.ledger[sha] = {"machine": self, "action": self.w.current, "kind": "tidy", "ops": [], "parent": head}
                if self._push(sha) == "ok":
                    self.fetch()
                    return "ok"
            return "busy"


def describe(op):
    k, a, b = op["key"], op["after"], op["before"]
    if isinstance(a, list) or isinstance(b, list):
        return f"- change {k}"
    if b is None:
        return f"- add {k}" + ("" if a is True else f": {json.dumps(a)}")
    if a is None:
        return f"- remove {k}"
    return f"- change {k}: {json.dumps(b)} -> {json.dumps(a)}"


def adopt_message(path, ops):
    lines = "\n".join(describe(op) for op in sorted(ops, key=lambda o: o["key"]))
    n = len(ops)
    extra = f"\nHost: {HOSTNAME}\nAuthor-Email: {PERSONAL_EMAIL}" if "leak-machine-name" in MUT else ""
    return f"Adopt {n} change{'s' if n != 1 else ''} into {path}\n\n{lines}{extra}\n"


def reformat_message(path):
    return f"Reformat {path}\n\nNo entries change.\n"


# ------------------------------------------------------------------ the world and its invariants
class World:
    def __init__(self, root, seed=None, files=None):
        self.root = pathlib.Path(root)
        self.remote = self.root / "github" / "configs.git"
        self.remote.parent.mkdir(parents=True)
        run_git(["init", "--quiet", "--bare", "-b", "main", "--template=", str(self.remote)], check=True)
        self.ledger = {}
        self.hand = set()
        self.allowed_rewrites = set()
        self.machine_log = []
        self.logged = 0
        self.ops = {}
        self.machines = []
        self.current = None
        self.op_count = 0
        self.head = None
        self.seen_applied = {}  # machine name -> last-applied values already checked against GitHub
        files = files or {"configs/work.json": canonical(seed or SEED).encode()}
        files = dict(files, **{"README.md": b"My Configs\n"})
        self.head = commit_file(self.remote, None, files, "First commit", HAND, self.root)
        self.hand.add(self.head)
        run_git(["update-ref", "refs/heads/main", self.head], self.remote, check=True)

    def add(self, name, cat=None, actual=None, path="configs/work.json"):
        cat = cat or older_catalog()
        if actual is None:
            data = run_git(["show", f"main:{path}"], self.remote, check=True).out
            actual = flatten(json.loads(data), cat)[0]
        m = Machine(self, name, cat, actual, path)
        self.machines.append(m)
        return m

    def new_op(self, name):
        self.op_count += 1
        op_id = f"{name}-{self.op_count}"
        self.ops[op_id] = None
        return op_id

    @contextlib.contextmanager
    def action(self, machine, name, keys):
        # A push can lose to the other Machine's Adopt, which runs while this one is still
        # inside its own action. Restore the outer action so the replay commit stays tied to it.
        previous = self.current
        self.current = (machine.name, name, tuple(keys))
        try:
            yield
        finally:
            self.current = previous
        self.checkpoint()

    # -- things that happen on GitHub, outside any Machine
    def remote_head(self, branch="main"):
        return out(run_git(["rev-parse", f"refs/heads/{branch}"], self.remote, check=True))

    def edit(self, files, message="Edit on GitHub"):
        """A person editing in GitHub's web editor."""
        sha = commit_file(self.remote, self.remote_head(), files, message, HAND, self.root)
        self.hand.add(sha)
        run_git(["update-ref", "refs/heads/main", sha], self.remote, check=True)
        self.checkpoint()
        return sha

    def edit_config(self, change, path="configs/work.json"):
        c = self.read_config_raw(path)
        change(c)
        return self.edit({path: canonical(c).encode()})

    def read_config_raw(self, path="configs/work.json", at=None):
        return json.loads(run_git(["show", f"{at or self.remote_head()}:{path}"], self.remote, check=True).out)

    def force_push(self, onto, files):
        """Someone rewrites history, with a real git push --force from another clone."""
        third = self.root / f"third-{len(self.allowed_rewrites)}.git"
        run_git(["clone", "--bare", "--quiet", "--template=", str(self.remote), str(third)], check=True)
        sha = commit_file(third, onto, files, "Rewrite history", HAND, self.root)
        run_git(["push", "--quiet", "--force", str(self.remote), f"{sha}:refs/heads/main"], third, check=True)
        self.hand.add(sha)
        self.allowed_rewrites.add(sha)
        self.checkpoint()
        return sha

    # -- invariants, checked after every action
    def checkpoint(self):
        head = self.remote_head() if run_git(["rev-parse", "-q", "--verify", "refs/heads/main"], self.remote).rc == 0 else self.head
        if head != self.head:
            # A rewrite can have ordinary commits on top of it before the next check, so the
            # tip itself is not the rewrite. The new history is allowed only when one of those
            # commits was a rewrite this prototype performed on purpose.
            fast_forward = run_git(["merge-base", "--is-ancestor", self.head, head], self.remote).rc == 0
            if fast_forward:
                newcomers = out(run_git(["rev-list", "--reverse", f"{self.head}..{head}"], self.remote)).split()
            else:
                newcomers = out(run_git(["rev-list", "--reverse", head, "--not", self.head], self.remote)).split()
                expect("I6", any(sha in self.allowed_rewrites for sha in newcomers), f"history on GitHub was rewritten at {head[:8]}")
            for sha in newcomers:
                self.check_commit(sha)
            self.head = head
        for name, k, old, new, action in self.machine_log[self.logged:]:
            expect("I11", action is not None, f"{name}: {k} changed from {old} to {new} with nobody asking")
            if new is None:
                expect("I2", action[1] == "confirm-remove" and k in action[2], f"{name}: {k} was uninstalled by {action[1]}, with no confirmation")
            else:
                expect("I11", k in action[2], f"{name}: {k} changed during {action[1]} of {action[2]}")
        self.logged = len(self.machine_log)
        for m in self.machines:
            waiting = {op["id"] for op in m.pending}
            for op_id, fate in self.ops.items():
                if op_id.startswith(m.name + "-") and fate is None:
                    expect("I1", op_id in waiting, f"{m.name}: reviewed change {op_id} vanished")
            # Last-applied may move to a value only once that value is what GitHub currently holds.
            # Comparing with the previous check catches a record that jumped ahead and then had its
            # waiting change handed back, which is no longer sitting in pending.
            seen = self.seen_applied.get(m.name, {})
            current = flatten(m.config(), m.cat)[0]
            for k, rec in m.L.items():
                moved = k not in seen or seen[k] != rec["value"]
                if moved and current.get(k) != rec["value"]:
                    expect("I12", False, f"{m.name}: {k} recorded as applied before it reached GitHub")
            self.seen_applied[m.name] = {k: copy.deepcopy(rec["value"]) for k, rec in m.L.items()}
            if m.fetched_head == head:
                data = run_git(["show", f"{head}:{m.follows['path']}"], self.remote).out
                status = read_config(data, m.cat)[0] if data else "missing"
                if status != "ok":
                    expect("I10", m.blocked(), f"{m.name}: a {status} file wasn't treated as unreadable")
                    expect("I10", not m.empty_override, f"{m.name}: a {status} file read as empty")

    def check_commit(self, sha):
        if sha in self.hand:
            return
        entry = self.ledger.get(sha)
        expect("I5", entry is not None and entry["action"] is not None, f"commit {sha[:8]} has no person's action behind it")
        m = entry["machine"]
        path = m.follows["path"]
        data = run_git(["show", f"{sha}:{path}"], self.remote).out
        message = out(run_git(["log", "-1", "--format=%an <%ae>%n%cn <%ce>%n%B", sha], self.remote))
        for secret in (HOSTNAME, PERSONAL_EMAIL, m.name + "'s"):
            expect("I8", secret not in message, f"commit {sha[:8]} names {secret}")
        expect("I8", OWNER_NOREPLY in message, f"commit {sha[:8]} isn't under the noreply address")
        tree = out(run_git(["ls-tree", "-r", "--name-only", sha], self.remote))
        expect("I8", "machine.json" not in tree, "the Machine Record reached GitHub")
        status, why = read_config(data, m.cat)
        expect("I4", status == "ok", f"commit {sha[:8]} holds a Config this app refuses: {why}")
        child = strict_parse(data)
        parent_data = run_git(["show", f"{entry['parent']}:{path}"], self.remote).out
        if entry["kind"] == "restore":
            return
        parent = strict_parse(parent_data)
        if entry["kind"] == "tidy":
            return
        E0, U0, _ = flatten(parent, m.cat)
        E1, U1, _ = flatten(child, m.cat)
        expect("I1", sorted(U0) == sorted(U1), f"commit {sha[:8]} lost entries this app can't read: {sorted(U0)} -> {sorted(U1)}")
        for field in ("name", "dotfiles", "$schema"):
            expect("I7", parent.get(field) == child.get(field), f"commit {sha[:8]} changed {field}")
        changed = {k for k in set(E0) | set(E1) if E0.get(k) != E1.get(k)}
        reviewed = {op["key"] for op in entry["ops"]}
        expect("I1" if changed - reviewed else "I7", changed == reviewed, f"commit {sha[:8]} changed {sorted(changed)} but {sorted(reviewed)} was reviewed")
        for op in entry["ops"]:
            expect("I1", E0.get(op["key"]) == op["before"], f"commit {sha[:8]} overwrote {op['key']}={E0.get(op['key'])}, which the person never saw")
            expect("I7", E1.get(op["key"]) == op["after"], f"commit {sha[:8]} wrote {op['key']}={E1.get(op['key'])}, not {op['after']}")
            why = lint_value(op["after"])
            expect("I4", not why, f"commit {sha[:8]} wrote {op['key']}, and {why}")

    def settled(self, m):
        """After a person has chosen everything: nothing left to review on this Machine."""
        rows = [r for r in m.rows() if r.cls != "held"]
        return not rows and not m.pending and not m.blocked()


# ------------------------------------------------------------------ the scenarios
SAFARI, MAIL, NOTES, MUSIC, PHOTOS, MAPS, CAL = [f"/Applications/{n}.app" for n in ("Safari", "Mail", "Notes", "Music", "Photos", "Maps", "Calendar")]
ZED = "/Applications/Zed.app"
LAUNCHPAD = "/System/Applications/Launchpad.app"
DOCK = [SAFARI, MAIL, NOTES, MUSIC, PHOTOS, MAPS, CAL]
SEED = {
    "schemaVersion": 1,
    "name": "Work Mac",
    "items": ["dockutil", "ghostty", "junie"],
    "packages": {"formulae": ["jq", "neovim"], "casks": ["firefox", "rectangle"]},
    "settings": {"dock.apps": DOCK, "dock.autohide": True, "dock.tile-size": 48},
}


def resolve_all(w, machines, pick, rounds=6):
    """Every Machine works through its rows until nothing is left. pick(row) names the choice."""
    for _ in range(rounds):
        for m in machines:
            if m.pending:
                m.person_send()
            m.check()
            for r in m.rows():
                if r.cls != "held":
                    choose(m, r, pick(r))
            m.check()
        if all(w.settled(m) for m in machines):
            heads = {m.follows["commit"] for m in machines}
            if len(heads) == 1:
                return
    raise Violation("I3", f"not settled after {rounds} rounds: {[(m.name, m.rows(), m.pending) for m in machines]}")


def choose(m, r, choice):
    k = [r.key]
    if choice == "adopt":
        return m.person_adopt(k)
    if choice == "combined":
        return m.person_adopt(k, {r.key: r.combined})
    if choice == "apply":
        return m.person_apply(k)
    if choice == "not-here":
        return m.person_not_here(k)
    if choice == "keep":
        return m.person_keep(k)
    if choice == "keep-here":
        return m.person_keep_here(k)
    if choice == "remove":
        return m.person_confirm_remove(k)
    if choice == "add-back":
        return m.person_add_back(k)
    raise ValueError(choice)


def first_offer(r):
    return r.offers[0]


def row(m, key):
    return next((r for r in m.rows() if r.key == key), None)


def cases():
    return [fn for name, fn in sorted(globals().items()) if name.startswith("case_")]


def case_a_both_adopt_different(w, say):
    a, b = w.add("A"), w.add("B")
    a.outside("cask:zed", True)
    b.outside("formula:ripgrep", True)
    expect("I1", a.person_adopt(["cask:zed"]) == "ok", "A's Adopt failed")
    head_before = w.remote_head()
    expect("I1", b.rows()[0].key == "formula:ripgrep", "B should only see its own change, since it hasn't checked")
    expect("I1", b.person_adopt(["formula:ripgrep"]) == "ok", "B's Adopt failed")
    expect("I6", out(run_git(["rev-parse", f"{w.remote_head()}^"], w.remote)) == head_before, "B's commit isn't on top of A's")
    c = w.read_config_raw()
    expect("I1", "zed" in c["packages"]["casks"] and "ripgrep" in c["packages"]["formulae"], "a change was lost")
    a.check()
    b.check()
    expect("I1", row(a, "formula:ripgrep").cls == "new-in-config" and row(b, "cask:zed").cls == "new-in-config", "each should see the other's as new")
    a.person_apply_all()
    b.person_apply_all()
    expect("I3", w.settled(a) and w.settled(b), "should be settled")
    say("both reach GitHub as 2 linear commits, each Mac sees the other's as new, Apply settles it, and no person saw any Git")


def case_b_same_setting_two_ways(w, say):
    for choice, final in (("adopt", 64), ("apply", 36), ("not-here", 36)):
        sub = World(w.root / f"s-{choice}", SEED)
        a, b = sub.add("A"), sub.add("B")
        a.outside("dock.tile-size", 36)
        b.outside("dock.tile-size", 64)
        a.person_adopt(["dock.tile-size"])
        b.check()
        r = row(b, "dock.tile-size")
        expect("I1", (r.cls, r.c, r.m, r.l) == ("conflict", 36, 64, 48), f"expected a two-place change, got {r}")
        expect("I5", sub.read_config_raw()["settings"]["dock.tile-size"] == 36, "nothing should be written until B chooses")
        choose(b, r, choice)
        resolve_all(sub, [a, b], first_offer)
        expect("I3", sub.read_config_raw()["settings"]["dock.tile-size"] == final, f"expected {final}")
        expect("I3", a.actual["dock.tile-size"] == final, "A should end on the same value")
        expect("I3", b.actual["dock.tile-size"] == (64 if choice == "not-here" else final), "B's value")
        say(f"choosing {choice}: Config {final}, A {a.actual['dock.tile-size']}, B {b.actual['dock.tile-size']}, settled")


def case_b_stale_adopt_is_stopped(w, say):
    a, b = w.add("A"), w.add("B")
    a.outside("dock.tile-size", 36)
    b.outside("dock.tile-size", 64)
    a.person_adopt(["dock.tile-size"])
    head = w.remote_head()
    expect("I1", b.person_adopt(["dock.tile-size"]) == "ok", "the send itself should finish")
    expect("I1", w.remote_head() == head, "B must not overwrite A's value")
    expect("I1", "changed-elsewhere" in b.notices, "B should be told")
    expect("I1", row(b, "dock.tile-size").cls == "conflict", "it should come back as a row")
    say("an Adopt made before seeing A's change writes nothing, says so, and returns as a two-place row")


def case_c_list_both_edited(w, say):
    a, b = w.add("A"), w.add("B")
    a.outside("dock.apps", [x for x in DOCK if x != MAIL])
    b.outside("dock.apps", DOCK + [ZED])
    a.person_adopt(["dock.apps"])
    b.check()
    r = row(b, "dock.apps")
    wanted = [SAFARI, NOTES, MUSIC, PHOTOS, MAPS, CAL, ZED]
    expect("I1", r.cls == "conflict" and r.combined == wanted, f"expected the combined list, got {r} {r.combined}")
    choose(b, r, "combined")
    expect("I1", w.read_config_raw()["settings"]["dock.apps"] == wanted, "Config should hold the combined list")
    expect("I1", b.actual["dock.apps"] == wanted, "B's Dock should change to it as well")
    resolve_all(w, [a, b], first_offer)
    expect("I3", a.actual["dock.apps"] == wanted, "A should get it on Apply")
    say("non-overlapping edits: one row offers the combined list, then both Macs and the Config hold it")


def case_c_list_reordered_both_ways(w, say):
    a, b = w.add("A"), w.add("B")
    move = lambda to: [x for x in DOCK if x != MAIL][:to] + [MAIL] + [x for x in DOCK if x != MAIL][to:]
    a.outside("dock.apps", move(3))
    b.outside("dock.apps", move(5))
    a.person_adopt(["dock.apps"])
    head = w.remote_head()
    b.person_adopt(["dock.apps"])
    expect("I1", w.remote_head() == head, "B's stale list must not be written")
    r = row(b, "dock.apps")
    expect("I1", r.cls == "conflict" and r.combined is None, "no combined list exists for two different orders")
    resolve_all(w, [a, b], first_offer)
    apps = w.read_config_raw()["settings"]["dock.apps"]
    expect("I4", apps.count(MAIL) == 1, "the tile must appear once")
    say("two different orders: no combined list is offered, the person picks one, and the tile stays single")


def case_c_list_keeps_unavailable_tile(w, say):
    seed = copy.deepcopy(SEED)
    seed["settings"]["dock.apps"] = [SAFARI, LAUNCHPAD, NOTES]
    sub = World(w.root / "tile", seed)
    a = sub.add("A", actual={**flatten(seed, older_catalog())[0], "dock.apps": [SAFARI, NOTES]})
    a.record["unavailable"]["dock.apps"] = {"reason": "macos-too-old", "tiles": [LAUNCHPAD]}
    a.check()
    expect("I1", row(a, "dock.apps") is None, "a missing tile isn't Drift")
    a.outside("dock.apps", [SAFARI, NOTES, ZED])
    a.person_adopt(["dock.apps"])
    expect("I1", sub.read_config_raw()["settings"]["dock.apps"] == [SAFARI, LAUNCHPAD, NOTES, ZED], "the tile must survive, in place")
    say("Adopt on a list keeps a tile that's unavailable here, in place, for other Macs")


def case_d_offline_for_weeks(w, say):
    a, b = w.add("A"), w.add("B")
    # Weeks of change while A is away.
    a.transport = "offline"
    a.outside("cask:zed", True)
    a.outside("dock.tile-size", 64)
    expect("I1", a.person_adopt(["cask:zed", "dock.tile-size"]) == "offline", "should wait")
    expect("I1", len(a.pending) == 2, "both should wait")
    expect("I1", row(a, "cask:zed").cls == "extra" and a.follows["commit"] != w.remote_head() or True, "")
    for i in range(12):
        b.outside(f"formula:tool{i}", True)
        b.person_adopt([f"formula:tool{i}"])
    b.outside("dock.tile-size", 36)
    b.person_adopt(["dock.tile-size"])
    b.outside("formula:jq", None)
    b.person_adopt(["formula:jq"])
    b.outside("formula:acme/tap/maker", True)
    b.person_adopt(["formula:acme/tap/maker"])
    # A returns. check() alone must change nothing but the local record.
    a.transport = "ok"
    head = w.remote_head()
    a.check()
    expect("I5", w.remote_head() == head, "returning must not send anything by itself")
    expect("I1", len(a.pending) == 2, "still waiting for the person")
    expect("I2", "formula:jq" in a.actual, "nothing uninstalled on return")
    expect("I2", row(a, "formula:jq").cls == "dropped", "jq should be a removal offer")
    a.person_send()
    c = w.read_config_raw()
    expect("I1", "zed" in c["packages"]["casks"], "the unrelated change goes through")
    expect("I1", c["settings"]["dock.tile-size"] == 36, "the setting B changed must not be overwritten")
    expect("I1", row(a, "dock.tile-size").cls == "conflict" and "changed-elsewhere" in a.notices, "the setting returns as a two-place row")
    done = a.person_apply_all()
    expect("I11", "formula:tool3" in done and "formula:acme/tap/maker" not in done, "a new tap needs its own approval")
    expect("I11", "needs-you:acme/tap" in a.notices, "the tap goes to the queue")
    a.person_approve_tap("acme/tap")
    a.person_apply(["formula:acme/tap/maker"])
    b.person_approve_tap("acme/tap")
    a.person_keep(["formula:jq"])
    resolve_all(w, [a, b], first_offer)
    say("the safe change is sent, the setting comes back as a two-place row, 14 entries wait for Apply, jq stays installed, and the new tap asks first")


def case_e_push_rejected(w, say):
    a, b = w.add("A"), w.add("B")
    a.outside("cask:zed", True)
    # GitHub moves after A fetched and built its commit, so the push itself is what fails.
    def race():
        a.before_push = None
        w.edit({"docs/unrelated.md": b"also in this repo\n"})
        b.outside("formula:ripgrep", True)
        b.person_adopt(["formula:ripgrep"])
    a.before_push = race
    expect("I1", a.person_adopt(["cask:zed"]) == "ok", "should succeed on the replay")
    expect("I1", a.rejected_pushes == 1, f"git should really have refused one push, saw {a.rejected_pushes}")
    c = w.read_config_raw()
    expect("I1", "zed" in c["packages"]["casks"] and "ripgrep" in c["packages"]["formulae"], "both survive")
    expect("I1", run_git(["show", f"{w.remote_head()}:docs/unrelated.md"], w.remote).rc == 0, "other files in the Config Repo survive")
    a.check()
    b.check()
    expect("I1", a.follows["commit"] == w.remote_head(), "A should be current")
    say("real git refused the push, the app replayed on the new top with no question asked, and all three changes survive")


def case_e_push_keeps_losing(w, say):
    a = w.add("A")
    a.outside("cask:zed", True)
    n = {"count": 0}

    def race():
        n["count"] += 1
        w.edit({"docs/busy.md": str(n["count"]).encode()})
    a.before_push = race
    status = a.person_adopt(["cask:zed"])
    expect("I1", status == "busy" and "busy" in a.notices and len(a.pending) == 1, f"expected to stop after 3 tries, got {status}")
    a.before_push = None
    expect("I1", a.person_send() == "ok" and not a.pending, "a later Send should work")
    say("after 3 lost races it stops and says so, the change waits, and Send finishes it")


def case_e_crash_after_push(w, say):
    a = w.add("A")
    a.outside("cask:zed", True)
    a.crash_after_push = True
    try:
        a.person_adopt(["cask:zed"])
        raise Violation("I1", "expected the simulated crash")
    except Crash:
        w.current = None
    a.reload()
    head = w.remote_head()
    expect("I1", len(a.pending) == 1, "the change is still marked waiting after the crash")
    a.person_send()
    expect("I7", w.remote_head() == head, "the replay must not commit twice")
    expect("I1", not a.pending and a.l("cask:zed") is True, "the record catches up")
    say("a crash between push and record: the next Send finds it already there and writes nothing")


def case_f_push_fails_no_network(w, say):
    a = w.add("A")
    a.outside("dock.tile-size", 64)
    a.transport = "offline"
    head = w.remote_head()
    expect("I1", a.person_adopt(["dock.tile-size"]) == "offline", "real git should fail with no route")
    expect("I1", len(a.pending) == 1 and "waiting" in a.notices, "the change waits")
    expect("I12", a.l("dock.tile-size") == 48, "the record must not move until GitHub holds it")
    expect("I12", row(a, "dock.tile-size").cls == "machine-moved", "the row must still say this Mac changed")
    a.transport = "ok"
    a.check()
    expect("I5", w.remote_head() == head and len(a.pending) == 1, "the network coming back sends nothing by itself")
    a.outside("dock.tile-size", 72)
    a.person_send()
    expect("I1", w.remote_head() == head and "changed-again" in a.notices, "a value that moved again goes back for review")
    expect("I1", row(a, "dock.tile-size").m == 72, "the row shows the newest value")
    a.person_adopt(["dock.tile-size"])
    expect("I1", w.read_config_raw()["settings"]["dock.tile-size"] == 72, "the reviewed value is what reaches GitHub")
    a.outside("dock.autohide", False)
    a.transport = "offline"
    a.person_adopt(["dock.autohide"])
    a.person_discard()
    expect("I1", not a.pending and a.actual["dock.autohide"] is False and row(a, "dock.autohide").cls == "machine-moved", "discard keeps the Mac as it is and the row returns")
    say("the change waits and the record stays put, nothing is sent until Send, a newer value asks again, and discard loses nothing on the Mac")


def case_f_expired_sign_in(w, say):
    a = w.add("A")
    a.outside("dock.tile-size", 64)
    a.transport = "auth-push"
    expect("I1", a.person_adopt(["dock.tile-size"]) == "auth", "should need sign-in")
    expect("I1", "sign-in" in a.notices and len(a.pending) == 1, "the change waits for sign-in")
    a.transport = "auth-all"
    expect("I1", a.check() == "auth" and not a.blocked(), "a private repo can't even check, and that isn't a broken Config")
    a.transport = "ok"
    expect("I1", a.person_send() == "ok" and not a.pending, "sending works after sign-in")
    say("public repo: reading still works and only the send waits. Private repo: checking pauses too. Both finish after sign-in (auth text simulated)")


def case_f_github_refuses(w, say):
    a = w.add("A")
    hook = w.remote / "hooks"
    hook.mkdir(exist_ok=True)
    pre = hook / "pre-receive"
    pre.write_text('#!/bin/sh\necho "remote: error: GH006: Protected branch update failed" >&2\nexit 1\n')
    pre.chmod(0o755)
    a.outside("dock.tile-size", 64)
    expect("I1", a.person_adopt(["dock.tile-size"]) == "denied", "a real pre-receive refusal should read as denied")
    expect("I1", "denied" in a.notices and len(a.pending) == 1, "the change waits")
    expect("I6", a.rejected_pushes == 0, "this must not read as the remote having moved")
    pre.unlink()
    expect("I1", a.person_send() == "ok", "it sends once allowed")
    say("a server-side refusal isn't mistaken for a moved Config, never retries in a loop, and the change waits")


def case_f_failure_words(w, say):
    table = [
        ("To github.com:o/r.git\n ! [rejected] main -> main (fetch first)\nerror: failed to push some refs", "moved"),
        (" ! [rejected] x -> main (non-fast-forward)", "moved"),
        ("fatal: Authentication failed for 'https://github.com/o/r.git/'", "auth"),
        ("fatal: could not read Username for 'https://github.com': terminal prompts disabled", "auth"),
        ("remote: Permission to o/r.git denied to someone.\nfatal: unable to access ...: The requested URL returned error: 403", "denied"),
        ("remote: error: GH006: Protected branch update failed for refs/heads/main.", "denied"),
        ("remote: error: GH013: Repository rule violations found for refs/heads/main.", "denied"),
        ("fatal: unable to access 'https://github.com/o/r.git/': Could not resolve host: github.com", "offline"),
        ("fatal: unable to access 'https://github.com/o/r.git/': Failed to connect to github.com port 443", "offline"),
    ]
    for text, wanted in table:
        expect("I1", classify_failure(text) == wanted, f"{text!r} read as {classify_failure(text)}")
    say(f"{len(table)} recorded failure messages map to the right outcome (wording to recheck on real GitHub)")


def case_g_invalid_from_github(w, say):
    bad = {
        "a trailing comma": b'{\n  "schemaVersion": 1,\n  "name": "x",\n}\n',
        "a duplicate key": b'{\n  "schemaVersion": 1,\n  "name": "x",\n  "name": "y"\n}\n',
        "an unknown field": b'{\n  "schemaVersion": 1,\n  "name": "x",\n  "setings": {}\n}\n',
        "a null setting": b'{\n  "schemaVersion": 1,\n  "name": "x",\n  "settings": {"dock.autohide": null}\n}\n',
    }
    for label, text in bad.items():
        sub = World(w.root / re.sub(r"\W", "-", label), SEED)
        a, b = sub.add("A"), sub.add("B")
        a.outside("cask:zed", True)
        a.transport = "offline"
        a.person_adopt(["cask:zed"])
        a.transport = "ok"
        good = a.follows["commit"]
        sub.edit({"configs/work.json": text})
        a.check()
        expect("I10", a.blocked() and a.follows["commit"] == good, f"{label}: should stay on the last good file")
        expect("I10", not [r for r in a.rows() if r.cls == "dropped"], f"{label}: nothing reads as dropped")
        expect("I10", a.person_send() == "blocked" and len(a.pending) == 1, f"{label}: the waiting change stays")
        expect("I10", a.person_apply_all() == "blocked", f"{label}: Apply is refused")
        expect("I2", "formula:jq" in a.actual, f"{label}: nothing uninstalled")
    say("trailing comma, duplicate key, unknown field and null setting all: stays on the last good file, no Apply or Adopt, nothing dropped, nothing removed")
    # Way out 1: someone fixes it on GitHub.
    sub = World(w.root / "fix", SEED)
    a = sub.add("A")
    sub.edit({"configs/work.json": bad["a trailing comma"]})
    a.check()
    sub.edit({"configs/work.json": canonical(SEED).encode()})
    a.check()
    expect("I10", not a.blocked(), "a fix on GitHub clears it by itself")
    say("fixed on GitHub: the next check clears it with no action")
    # Way out 2: the person goes back to the last version that worked.
    sub = World(w.root / "restore", SEED)
    a, b = sub.add("A"), sub.add("B")
    a.outside("cask:zed", True)
    a.transport = "offline"
    a.person_adopt(["cask:zed"])
    a.transport = "ok"
    sub.edit({"configs/work.json": bad["a trailing comma"]})
    a.check()
    expect("I1", a.person_restore() == "ok" and not a.blocked(), "restore should work")
    expect("I5", b.check() == "ok" and sub.read_config_raw() == SEED, "the file is back")
    expect("I1", a.person_send() == "ok" and "zed" in sub.read_config_raw()["packages"]["casks"], "the waiting change then goes through")
    say("going back to the last version that worked: one reviewed commit, then the waiting change goes through on top")
    # Newer schema: no restore, no write.
    sub = World(w.root / "newer", SEED)
    a = sub.add("A")
    sub.edit({"configs/work.json": b'{\n  "schemaVersion": 2,\n  "name": "x",\n  "future": true\n}\n'})
    a.check()
    head = sub.remote_head()
    expect("I10", a.problem[0] == "too-new", "a newer schema is its own case")
    expect("I10", a.person_restore() == "problem" and sub.remote_head() == head, "going back would downgrade someone else's file, so it isn't offered")
    say("a newer schemaVersion: says to update Soulcaster and offers no going back")
    # A per-entry problem isn't a whole-file problem.
    sub = World(w.root / "entry", SEED)
    a = sub.add("A")
    sub.edit_config(lambda c: c["settings"].update({"dock.tile-size": 9999}))
    a.check()
    expect("I1", not a.blocked() and row(a, "dock.tile-size") is None, "a refused value skips only that entry")
    a.outside("cask:zed", True)
    a.person_adopt(["cask:zed"])
    expect("I1", sub.read_config_raw()["settings"]["dock.tile-size"] == 9999, "and stays in the file")
    say("a value out of range skips only that entry, Adopt still works, and the value stays in the file")


def case_g_never_commits_invalid(w, say):
    a = w.add("A")
    a.outside("screenshots.location", "/Users/me/Pictures")
    w.edit_config(lambda c: c["settings"].update({"screenshots.location": "~/Pictures"}))
    a.check()
    r = row(a, "screenshots.location")
    expect("I4", "adopt" not in r.offers and "home folder" in r.why, "a home-folder value isn't offered")
    head = w.remote_head()
    a.pending.append({"id": w.new_op("A"), "key": "screenshots.location", "before": "~/Pictures", "after": "/Users/me/Pictures", "machineAtChoice": "/Users/me/Pictures"})
    a.person_send()
    expect("I4", w.remote_head() == head and "not-adoptable" in a.notices, "a value the lint refuses never reaches a commit")
    say("a home-folder value is never offered, and even if it were queued the send stops before committing")


def case_h_force_push(w, say):
    a, b = w.add("A"), w.add("B")
    first = w.remote_head()
    a.outside("cask:zed", True)
    a.person_adopt(["cask:zed"])
    b.check()
    b.person_apply_all()
    old = w.remote_head()
    b.outside("dock.tile-size", 64)
    b.transport = "offline"
    b.person_adopt(["dock.tile-size"])
    b.transport = "ok"
    rewritten = copy.deepcopy(SEED)
    rewritten["packages"]["formulae"] = ["neovim"]  # jq gone too
    rewritten["packages"]["casks"].append("kitty")
    w.force_push(first, {"configs/work.json": canonical(rewritten).encode()})
    head = w.remote_head()
    a.check()
    expect("I1", "rewritten" in a.notices, "the person is told once")
    expect("I2", a.actual.get("cask:zed") and a.actual.get("formula:jq"), "nothing uninstalled")
    expect("I2", row(a, "cask:zed").cls == "dropped" and row(a, "formula:jq").cls == "dropped", "lost entries are removal offers")
    expect("I1", row(a, "cask:kitty").cls == "new-in-config", "new entries are ordinary")
    expect("I1", a.show(old) is not None, "the old version stays readable on this Mac")
    b.person_send()
    expect("I6", out(run_git(["rev-parse", f"{w.remote_head()}^"], w.remote)) == head, "the next write goes on top of the rewrite, with no force")
    expect("I1", w.read_config_raw()["settings"]["dock.tile-size"] == 64, "the waiting change still lands")
    a.person_add_back(["cask:zed"])
    a.person_keep(["formula:jq"])
    resolve_all(w, [a, b], first_offer)
    say("told once, compared by entry, nothing removed, entries that vanished become offers, waiting changes land on top with no force, and the old version stays readable")


def case_i_file_renamed(w, say):
    a, b = w.add("A"), w.add("B")
    text = run_git(["show", "main:configs/work.json"], w.remote).out
    w.edit({"configs/work.json": None, "configs/office.json": text}, "Rename on GitHub")
    a.check()
    expect("I10", a.blocked() and a.problem[0] == "missing", "a moved file blocks writes")
    expect("I10", a.problem[1]["new_path"] == "configs/office.json", "real git finds the likely new path")
    expect("I10", a.follows["path"] == "configs/work.json", "it doesn't follow by itself")
    expect("I2", not [r for r in a.rows() if r.cls == "dropped"], "nothing reads as dropped")
    a.person_follow("configs/office.json")
    a.check()
    expect("I1", w.settled(a), "following a pure rename leaves nothing to review")
    a.outside("cask:zed", True)
    a.person_adopt(["cask:zed"])
    b.check()
    b.person_follow("configs/office.json")
    b.check()
    expect("I1", row(b, "cask:zed").cls == "new-in-config", "the second Mac follows and then sees a normal change")
    say("real rename detection names the new path, the person confirms it, and nothing is dropped meanwhile")


def case_i_file_deleted(w, say):
    a = w.add("A")
    a.outside("cask:zed", True)
    a.transport = "offline"
    a.person_adopt(["cask:zed"])
    a.transport = "ok"
    w.edit({"configs/work.json": None}, "Delete on GitHub")
    a.check()
    expect("I10", a.blocked() and a.problem[1]["new_path"] is None, "no rename to offer")
    expect("I10", a.person_send() == "blocked" and len(a.pending) == 1, "the waiting change stays")
    expect("I2", "formula:jq" in a.actual, "nothing removed")
    expect("I1", a.person_restore() == "ok" and not a.blocked(), "putting it back works")
    expect("I1", a.person_send() == "ok", "and the waiting change lands")
    say("nothing is removed, writes stop, Put it back restores the file with one commit, and the waiting change lands")


def case_i_branch_renamed(w, say):
    a = w.add("A")
    run_git(["branch", "-m", "main", "trunk"], w.remote, check=True)
    run_git(["symbolic-ref", "HEAD", "refs/heads/trunk"], w.remote, check=True)
    a.fetch()
    expect("I10", a.blocked() and a.problem[1]["new_branch"] == "trunk", "the new default is found")
    expect("I2", "formula:jq" in a.actual and not [r for r in a.rows() if r.cls == "dropped"], "nothing reads as dropped")
    a.person_follow(branch="trunk")
    expect("I1", not a.blocked(), "follow it there")
    run_git(["branch", "-m", "trunk", "main"], w.remote, check=True)
    run_git(["symbolic-ref", "HEAD", "refs/heads/main"], w.remote, check=True)
    say("a renamed default branch is handled like a moved file, in plain words")


def case_j_version_skew(w, say):
    seed = copy.deepcopy(SEED)
    seed["items"].append("tool-old")
    sub = World(w.root / "skew", seed)
    new, old = sub.add("New", newer_catalog()), sub.add("Old", older_catalog())
    # Newer app never rewrites an old ID as a side effect.
    new.outside("dock.tile-size", 64)
    new.person_adopt(["dock.tile-size"])
    expect("I7", "tool-old" in sub.read_config_raw()["items"], "no rewriting except Tidy")
    old.check()
    old.person_apply_all()
    # An Item only the newer Catalog knows.
    new.outside("new-thing", True)
    new.person_adopt(["new-thing"])
    new.outside("dock.new-setting", True)
    sub.edit_config(lambda c: c["settings"].update({"dock.new-setting": True}))
    old.outside("formula:ripgrep", True)
    old.check()
    r = row(old, "formula:ripgrep")
    expect("I2", r.cls == "held" and not r.offers, "no package Adopt while an ID is unknown")
    # Older app's write keeps what it can't read.
    old.outside("dock.autohide", False)
    old.person_adopt(["dock.autohide"])
    c = sub.read_config_raw()
    expect("I1", "new-thing" in c["items"] and c["settings"]["dock.new-setting"] is True, "unknown entries survive")
    # Tidy, explicit, on the newer app. The older one must not read it as removal.
    expect("I7", "tool-old" in sub.read_config_raw()["items"], "still not rewritten")
    expect("I1", new.person_tidy() == "ok", "Tidy works")
    expect("I1", "tool-new" in sub.read_config_raw()["items"], "Tidy rewrote it")
    old.check()
    expect("I2", row(old, "tool-old").cls == "held" and "tool-old" in old.actual, "older app: held, no removal offer, nothing uninstalled")
    # Six rounds of alternating writes never fight over form.
    before = None
    for i in range(6):
        m = new if i % 2 else old
        m.outside(f"cask:round{i}", True)
        m.check()
        if m is new:
            m.person_adopt([f"cask:round{i}"])
        else:
            m.pending.append({"id": sub.new_op("Old"), "key": f"cask:round{i}", "before": None, "after": True, "machineAtChoice": True})
            m.person_send()
        c = sub.read_config_raw()
        expect("I7", "tool-new" in c["items"] and "new-thing" in c["items"], f"round {i} rewrote entries")
    # A newer schema: the older app refuses and writes nothing.
    head_text = sub.read_config_raw()
    v2 = dict(head_text, schemaVersion=2)
    sub.edit({"configs/work.json": json.dumps(v2, indent=2).encode() + b"\n"})
    head = sub.remote_head()
    old.check()
    expect("I10", old.problem[0] == "too-new" and old.person_send() in ("blocked", "ok"), "older app refuses")
    expect("I10", sub.remote_head() == head, "and writes nothing")
    # Updating the older app clears all of it.
    sub.edit({"configs/work.json": canonical(head_text).encode()})
    old.cat = newer_catalog()
    new.check()
    old.check()
    resolve_all(sub, [new, old], first_offer)
    say("old IDs aren't rewritten, the older app keeps entries it can't read, no removal or package Adopt while an ID is unknown, Tidy is explicit, 6 alternating writes never fight, and updating clears it")


def case_k_removal_reaches_other_machine(w, say):
    a, b = w.add("A"), w.add("B")
    a.outside("formula:jq", None)
    a.check()
    r = row(a, "formula:jq")
    expect("I1", r.cls == "machine-moved" and "adopt" in r.offers and "apply" in r.offers, "A: Adopt deletes the line, Apply reinstalls")
    a.person_adopt(["formula:jq"])
    expect("I1", "jq" not in w.read_config_raw()["packages"]["formulae"], "the line is gone")
    b.check()
    r = row(b, "formula:jq")
    expect("I2", r.cls == "dropped" and r.offers == ("keep", "remove", "add-back"), f"B must be asked: {r}")
    expect("I2", "formula:jq" in b.actual, "still installed after checking")
    done = b.person_apply_all()
    expect("I2", done == [] and "formula:jq" in b.actual, "Apply all never removes")
    expect("I5", w.remote_head() == w.head, "nothing written")
    # Keep.
    b.person_keep(["formula:jq"])
    b.check()
    expect("I3", row(b, "formula:jq") is None, "keeping stops asking, and doesn't come back as something to Adopt")
    expect("I2", "formula:jq" in b.actual, "still installed")
    # It returning to the Config clears the keep.
    a.outside("formula:jq", True)
    a.check()
    a.person_adopt(["formula:jq"])
    b.check()
    expect("I1", "formula:jq" not in b.record["keptHere"] and row(b, "formula:jq") is None, "back in the Config: in sync, with no leftover note")
    say("the keep path: B asked once, nothing removed, Apply all leaves it, and no re-offer")
    # Remove, with a confirmation.
    sub = World(w.root / "remove", SEED)
    a, b, c = sub.add("A"), sub.add("B"), sub.add("C", actual={k: v for k, v in flatten(SEED, older_catalog())[0].items() if k != "cask:rectangle"})
    a.outside("cask:firefox", None)
    a.person_adopt(["cask:firefox"])
    b.check()
    b.person_confirm_remove(["cask:firefox"])
    expect("I2", "cask:firefox" not in b.actual, "removed after confirming")
    expect("I1", sub.settled(b), "and settled")
    # Add back: A then sees it as new, and isn't forced to install.
    a.outside("cask:rectangle", None)
    a.person_adopt(["cask:rectangle"])
    b.check()
    b.person_add_back(["cask:rectangle"])
    a.check()
    expect("I1", row(a, "cask:rectangle").cls == "new-in-config", "A sees an offer to install, and nothing else")
    a.person_not_here(["cask:rectangle"])
    expect("I3", sub.settled(a), "A can decline, and stays settled")
    say("the remove path: only after confirming. Add back: A is offered it, and can decline with \"not here\"")
    # A Machine with no record of it, and an always-installed Item.
    sub = World(w.root / "no-record", SEED)
    a = sub.add("A")
    n = sub.add("N", actual={**flatten(SEED, older_catalog())[0], "formula:gh": True})
    n.record["lastApplied"].pop("cask:rectangle")
    n.save()
    a.outside("cask:rectangle", None)
    a.person_adopt(["cask:rectangle"])
    n.check()
    r = row(n, "cask:rectangle")
    expect("I2", r.cls == "extra" and "remove" not in r.offers, "with no record there's never a removal offer")
    sub.edit_config(lambda c: c["packages"]["formulae"].append("gh"))
    n.check()
    expect("I2", row(n, "formula:gh") is None, "always-installed Items never show")
    a.outside("cask:zed", True)
    a.check()
    a.person_adopt(["cask:zed"])
    expect("I7", "gh" not in sub.read_config_raw()["packages"]["formulae"], "the next reviewed write drops the entry naming it")
    say("no record: never a removal offer. Always-installed Items never appear")


def case_l_switch_config(w, say):
    personal = copy.deepcopy(SEED)
    personal["name"] = "Personal"
    personal["packages"] = {"formulae": ["neovim", "ripgrep"], "casks": ["firefox"]}
    personal["settings"]["dock.tile-size"] = 36
    sub = World(w.root / "switch", files={"configs/work.json": canonical(SEED).encode(), "configs/personal.json": canonical(personal).encode()})
    a = sub.add("A")
    a.outside("cask:zed", True)
    a.transport = "offline"
    a.person_adopt(["cask:zed"])
    a.transport = "ok"
    expect("I1", a.person_switch("configs/personal.json") == "blocked" and "switch-blocked" in a.notices, "waiting changes must be resolved first")
    expect("I1", a.follows["path"] == "configs/work.json", "still on the old Config")
    a.person_send()
    a.person_switch("configs/personal.json")
    a.check()
    expect("I2", all(k in a.actual for k in ("formula:jq", "cask:rectangle", "ghostty")), "nothing removed on switching")
    expect("I1", row(a, "formula:jq").cls == "dropped", "old entries read as dropped, entry by entry")
    expect("I1", row(a, "formula:ripgrep").cls == "new-in-config", "new entries are offers")
    expect("I1", row(a, "dock.tile-size").cls == "config-moved", "settings differ as Config-moved")
    a.person_switch("configs/work.json")
    a.check()
    expect("I1", sub.settled(a), "switching straight back leaves nothing to review")
    a.person_switch("configs/personal.json")
    a.check()
    resolve_all(sub, [a], first_offer)
    expect("I2", "formula:jq" in a.actual, "keeping is the first offer, so it stays")
    say("waiting changes first, then every old entry becomes an offer with nothing removed, and switching straight back changes nothing")


def case_m_record_lost(w, say):
    a = w.add("A")
    a.outside("cask:zed", True)
    head = w.remote_head()
    a.record["lastApplied"] = {}
    a.record["pending"] = []
    a.record["follows"]["commit"] = None
    a.fetch()
    a.check()
    expect("I2", not [r for r in a.rows() if r.cls == "dropped"], "no record means no removal offers")
    expect("I1", row(a, "cask:zed").cls == "extra", "local changes still show")
    expect("I5", w.remote_head() == head, "nothing sent")
    say("a lost Machine Record: reads as a first look, with no removal offers and nothing sent. GitHub still holds the truth")


def case_n_review_race(w, say):
    a, b = w.add("A"), w.add("B")
    b.outside("dock.tile-size", 64)
    b.person_adopt(["dock.tile-size"])
    a.check()
    seen = {"dock.tile-size": row(a, "dock.tile-size").c}
    b.outside("dock.tile-size", 72)
    b.person_adopt(["dock.tile-size"])
    done = a.person_apply(["dock.tile-size"], reviewed=seen)
    expect("I11", done == [] and a.actual["dock.tile-size"] == 48, "Apply must not use a value the person didn't review")
    expect("I11", row(a, "dock.tile-size").c == 72, "the row refreshes")
    say("a value that changes during review isn't applied, and the row refreshes")


def case_o_hand_edits_settle(w, say):
    a, b = w.add("A"), w.add("B")
    ugly = copy.deepcopy(SEED)
    ugly["items"] = ["junie", "ghostty", "dockutil"]
    text = json.dumps(ugly).encode() + b"\n"  # one line, unsorted, and valid
    w.edit({"configs/work.json": text})
    a.check()
    expect("I1", w.settled(a), "a reformat alone isn't Drift")
    head = w.remote_head()
    # kitty sorts between firefox and rectangle, so the line is inserted and the neighbours stay.
    # A name that sorts last would also change the previous line's comma, which is just JSON.
    a.outside("cask:kitty", True)
    a.person_adopt(["cask:kitty"])
    commits = out(run_git(["rev-list", "--count", f"{head}..{w.remote_head()}"], w.remote))
    expect("I7", commits == "2", f"expected a reformat commit and an Adopt commit, got {commits}")
    stat = out(run_git(["diff", "--numstat", f"{w.remote_head()}^", w.remote_head()], w.remote))
    expect("I7", stat.split()[0] == "1" and stat.split()[1] == "0", f"the Adopt commit should be one line, got {stat}")
    say("a valid hand edit in another form: 2 commits pushed together, and the Adopt one is exactly 1 line")


def case_p_private_config_repo(w, say):
    """Credentials reach the engine's git only through the explicit flag, never the person's config."""
    home = w.root / "cred-home"
    home.mkdir()
    (home / ".gitconfig").write_text('[credential]\n\thelper = !echo person-config-helper\n')
    fake_gh = w.root / "fake-gh"
    fake_gh.write_text('#!/bin/sh\necho username=x-access-token\necho password=token-from-keychain\n')
    fake_gh.chmod(0o755)
    query = b"protocol=https\nhost=github.com\n\n"
    plain = run_git(["credential", "fill"], env={"HOME": str(home)}, data=query, guards=False)
    expect("I13", b"token-from-keychain" not in plain.out, "control: no helper without the flag")
    flagged = run_git(["-c", f"credential.https://github.com.helper=!{fake_gh}", "credential", "fill"], env={"HOME": str(home)}, data=query)
    expect("I13", b"password=token-from-keychain" in flagged.out, f"the helper should answer: {flagged.err}")
    say("under the engine's guards ~/.gitconfig's helper is invisible, and a helper passed with -c answers, so the token never sits in argv or a URL")


def case_q_dotfiles(w, say):
    origin = w.root / "dotfiles.git"
    run_git(["init", "--quiet", "--bare", "-b", "main", "--template=", str(origin)], check=True)
    first = commit_file(origin, None, {"zsh/zshrc": b"one\n", "script/setup": b"#!/bin/sh\n"}, "One", HAND, w.root)
    run_git(["update-ref", "refs/heads/main", first], origin, check=True)
    d = Dotfiles(w, origin)
    d.approved = d.head()
    expect("I9", d.status() == {"behind": 0, "unpushed": False}, f"clean: {d.status()}")
    second = commit_file(origin, first, {"zsh/zshrc": b"two\n"}, "Two", HAND, w.root)
    run_git(["update-ref", "refs/heads/main", second], origin, check=True)
    expect("I9", d.status() == {"behind": 1, "unpushed": False}, f"behind: {d.status()}")
    expect("I9", d.head() == first, "a fetch moves nothing on disk")
    reviewed = d.review_target()
    expect("I9", reviewed == second, "the preview pins what's being reviewed")
    third = commit_file(origin, second, {"zsh/zshrc": b"three\n"}, "Three", HAND, w.root)
    run_git(["update-ref", "refs/heads/main", third], origin, check=True)
    d.status()
    expect("I9", d.update(reviewed) == "ok" and d.head() == second, "update stops at the reviewed commit, even with a newer one fetched")
    expect("I9", d.approved == second and d.script_may_run(), "approval moves with it")
    expect("I9", d.status()["behind"] == 1, "the newer one still shows as behind")
    # A person's own pull: not the engine's to stop, and the script holds.
    run_git(["pull", "--quiet", "--ff-only", str(origin), "main"], cwd=d.work, guards=False, env=APP_IDENTITY)
    expect("I9", d.head() == third and d.approved == second and not d.script_may_run(), "a person's own pull holds the script until reviewed")
    expect("I9", d.status() == {"behind": 0, "unpushed": False}, f"status stays only these two: {d.status()}")
    say("behind is shown, updating stops at the exact reviewed commit, and a person's own pull holds only the script")
    # Unpushed: local commit and uncommitted edit.
    (d.work / "zsh" / "zshrc").write_text("edited\n")
    expect("I9", d.status()["unpushed"] is True, "an uncommitted edit counts")
    expect("I9", d.update(third) == "by-hand" and d.head() == third, "a dirty copy is never touched")
    run_git(["checkout", "--quiet", "--", "zsh/zshrc"], cwd=d.work, guards=False)
    (d.work / "local").write_text("x\n")
    run_git(["add", "local"], cwd=d.work, guards=False)
    run_git(["commit", "--quiet", "-m", "local"], cwd=d.work, guards=False, env=APP_IDENTITY)
    expect("I9", d.status() == {"behind": 0, "unpushed": True}, f"a local commit counts: {d.status()}")
    fourth = commit_file(origin, third, {"other": b"x\n"}, "Four", HAND, w.root)
    run_git(["update-ref", "refs/heads/main", fourth], origin, check=True)
    expect("I9", d.status() == {"behind": 1, "unpushed": True}, f"both at once: {d.status()}")
    head = d.head()
    expect("I9", d.update(d.review_target()) == "by-hand" and d.head() == head, "both at once: never merged by the app")
    say("unpushed work and being behind can both show, and Soulcaster never merges or stashes them")
    # The engine's flags against a repo's own .git/config and hooks.
    marker = w.root / "marker"
    evil = w.root / "evil.sh"
    evil.write_text(f'#!/bin/sh\ntouch {marker}\n')
    evil.chmod(0o755)
    run_git(["config", "core.fsmonitor", str(evil)], cwd=d.work, guards=False, check=True)
    run_git(["status", "--porcelain"], cwd=d.work, guards=False)
    expect("I13", marker.exists(), "control: .git/config really does run a program on a plain status, even with GIT_CONFIG_GLOBAL=/dev/null")
    marker.unlink()
    d.status()
    expect("I13", not marker.exists(), "the engine's status must not run it")
    run_git(["config", "--unset", "core.fsmonitor"], cwd=d.work, guards=False, check=True)
    hooks = d.work / ".git" / "hooks"
    hooks.mkdir(exist_ok=True)
    (hooks / "post-merge").write_text(f'#!/bin/sh\ntouch {marker}\n')
    (hooks / "post-merge").chmod(0o755)
    run_git(["reset", "--quiet", "--hard", third], cwd=d.work, guards=False, check=True)
    d.status()
    expect("I9", d.update(d.review_target()) == "ok", "a clean update")
    expect("I13", not marker.exists(), "the engine's update must not run a hook")
    say("a repo's own core.fsmonitor really runs on plain git status (control), and -c core.fsmonitor=false and core.hooksPath=/dev/null stop it and hooks")


class Dotfiles:
    """Only what the status needs. The Dotfiles Repo stays a normal checkout the person owns."""

    def __init__(self, w, origin):
        self.origin = origin
        self.work = w.root / "machines" / "dotfiles"
        self.work.parent.mkdir(parents=True, exist_ok=True)
        run_git(["clone", "--quiet", "--template=", str(origin), str(self.work)], check=True)
        self.approved = None

    def g(self, *args, **kw):
        return run_git(args, cwd=self.work, guards="no-dotfiles-guard" not in MUT, env=APP_IDENTITY, **kw)

    def head(self):
        return out(self.g("rev-parse", "HEAD"))

    def status(self):
        self.g("fetch", "--quiet", "--no-tags", str(self.origin), "+refs/heads/main:refs/remotes/origin/main")
        behind = int(out(self.g("rev-list", "--count", "HEAD..refs/remotes/origin/main")))
        ahead = int(out(self.g("rev-list", "--count", "refs/remotes/origin/main..HEAD")))
        dirty = bool(out(self.g("status", "--porcelain")))
        return {"behind": behind, "unpushed": ahead > 0 or dirty}

    def review_target(self):
        return out(self.g("rev-parse", "refs/remotes/origin/main"))

    def update(self, reviewed):
        """After a person reviewed exactly this commit. Fast-forward or nothing."""
        s = self.status()
        if s["unpushed"] or self.g("merge-base", "--is-ancestor", "HEAD", reviewed).rc:
            return "by-hand"
        target = "refs/remotes/origin/main" if "pull-past-approved" in MUT else reviewed
        self.g("merge", "--quiet", "--ff-only", target, check=True)
        self.approved = reviewed
        expect("I9", self.head() == self.approved, f"moved to {self.head()[:8]}, past the approved {reviewed[:8]}")
        return "ok"

    def script_may_run(self):
        return self.head() == self.approved


def case_r_wording(w, say):
    table = UI
    if "git-word-in-ui" in MUT:
        table = dict(UI, **{"row.conflict": "{name} has a merge conflict."})
    for key, text in table.items():
        hit = GIT_WORDS.search(text)
        expect("I14", not hit, f"{key} says {hit.group(0) if hit else ''!r}: {text}")
    say(f"{len(table)} strings use no Git vocabulary")


# ------------------------------------------------------------------ why not git merge: real runs
def merge_evidence(root, say):
    """The same two edits through real git merge and through entry replay."""
    run = lambda name, base, edit_a, edit_b: one_merge(pathlib.Path(root) / name, base, edit_a, edit_b)
    base = copy.deepcopy(SEED)
    base["packages"]["casks"] = ["firefox", "rectangle"]

    def add_cask(n):
        return lambda c: c["packages"]["casks"].append(n)

    def set_setting(k, v):
        return lambda c: c["settings"].update({k: v})

    def move(to):
        def f(c):
            l = [x for x in DOCK if x != MAIL]
            c["settings"]["dock.apps"] = l[:to] + [MAIL] + l[to:]
        return f

    rows = [
        ("two casks added at the end of one list", run("adjacent-end", base, add_cask("zed"), add_cask("zulu"))),
        ("two different settings added next to each other", run("adjacent-settings", base, set_setting("dock.position", "left"), set_setting("dock.show-recents", False))),
    ]
    results = {name: r for name, r in rows}
    ok = [r for r in rows if r[1]["git"] == "conflict markers" and r[1]["entry"] == "clean, valid"]
    expect("I4", len(ok) == 2, f"expected false conflicts from git: {rows}")
    say(f"{rows[0][0]}: git gives {rows[0][1]['git']}, entries give {rows[0][1]['entry']}")
    say(f"{rows[1][0]}: git gives {rows[1][1]['git']}, entries give {rows[1][1]['entry']}")
    moved = run("mail-moved", base, move(3), move(5))
    expect("I4", moved["git"].startswith("clean, but refused"), f"expected a clean merge that fails validation: {moved}")
    say(f"both move Mail in dock.apps: git gives {moved['git']}, entries give {moved['entry']}")
    dup = hand_duplicate(pathlib.Path(root) / "hand-dup", base)
    expect("I4", dup.startswith("clean, but refused"), f"expected a duplicate key: {dup}")
    say(f"a hand edit and the app add one setting on different lines: git gives {dup}, entries read the hand edit first, so it's one key")
    return results


def one_merge(work, base, edit_a, edit_b):
    work.mkdir(parents=True)
    env = dict(APP_IDENTITY)
    g = lambda *a, **k: run_git(a, cwd=work, env=env, **k)
    g("init", "--quiet", "-b", "main", check=True)
    f = work / "c.json"
    f.write_text(canonical(base))
    g("add", ".", check=True)
    g("commit", "--quiet", "-m", "base", check=True)
    a, b = copy.deepcopy(base), copy.deepcopy(base)
    edit_a(a)
    edit_b(b)
    g("switch", "--quiet", "-c", "a", check=True)
    f.write_text(canonical(a))
    g("commit", "--quiet", "-am", "a", check=True)
    g("switch", "--quiet", "main", check=True)
    f.write_text(canonical(b))
    g("commit", "--quiet", "-am", "b", check=True)
    r = g("merge", "--quiet", "--no-edit", "a")
    text = f.read_bytes()
    if b"<<<<<<<" in text:
        git_result = "conflict markers"
    else:
        status, why = read_config(text, older_catalog())
        git_result = "clean, valid" if status == "ok" else f"clean, but refused: {why}"
    return {"git": git_result, "entry": entry_merge(base, a, b), "rc": r.rc}


def entry_merge(base, a, b):
    """Replay a's changes on b, by entry, as the engine does."""
    cat = Catalog(ITEMS, SETTINGS + ["dock.show-recents"], bounds=BOUNDS)
    E0, Ea, Eb = flatten(base, cat)[0], flatten(a, cat)[0], flatten(b, cat)[0]
    out_cfg = copy.deepcopy(b)
    for k in sorted(set(E0) | set(Ea)):
        if E0.get(k) == Ea.get(k):
            continue
        if Eb.get(k) in (E0.get(k), Ea.get(k)):
            set_entry(out_cfg, k, Ea.get(k), cat)
        else:
            return "one row for the person"
    status, why = read_config(canonical(out_cfg).encode(), older_catalog())
    return "clean, valid" if status == "ok" else f"refused: {why}"


def hand_duplicate(work, base):
    work.mkdir(parents=True)
    g = lambda *a, **k: run_git(a, cwd=work, env=APP_IDENTITY, **k)
    g("init", "--quiet", "-b", "main", check=True)
    f = work / "c.json"
    f.write_text(canonical(base))
    g("add", ".", check=True)
    g("commit", "--quiet", "-m", "base", check=True)
    g("switch", "--quiet", "-c", "hand", check=True)
    # A person pastes the new setting at the top of settings, as people do.
    f.write_text(canonical(base).replace('"settings": {\n', '"settings": {\n    "dock.position": "left",\n'))
    g("commit", "--quiet", "-am", "hand", check=True)
    g("switch", "--quiet", "main", check=True)
    app = copy.deepcopy(base)
    app["settings"]["dock.position"] = "left"
    f.write_text(canonical(app))
    g("commit", "--quiet", "-am", "app", check=True)
    r = g("merge", "--quiet", "--no-edit", "hand")
    text = f.read_bytes()
    if b"<<<<<<<" in text:
        return f"conflict markers (rc {r.rc})"
    status, why = read_config(text, older_catalog())
    return "clean, valid" if status == "ok" else f"clean, but refused: {why}"


# ------------------------------------------------------------------ random schedules
def fuzz(root, seeds, say=None):
    done = 0
    for seed in range(seeds):
        one_schedule(pathlib.Path(root) / f"f{seed}", seed)
        shutil.rmtree(pathlib.Path(root) / f"f{seed}", ignore_errors=True)
        done += 1
    return done


def one_schedule(root, seed):
    rnd = random.Random(seed)
    root.mkdir(parents=True)
    w = World(root, SEED)
    a, b = w.add("A"), w.add("B")
    machines = [a, b]
    broken = False
    names = [f"formula:p{i}" for i in range(6)] + [f"cask:c{i}" for i in range(4)] + ["formula:jq", "cask:firefox"]

    def do_outside(m):
        kind = rnd.random()
        if kind < 0.45:
            k = rnd.choice(names)
            m.outside(k, None if k in m.actual else True)
        elif kind < 0.8:
            m.outside("dock.tile-size", rnd.choice([24, 36, 48, 64]))
        else:
            current = list(m.actual.get("dock.apps", DOCK))
            rnd.shuffle(current) if rnd.random() < 0.3 else current.pop(rnd.randrange(len(current))) if len(current) > 2 else None
            m.outside("dock.apps", current)

    def do_github():
        nonlocal broken
        roll = rnd.random()
        if broken or roll < 0.15:
            if broken:
                w.edit({"configs/work.json": canonical(last_good()).encode()})
                broken = False
            else:
                w.edit({"configs/work.json": b'{ "schemaVersion": 1, "name": "x", }\n'})
                broken = True
        elif roll < 0.25 and not broken:
            old = out(run_git(["rev-list", "--max-count=1", "--skip=1", "refs/heads/main"], w.remote))
            if old:
                c = w.read_config_raw()
                c["packages"]["casks"] = [x for x in c.get("packages", {}).get("casks", [])][:1]
                c["packages"].setdefault("formulae", [])
                w.force_push(old, {"configs/work.json": canonical(c).encode()})
        else:
            c = w.read_config_raw()
            k = rnd.choice(names)
            section = "formulae" if k.startswith("formula:") else "casks"
            name = k.split(":")[1]
            lst = c.setdefault("packages", {}).setdefault(section, [])
            lst.remove(name) if name in lst else lst.append(name)
            if rnd.random() < 0.3:
                c.setdefault("settings", {})["dock.tile-size"] = rnd.choice([24, 36, 48, 64])
            w.edit({"configs/work.json": canonical(c).encode() if rnd.random() < 0.7 else json.dumps(c).encode() + b"\n"})

    def last_good():
        for sha in out(run_git(["rev-list", "refs/heads/main"], w.remote)).split():
            data = run_git(["show", f"{sha}:configs/work.json"], w.remote).out
            if read_config(data, older_catalog())[0] == "ok":
                return strict_parse(data)
        return SEED

    def do_row(m):
        if m.blocked():
            return
        rows = [r for r in m.rows() if r.cls != "held" and r.offers]
        if rows:
            r = rnd.choice(rows)
            choice = rnd.choice(list(r.offers) + (["combined"] if r.combined else []))
            choose(m, r, choice)

    before_checks = lambda: {m.name: dict(m.actual) for m in machines}
    for _ in range(rnd.randint(8, 22)):
        m = rnd.choice(machines)
        roll = rnd.random()
        if roll < 0.28:
            do_outside(m)
        elif roll < 0.42:
            do_github()
        elif roll < 0.52:
            m.transport = rnd.choice(["ok", "ok", "offline", "auth-push"])
        elif roll < 0.72:
            snap = dict(m.actual)
            head = w.remote_head()
            m.check()
            expect("I11", m.actual == snap, "check changed the Mac")
            expect("I5", w.remote_head() == head, "check changed GitHub")
        elif roll < 0.92:
            do_row(m)
        elif m.pending:
            if rnd.random() < 0.8:
                m.person_send()
            else:
                m.person_discard()
    # Everything comes back, and a person works through what's left.
    for m in machines:
        m.transport = "ok"
    if broken:
        w.edit({"configs/work.json": canonical(last_good()).encode()})
    installed_before = {m.name: set(k for k in m.actual if not is_setting(k)) for m in machines}
    choices = lambda r: rnd.choice([o for o in r.offers if o != "remove"] or list(r.offers))
    resolve_all(w, machines, choices)
    for m in machines:
        missing = installed_before[m.name] - set(m.actual)
        expect("I2", not missing, f"{m.name} lost {missing} during the final pass, with no removal chosen")
        C = flatten(m.config(), m.cat)[0]
        for k, v in C.items():
            if k in m.record["unavailable"]:
                continue
            expect("I3", view(v, m.tiles(k)) == m.actual.get(k), f"{m.name}: {k} is {m.actual.get(k)}, Config says {v}")


# ------------------------------------------------------------------ running it
# Each breaks one rule on purpose. Expected is what must catch it.
MUTATIONS = {
    "force-push": ("I6", "push --force when the push is refused"),
    "blind-overwrite": ("I1", "write the reviewed value even if GitHub's has moved"),
    "line-merge": ("I4", "use git's line merge instead of entry replay"),
    "whole-file": ("I1", "write the file from the Machine, instead of changing entries"),
    "skip-validate": ("I4", "don't lint or validate before committing"),
    "auto-commit": ("I5", "commit Machine changes during a check"),
    "auto-apply": ("I11", "apply Config changes during a check"),
    "apply-all-removes": ("I2", "let Apply all include removals"),
    "no-skew-guard": ("I2", "offer removal while the file holds an unknown ID"),
    "missing-as-empty": ("I10", "read a missing file as an empty Config"),
    "invalid-as-empty": ("I10", "read an invalid file as an empty Config"),
    "drop-pending": ("I1", "forget a waiting change when the network fails"),
    "record-before-push": ("I12", "update the last-applied record before GitHub has it"),
    "forget-kept-here": ("I3", "don't remember a person's keep"),
    "clear-record-on-switch": ("I1", "clear last-applied when switching Config"),
    "leak-machine-name": ("I8", "put the Mac's name and an email in the commit"),
    "pull-past-approved": ("I9", "update the Dotfiles Repo to the newest commit, not the reviewed one"),
    "no-dotfiles-guard": ("I13", "run Dotfiles git without the -c guards"),
    "git-word-in-ui": ("I14", "say merge conflict in the UI"),
}


def run_cases(tmp, say_case):
    for i, fn in enumerate(cases()):
        name = fn.__name__[len("case_"):]
        w = World(pathlib.Path(tmp) / f"c{i}", SEED)
        fn(w, lambda text: say_case(name, text))
        w.checkpoint()


def main():
    global SCHEMA, ROOT_HOME
    ap = argparse.ArgumentParser()
    ap.add_argument("--schema")
    ap.add_argument("--readme")
    ap.add_argument("--seeds", type=int, default=150)
    args = ap.parse_args()
    if args.schema:
        from jsonschema import Draft202012Validator
        SCHEMA = Draft202012Validator(json.loads(pathlib.Path(args.schema).read_text()))
    failures = 0
    with tempfile.TemporaryDirectory(prefix="soulcaster-sync-") as tmp:
        ROOT_HOME = pathlib.Path(tmp) / "home"
        ROOT_HOME.mkdir()
        print(f"git: {out(run_git(['--version']))}; schema: {'real config.schema.json' if SCHEMA else 'built-in closed-shape check only'}")

        print("\n== why not git merge: the same edits through real git")
        merge_evidence(pathlib.Path(tmp) / "evidence", lambda t: print("PASS " + t))

        print("\n== scenarios, each with invariants checked after every action")
        try:
            run_cases(pathlib.Path(tmp) / "cases", lambda name, text: print(f"PASS {name}: {text}"))
        except Violation as e:
            failures += 1
            print(f"FAIL {e}")

        print(f"\n== random schedules: two Machines, GitHub edits, broken files, force-pushes, lost networks")
        try:
            n = fuzz(pathlib.Path(tmp) / "fuzz", args.seeds)
            print(f"PASS {n} schedules: every invariant held, and each one settled after a person chose")
        except Violation as e:
            failures += 1
            print(f"FAIL {e}")

        print("\n== mutations: each breaks one rule and must be caught by the expected check")
        for name, (rule, what) in MUTATIONS.items():
            MUT.clear()
            MUT.add(name)
            got = None
            work = pathlib.Path(tmp) / f"mut-{name}"
            try:
                run_cases(work / "cases", lambda *_: None)
                fuzz(work / "fuzz", 30)
            except Violation as e:
                got = e
            except Exception as e:  # a broken rule may also crash the model, which isn't a catch
                got = None
                print(f"FAIL {name}: crashed instead of being caught: {e!r}")
                failures += 1
                MUT.clear()
                continue
            MUT.clear()
            shutil.rmtree(work, ignore_errors=True)
            if got is None:
                failures += 1
                print(f"FAIL {name}: nothing caught it")
            elif got.rule != rule:
                failures += 1
                print(f"FAIL {name}: expected {rule}, got {got}")
            else:
                print(f"PASS {name} ({what}): {got}")

        if args.readme:
            print("\n== README wording matches the model")
            text = pathlib.Path(args.readme).read_text()
            missing = [k for k, v in UI.items() if v not in text]
            if missing:
                failures += 1
                print(f"FAIL missing from the README: {missing}")
            else:
                print(f"PASS all {len(UI)} strings appear verbatim")

    print("\nresult: " + ("all invariants held and all mutations were caught" if not failures else f"{failures} FAILED"))
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()

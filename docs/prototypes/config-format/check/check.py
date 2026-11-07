#!/usr/bin/env python3
"""PROTOTYPE, not the engine. Validates the Config examples and tests the rules a schema can't express.

Everything below "the rules" is a small model of what the engine must do. It reads no real Machine, except that
real_process_checks runs real git and zsh inside a temp folder. Rules tested here are still unbuilt in Rust.

    check.py <docs/prototypes/config-format> [--recipe-schema FILE] [--corpus FILE]

--recipe-schema also validates the stub against the Item lifecycle's recipe schema.
--corpus writes the conformance and fuzz cases for check-ajv.mjs and the Rust harness.
"""
import argparse, copy, difflib, hashlib, json, math, os, pathlib, random, re, subprocess, sys, tempfile, unicodedata
from jsonschema import Draft202012Validator

args = argparse.ArgumentParser()
args.add_argument("root")
args.add_argument("--recipe-schema")
args.add_argument("--corpus")
args = args.parse_args()

root = pathlib.Path(args.root).resolve()
repo = root.parents[2]
schema = json.loads((root / "config.schema.json").read_text(encoding="utf-8"))
Draft202012Validator.check_schema(schema)
validator = Draft202012Validator(schema)
stub = json.loads((root / "catalog-stub.json").read_text(encoding="utf-8"))
CATALOG = {r["id"]: r for r in stub["recipes"]}
ALIASES = {"nvim": "neovim"}  # In the engine this comes from Homebrew's metadata. nvim is a real alias.
DEFAULT_CLONE = "~/Developer/dotfiles"
failures = 0


def report(ok, line):
    global failures
    failures += 0 if ok else 1
    print(f"{'PASS' if ok else 'FAIL'} {line}")


print("schema: valid against the 2020-12 metaschema")


def all_patterns(node):
    if isinstance(node, dict):
        for k, v in node.items():
            if k == "pattern" and isinstance(v, str):
                yield v
            else:
                yield from all_patterns(v)
    elif isinstance(node, list):
        for v in node:
            yield from all_patterns(v)


LOOKAROUND = re.compile(r"\(\?[=!<]")
pats = list(all_patterns(schema))
report(not any(LOOKAROUND.search(p) for p in pats), f"schema: none of its {len(pats)} patterns uses lookaround")

# ---------------------------------------------------------------- strict parser
MAX_BYTES = 1024 * 1024


class ParseRefused(ValueError):
    pass


def strict_parse(data):
    """Plain RFC 8259 with no duplicate keys, no NaN or Infinity, finite numbers, and a size cap before parsing."""
    if len(data) > MAX_BYTES:
        raise ParseRefused(f"file is over {MAX_BYTES} bytes")

    def no_dupes(pairs):
        keys = [k for k, _ in pairs]
        dupes = {k for k in keys if keys.count(k) > 1}
        if dupes:
            raise ParseRefused(f"duplicate key {sorted(dupes)}")
        return dict(pairs)

    def no_constant(name):
        raise ParseRefused(f"{name} isn't JSON")

    def finite(text, convert):
        if math.isinf(float(text)):
            raise ParseRefused(f"number out of range: {text[:20]}")
        return convert(text)

    try:
        value = json.loads(
            data.decode("utf-8"),
            object_pairs_hook=no_dupes,
            parse_constant=no_constant,
            parse_float=lambda s: finite(s, float),
            parse_int=lambda s: finite(s, int),
        )
    except RecursionError:
        raise ParseRefused("nested too deeply")
    except UnicodeDecodeError as e:
        raise ParseRefused(f"not UTF-8: {e}")
    except json.JSONDecodeError as e:
        raise ParseRefused(str(e))

    def no_lone_surrogates(v):
        if isinstance(v, str):
            try:
                v.encode("utf-8")
            except UnicodeEncodeError:
                raise ParseRefused("lone surrogate in a string")
        elif isinstance(v, list):
            for x in v:
                no_lone_surrogates(x)
        elif isinstance(v, dict):
            for k, x in v.items():
                no_lone_surrogates(k)
                no_lone_surrogates(x)

    no_lone_surrogates(value)
    return value


def strict_load(path):
    return strict_parse(path.read_bytes())


# ---------------------------------------------------------------- canonical form
ORDER = ["$schema", "schemaVersion", "name", "items", "packages", "settings", "dotfiles"]


def canonical(c):
    """The one form the app writes. It never adds $schema, so a reviewed write stays small."""
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
        elif k == "dotfiles":
            kept = {"repo": v["repo"]}
            if v.get("links"):
                kept["links"] = dict(sorted(v["links"].items()))
            if "installScript" in v:
                kept["installScript"] = v["installScript"]
            v = kept
        if v != [] and v != {}:
            out[k] = v
    return json.dumps(out, indent=2, ensure_ascii=False) + "\n"


# ================================================================ the rules
# ---------------------------------------------------------------- catalog helpers
def covers(cat):
    """Package key to the Items that cover it."""
    out = {}
    for r in cat.values():
        for key in r.get("coversPackages", []):
            out.setdefault(key, []).append(r["id"])
    return out


def follow(entry_id, cat):
    """Old IDs resolve at read time, forever. Returns None on a rename cycle, which Catalog lint also refuses."""
    seen = [entry_id]
    while "renamedTo" in cat.get(seen[-1], {}):
        nxt = cat[seen[-1]]["renamedTo"]
        if nxt in seen:
            return None
        seen.append(nxt)
    return seen[-1]


def base_ids(cat):
    return {i for i, r in cat.items() if r.get("base")}


def package_key(kind, name, aliases=ALIASES):
    if kind == "formulae":
        return "formula:" + aliases.get(name, name)
    return "cask:" + name


def tap_of(name):
    return "/".join(name.split("/")[:2]) if name.count("/") == 2 else None


def value_error(e, value):
    kind = e["valueType"]
    if kind == "boolean":
        return None if isinstance(value, bool) else "expected a boolean"
    if kind == "integer":
        if isinstance(value, bool) or not isinstance(value, int):
            return "expected an integer"
        if not e["minimum"] <= value <= e["maximum"]:
            return f"{value} is outside {e['minimum']} to {e['maximum']}"
        return None
    if kind == "string-list" and not isinstance(value, list):
        return "expected a list"
    if kind == "string" and not isinstance(value, str):
        return "expected a string"
    for v in value if kind == "string-list" else [value]:
        if not isinstance(v, str):
            return "expected a string"
        if "values" in e and v not in e["values"]:
            return f"{v!r} isn't an allowed value"
        if "pattern" in e and not re.search(e["pattern"], v):
            return f"{v!r} doesn't match"
    return None


def suggestion(raw, cat):
    close = difflib.get_close_matches(raw, sorted(cat), n=1, cutoff=0.8)
    return f" Did you mean {close[0]}?" if close else ""


# ---------------------------------------------------------------- resolve
def resolve(c, cat=CATALOG, record=None, aliases=ALIASES):
    """Pure function of the Config, the Catalog and the local record. It reads nothing from the Machine or the network."""
    record = record or {}
    r = dict(
        formulae=set(), casks=set(), recipes=set(), localValues=set(), needsYou=[], taps={}, notes=[],
        wanted=set(), kept=set(), unknown=[], held=[], conflicts=[], pulledIn={}, skippedSettings=set(),
    )
    live_covers = {k: [i for i in v if "retired" not in cat[i]] for k, v in covers(cat).items()}
    seen_items = set()

    def want(kind, name, via):
        r[kind].add(name)
        tap = tap_of(name)
        if tap:
            r["taps"].setdefault(tap, []).append(via)

    def install(entry_id, via):
        """Install one Item, pulling in whatever it requires. Base Items are always there, so they're not pulled."""
        if entry_id in seen_items or entry_id in base_ids(cat):
            return
        seen_items.add(entry_id)
        e = cat[entry_id]
        for q in e.get("requires", []):
            if q not in base_ids(cat) and q not in seen_items:
                if q not in selected:
                    r["pulledIn"].setdefault(q, entry_id)
                install(q, f"Item {q}, pulled in by {entry_id}" if q not in selected else f"Item {q}")
        for s in e.get("steps", []):
            if s["type"] == "brew-formula":
                want("formulae", s["name"], via)
            elif s["type"] == "brew-cask":
                want("casks", s["name"], via)
            else:
                r["recipes"].add(entry_id)
            if s["type"] == "needs-you":
                r["needsYou"].append(s["text"])
        for v in e.get("values", []):
            if e.get("kind") == "machine-values":
                r["localValues"].add(v["name"])

    def resolvable(raw):
        """Returns the live ID to use, or None after adding its note."""
        entry_id = follow(raw, cat)
        if entry_id is None:
            r["notes"].append(f"rename cycle at {raw}: skipped and kept in the file")
            r["kept"].add(raw)
            return None
        e = cat.get(entry_id)
        if e is None:
            r["unknown"].append(raw)
            r["kept"].add(raw)
            r["notes"].append(f"unknown ID {raw}: skipped and kept in the file.{suggestion(raw, cat)}")
            return None
        r["kept"].add(entry_id)
        if raw in record.get("unknownAtReview", []):
            r["held"].append(entry_id)
            r["notes"].append(f"{raw} was unknown when reviewed and now resolves: held for review as a new selection")
            return None
        if raw != entry_id:
            r["notes"].append(f"renamed {raw} -> {entry_id}: resolved, and the file isn't rewritten")
        if "retired" in e:
            r["notes"].append(f"retired {raw}: skipped and kept, nothing uninstalled. {e['retired']['message']} Replacement offered: {e['retired']['replacedBy']}")
            return None
        if "deprecated" in e:
            r["notes"].append(f"deprecated {raw}: still installed, replacement offered: {e['deprecated']['replacedBy']}")
        return entry_id

    selected = []
    for raw in c.get("items", []):
        entry_id = resolvable(raw)
        if entry_id is None:
            continue
        if entry_id in base_ids(cat):
            r["notes"].append(f"{raw} is always installed: ignored, and dropped at the next reviewed write")
            continue
        if cat[entry_id].get("kind") == "setting":
            r["notes"].append(f"{raw} is a setting, not an Item: skipped and kept in the file")
            continue
        if entry_id not in selected:
            selected.append(entry_id)
        r["wanted"].add(entry_id)

    packages = {}
    pk = c.get("packages", {})
    for kind in ("formulae", "casks"):
        for name in pk.get(kind, []):
            key = package_key(kind, name, aliases)
            r["kept"].add(key)
            owners = live_covers.get(key, [])
            if owners and owners[0] in base_ids(cat):
                r["notes"].append(f"{name} is always installed: ignored, and dropped at the next reviewed write")
                continue
            short = name.rsplit("/", 1)[-1]
            if tap_of(name) and short in base_ids(cat):
                r["notes"].append(f"warning: {name} has the same name as always-installed {short}, and is a different program")
            if owners:
                r["notes"].append(f"{name} is covered by Item {owners[0]}: installed once. An explicit Tidy can move it.")
                r["kept"].add(owners[0])
                r["wanted"].add(owners[0])
                if owners[0] not in selected:
                    selected.append(owners[0])
                continue
            packages[key] = (kind, aliases.get(name, name) if kind == "formulae" else name)

    # Per-entry conflicts, reported before a Run. Neither side installs until a person picks one.
    for entry_id in list(selected):
        for key in cat[entry_id].get("conflictsWith", []):
            if key in packages:
                r["conflicts"].append((entry_id, key))
                r["notes"].append(f"conflict: {entry_id} and {key} can't both be installed. Neither is, until you pick one.")
                selected.remove(entry_id)
                r["wanted"].discard(entry_id)
                del packages[key]
                break

    for entry_id in selected:
        install(entry_id, f"Item {entry_id}")
    for key, (kind, name) in packages.items():
        r["wanted"].add(key)
        want(kind, name, f"package {name}")

    # Settings, after following renames. Two spellings of one setting with different values skip both.
    by_id = {}
    for raw, value in c.get("settings", {}).items():
        entry_id = follow(raw, cat)
        by_id.setdefault(entry_id if entry_id else raw, []).append((raw, value))
    for entry_id, spellings in by_id.items():
        if len(spellings) > 1:
            names = " and ".join(raw for raw, _ in spellings)
            r["kept"].add(entry_id)
            if len({json.dumps(v, sort_keys=True) for _, v in spellings}) > 1:
                r["notes"].append(f"{names} are one setting with different values: both skipped and kept in the file")
                r["skippedSettings"].add(entry_id)
                continue
            r["notes"].append(f"{names} are one setting with the same value: applied once")
        raw, value = spellings[-1]
        live = resolvable(raw)
        if live is None:
            continue
        e = cat[live]
        if e.get("kind") != "setting":
            r["notes"].append(f"{raw} isn't a setting: skipped and kept in the file")
            continue
        err = value_error(e, value)
        if err:
            r["notes"].append(f"bad value for {raw}: {err}: skipped and kept in the file")
            r["skippedSettings"].add(live)
            continue
        if e["evidence"]["group"] == "broken":
            r["notes"].append(f"{raw} doesn't work on this macOS: unavailable here. {e.get('notes', [''])[0]}")
            r["skippedSettings"].add(live)
            continue
        r["wanted"].add(live)
        for q in e.get("requires", []):
            if q not in base_ids(cat) and q not in seen_items:
                r["pulledIn"].setdefault(q, live)
                r["notes"].append(f"{q} is pulled in by {raw}")
                install(q, f"Item {q}, pulled in by {raw}")
    d = c.get("dotfiles", {})
    r["dotfiles"] = {"repo": d["repo"], "links": sorted(d.get("links", {})), "script": d.get("installScript")} if d else None
    return r


# ---------------------------------------------------------------- lint stage
SECRET_KEY = re.compile(r"(?:^|[.-])(?:tokens?|secrets?|passwords?|passwd|pass|pwd|passphrases?|credentials?|api-keys?|apikeys?|private-keys?|auth|license-keys?|sudo)(?:[.-]|$)")
TOKEN = re.compile(r"(?:ghp_|gho_|ghs_|ghu_|ghr_|github_pat_|glpat-|xox[abprs]-|AIza|sk_live_|sk_test_|npm_|hf_|pypi-)[A-Za-z0-9_-]{8,}|sk-[A-Za-z0-9_-]{20,}|A(?:KIA|SIA)[0-9A-Z]{16}|-----BEGIN|\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.|\bBearer\s+\S{8,}")
EMAIL = re.compile(r"^\s*[^@\s]+@[^@\s]+\.[^@\s]+\s*$")
HOME_PATH = re.compile(r"^\s*(?:file://)?/Users/")
PROTECTED = [
    "~/Applications", "~/Desktop", "~/Documents", "~/Downloads", "~/Library", "~/Movies", "~/Music", "~/Pictures", "~/Public",
    "~/.ssh", "~/.gnupg", "~/.Trash", "~/.config", "~/.local", "~/.local/bin", "~/.local/share",
    "~/Library/Application Support", "~/Library/Containers", "~/Library/Group Containers", "~/Library/Keychains",
    "~/Library/LaunchAgents", "~/Library/Mobile Documents", "~/Library/Preferences",
]


def fold(path):
    """How macOS's default volume compares names: case-insensitive and normalization-insensitive."""
    return unicodedata.normalize("NFC", path).casefold()


def overlaps(a, b):
    return a == b or a.startswith(b + "/") or b.startswith(a + "/")


def link_findings(links, clone=DEFAULT_CLONE):
    """Per-link checks. A refused link is skipped and reported, and the rest of the Config carries on."""
    out = []
    folded = {t: fold(t) for t in links}
    protected = {fold(p) for p in PROTECTED}
    clone_f = fold(clone)
    names = sorted(links)
    for i, a in enumerate(names):
        for b in names[i + 1:]:
            if folded[a] == folded[b]:
                out.append(("link-duplicate", a, f"{a} and {b} are the same location on this Mac"))
            elif overlaps(folded[a], folded[b]):
                out.append(("link-overlap", a, f"{a} and {b} overlap, so one would land inside the other"))
    for t in names:
        if folded[t] in protected:
            out.append(("link-protected", t, f"{t} holds user data, so it can't be replaced by a link"))
        if overlaps(folded[t], clone_f):
            out.append(("link-clone", t, f"{t} is at, above or inside the Dotfiles Repo's clone location {clone}"))
    return out


def written_paths(e):
    """Files a Catalog entry writes to, for the rule that a written value never goes through a link into the repo."""
    stores = list(e.get("stores", [])) + [v["store"] for v in e.get("values", []) if isinstance(v, dict) and "store" in v]
    for s in stores:
        if s["type"] == "git-config":
            yield "~/.gitconfig"
        elif s["type"] == "defaults" and s["domain"] != "NSGlobalDomain":
            yield f"~/Library/Preferences/{s['domain']}.plist"


def collision_findings(c, cat):
    out = []
    links = {fold(t): t for t in c.get("dotfiles", {}).get("links", {})}
    ids = [(follow(i, cat), i) for i in c.get("items", [])] + [(follow(i, cat), i) for i in c.get("settings", {})]
    for entry_id, raw in ids:
        hits = {(path, target) for path in written_paths(cat.get(entry_id, {})) for lf, target in links.items() if overlaps(fold(path), lf)}
        for path, target in sorted(hits):
            out.append(("writes-through-link", raw, f"{raw} writes {path}, which {target} links into the Dotfiles Repo: unavailable here"))
    return out


def secret_findings(c, cat):
    """Best effort. A false negative is expected, and the real guard is typed fields and scanning writes."""
    out = []

    def known(entry):
        return follow(entry, cat) in cat

    def scan(where, value, typed=None):
        if not isinstance(value, str):
            return
        if TOKEN.search(value):
            out.append(("secret-value", where, f"{where} looks like a token"))
        elif typed is None and EMAIL.match(value):
            out.append(("email-value", where, f"{where} looks like an email address"))
        elif typed is None and HOME_PATH.match(value):
            out.append(("home-path-value", where, f"{where} names a home folder, so it won't work on another Mac"))

    if "name" in c:
        scan("name", c["name"])
    for key, value in c.get("settings", {}).items():
        if key.startswith("git.user."):
            out.append(("machine-value", key, f"{key} is personal and stays on the Machine"))
            continue
        if SECRET_KEY.search(key) and not known(key):
            out.append(("secret-key", key, f"{key} looks like a secret's name"))
        e = cat.get(follow(key, cat) or "", {})
        for v in value if isinstance(value, list) else [value]:
            # A value the Catalog enumerates can't be a secret someone typed. One it only patterns still can.
            if "values" in e and v in e["values"]:
                continue
            scan(key, v)
    d = c.get("dotfiles", {})
    for target, source in d.get("links", {}).items():
        scan(target, target, typed=True)
        scan(target, source, typed=True)
    if "repo" in d:
        scan("dotfiles.repo", d["repo"], typed=True)
    return out


def changed_entries(before, after):
    """Top-level names and setting IDs this write adds or changes. Entries already in the file are left alone."""
    out = {"name"} if before.get("name") != after.get("name") else set()
    b, a = before.get("settings", {}), after.get("settings", {})
    out |= {k for k in a if k not in b or a[k] != b[k]}
    bl, al = before.get("dotfiles", {}).get("links", {}), after.get("dotfiles", {}).get("links", {})
    out |= {k for k in al if k not in bl or al[k] != bl[k]}
    if before.get("dotfiles", {}).get("repo") != after.get("dotfiles", {}).get("repo"):
        out.add("dotfiles.repo")
    return out


def lint_write(before, after, cat):
    """Hard stop on an app write, for only what it adds or changes. So a value from a newer Catalog can't block Adopt."""
    changed = changed_entries(before, after)
    return [f for f in secret_findings(after, cat) if f[1] in changed]


def lint_read(c, cat, clone=DEFAULT_CLONE):
    """Per-entry skip on read: returns every finding, and the file itself still loads."""
    return secret_findings(c, cat) + link_findings(c.get("dotfiles", {}).get("links", {}), clone) + collision_findings(c, cat)


# ---------------------------------------------------------------- catalog lint
KEY_SPELLING = re.compile(r"^(?:formula|cask):")


def catalog_lint(cat):
    """Rules over the whole Catalog. A real Catalog would run these in CI."""
    problems = []
    ids = base_ids(cat)
    if ids != {"command-line-tools", "homebrew", "gh"}:
        problems.append(f"base Items must be exactly Command Line Tools, Homebrew and gh, got {sorted(ids)}")
    for i, e in cat.items():
        if "renamedTo" in e:
            if follow(i, cat) is None:
                problems.append(f"{i}: rename cycle")
                continue
            target = cat.get(e["renamedTo"])
            if target is None:
                problems.append(f"{i}: renamedTo names an unknown ID")
            elif ("." in i) != ("." in e["renamedTo"]):
                problems.append(f"{i}: a rename can't change an Item into a setting")
            continue
        if e.get("kind") == "sign-in":
            problems.append(f"{i}: there is no sign-in kind")
        for field in ("retired", "deprecated"):
            if field in e and e[field].get("replacedBy") not in cat:
                problems.append(f"{i}: {field}.replacedBy names an unknown ID")
        if "retired" in e and "checks" not in e and "stores" not in e:
            problems.append(f"{i}: a retired Item keeps its detection data")
        for q in e.get("requires", []):
            if q not in cat:
                problems.append(f"{i}: unknown requirement {q}")
        if e.get("kind") == "setting":
            if not e.get("stores"):
                problems.append(f"{i}: a setting needs a read path")
            if e.get("valueType") == "integer" and not ("minimum" in e and "maximum" in e):
                problems.append(f"{i}: an integer setting needs bounds")
            if "pattern" in e:
                if LOOKAROUND.search(e["pattern"]):
                    problems.append(f"{i}: a pattern can't use lookaround")
                try:
                    re.compile(e["pattern"])
                except re.error:
                    problems.append(f"{i}: pattern doesn't compile")
        names = [("formula:" if s["type"] == "brew-formula" else "cask:") + s["name"] for s in e.get("steps", []) if s["type"].startswith("brew-")]
        for key in names:
            if key not in e.get("coversPackages", []):
                problems.append(f"{i}: coversPackages must list {key}, which its steps install")
        for field in ("coversPackages", "conflictsWith"):
            for key in e.get(field, []):
                if not KEY_SPELLING.match(key):
                    problems.append(f"{i}: {field} entry {key} needs formula: or cask:")
        for key in set(e.get("conflictsWith", [])) & set(e.get("coversPackages", [])):
            problems.append(f"{i}: {key} can't be both covered and a conflict")
    for key, owners in covers(cat).items():
        live = [o for o in owners if "retired" not in cat[o]]
        if len(live) > 1:
            problems.append(f"{key} is covered by more than one Item: {live}")
    return problems


# ---------------------------------------------------------------- Drift and Adopt, modelled
def migrate_key(key, cat, aliases=ALIASES):
    """Record keys from an older Catalog or spelling read as today's key, so a rename or a new Item isn't a removal."""
    if key.startswith("formula:"):
        key = "formula:" + aliases.get(key[8:], key[8:])
    owners = covers(cat).get(key)
    if owners:
        live = [o for o in owners if "retired" not in cat[o]]
        return live[0] if live else owners[0]
    return follow(key, cat) or key


def compare_list(want, have, unavailable):
    """List settings compare only on entries available on this Machine."""
    return [x for x in want if x not in unavailable] == have


def adopt_list(want, have, unavailable):
    """Adopt keeps entries this Machine can't show, in place, so they stay for other Machines."""
    out = list(have)
    for i, x in enumerate(want):
        if x in unavailable:
            out.insert(min(i, len(out)), x)
    return out


def offers(c, machine, record, cat, aliases=ALIASES):
    """What the app shows for one Config, Machine and record. Returns a sorted list of (offer, entry)."""
    r = resolve(c, cat, record, aliases)
    out = []
    unavailable = {migrate_key(k, cat, aliases): v for k, v in record.get("unavailable", {}).items()}
    last = {migrate_key(k, cat, aliases): v for k, v in record.get("lastApplied", {}).items()}
    holds_unknown = bool(r["unknown"])
    if holds_unknown:
        out.append(("update-the-app", ",".join(sorted(r["unknown"]))))
    installed = machine.get("installed", set())
    base = base_ids(cat)
    cov = covers(cat)

    def item_present(i):
        return i in installed or any(k in installed for k in cat.get(i, {}).get("coversPackages", []))

    # Dropped from the Config. Never for a base Item, and never while the file holds an ID this app doesn't know.
    for key in sorted(last):
        if key in r["wanted"] or key in r["kept"] or key in unavailable or key in base:
            continue
        if key in r["skippedSettings"]:
            continue
        if not holds_unknown and (key in machine.get("settings", {}) or key in installed or item_present(key)):
            out.append(("dropped-from-config", key))

    # In the Config but missing here. An unavailable entry shows on its own line, with no Apply or Adopt.
    for key in sorted(r["wanted"]):
        if key in unavailable:
            out.append(("unavailable-here", f"{key}: {unavailable[key]}"))
            continue
        e = cat.get(key, {})
        if e.get("kind") == "setting":
            continue
        if key.startswith(("formula:", "cask:")):
            present = key in installed
        else:
            present = item_present(key)
        if not present:
            out.append(("missing", key))

    # Settings. Lists compare only on what's available here.
    for raw, want in c.get("settings", {}).items():
        key = follow(raw, cat)
        if key not in r["wanted"] or key in unavailable:
            continue
        have = machine.get("settings", {}).get(key)
        if have is None:
            continue
        tiles = machine.get("unavailableListEntries", {}).get(key, {})
        if isinstance(want, list):
            for tile, why in sorted(tiles.items()):
                out.append(("unavailable-here", f"{key}: {tile}: {why}"))
            if not compare_list(want, have, tiles):
                out.append(("setting-differs", key))
        elif have != want:
            out.append(("setting-differs", key))

    # On the Machine and not in the Config.
    claimed = set()
    for i in r["wanted"]:
        claimed |= set(cat.get(i, {}).get("coversPackages", []))
    for q in r["pulledIn"]:
        claimed |= set(cat.get(q, {}).get("coversPackages", []))
    for key in sorted(k for k in installed if k.startswith(("formula:", "cask:"))):
        if key in r["wanted"] or key in claimed or key in record.get("pulledIn", []):
            continue
        owners = cov.get(key, [])
        if any(o in base or "retired" in cat[o] for o in owners):
            continue  # always installed, or a retired Item's leftover: never Drift, and never offered to Adopt
        if migrate_key(key, cat, aliases) in last:
            continue
        if holds_unknown:
            continue  # the file may already name this through an ID this app can't read
        out.append(("adopt-into-items" if owners else "adopt-into-packages", owners[0] if owners else key))
    return sorted(set(out))


def adopt_setting(c, setting_id, value, cat):
    """Validate every Adopt result before it's committed. A value with no valid spelling isn't adoptable."""
    after = copy.deepcopy(c)
    after.setdefault("settings", {})[setting_id] = value
    e = cat.get(setting_id)
    if e is None or e.get("kind") != "setting":
        return None, "not a setting this Catalog exposes"
    err = value_error(e, value)
    if err:
        return None, f"not adoptable: {err}"
    errors = list(validator.iter_errors(after))
    if errors:
        return None, f"not adoptable: {errors[0].message[:80]}"
    stop = lint_write(c, after, cat)
    if stop:
        return None, f"not adoptable: {stop[0][2]}"
    return after, None


def reviewed_write(c, cat):
    """What any reviewed write does beyond the change itself: drop base entries. It never renames or moves entries."""
    after = copy.deepcopy(c)
    base = base_ids(cat)
    if "items" in after:
        after["items"] = [i for i in after["items"] if follow(i, cat) not in base]
    for kind in ("formulae", "casks"):
        names = after.get("packages", {}).get(kind)
        if names is not None:
            after["packages"][kind] = [
                n for n in names
                if not any(o in base for o in covers(cat).get(package_key(kind, n), []))
            ]
    return canonical(after)


def tidy(c, cat, aliases=ALIASES):
    """The explicit, separate action: follow renames, move packages to the Items that cover them, resolve aliases."""
    after = copy.deepcopy(c)
    items = {follow(i, cat) or i for i in after.get("items", [])}
    after_pk = {}
    for kind in ("formulae", "casks"):
        kept = set()
        for name in after.get("packages", {}).get(kind, []):
            key = package_key(kind, name, aliases)
            live = [o for o in covers(cat).get(key, []) if "retired" not in cat[o]]
            if live:
                items.add(live[0])
            else:
                kept.add(aliases.get(name, name) if kind == "formulae" else name)
        after_pk[kind] = sorted(kept)
    after["items"] = sorted(items)
    after["packages"] = after_pk
    after["settings"] = {follow(k, cat) or k: v for k, v in after.get("settings", {}).items()}
    return canonical(after)


# ---------------------------------------------------------------- Import, Git identity, repo checks
def import_approvals(c, cat, signed_in_login):
    """The three separate approvals. Only the signed-in account's own Dotfiles Repo starts on."""
    r = resolve(c, cat)
    d = r["dotfiles"]
    out = {"taps": {t: False for t in sorted(r["taps"])}, "dotfiles": None}
    if d:
        out["dotfiles"] = {**d, "startsOn": repo_owner(d["repo"]) == (signed_in_login or "").casefold() and bool(signed_in_login)}
    return out


def repo_owner(repo_name):
    """Owner of a GitHub repo, or None. The engine must also compare GitHub's canonical owner, in case of a transfer."""
    m = re.match(r"^https://github\.com/([^/]+)/[^/]+$", repo_name, re.I)
    if m:
        return m.group(1).casefold()
    if repo_name.startswith("https://"):
        return None
    return repo_name.split("/")[0].casefold()


def fill_git_identity(existing, local_file, github):
    """Existing git config, then the local file, then the GitHub account, then the queue. It never prompts."""
    values, queue = {}, []
    for name in ("git.user.name", "git.user.email"):
        for source in (existing, local_file, github):
            if source.get(name):
                values[name] = source[name]
                break
        else:
            queue.append(name)
    return values, queue


def repo_symlink_problems(repo_dir, links, script=None):
    """Repo symlinks that resolve outside the repo can't be linked or run. Checked at the approved commit."""
    top = os.path.realpath(repo_dir)
    out = []
    for source in list(links.values()) + ([script] if script else []):
        real = os.path.realpath(os.path.join(repo_dir, source))
        if real != top and not real.startswith(top + os.sep):
            out.append(source)
    return out


def writes_through_link(path, clone):
    """True if writing to this path would change a file inside the Dotfiles Repo."""
    real, top = os.path.realpath(path), os.path.realpath(clone)
    return real == top or real.startswith(top + os.sep)


def real_process_checks():
    """Runs real git and zsh with a sandboxed HOME, to show the guards stop what the review reproduced."""
    guard = {"GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_NOSYSTEM": "1"}
    with tempfile.TemporaryDirectory(prefix="soulcaster-check-") as tmp:
        tmp = pathlib.Path(tmp).resolve()
        home, work = tmp / "home", tmp / "work"
        home.mkdir()
        work.mkdir()
        mark = tmp / "marker"
        hook = tmp / "hook.sh"
        hook.write_text(f"#!/bin/sh\ntouch {mark}\n")
        hook.chmod(0o755)
        (home / ".gitconfig").write_text(f'[core]\n\tfsmonitor = {hook}\n[url "{tmp}/attacker"]\n\tinsteadOf = https://github.com/acme/tap\n')
        (home / ".zshenv").write_text(f"touch {mark}\n")
        base_env = {"HOME": str(home), "PATH": os.environ["PATH"], "GIT_CONFIG_NOSYSTEM": "1"}

        def git(*cmd, env=None):
            return subprocess.run(["git", *cmd], cwd=work, env={**base_env, **(guard if env is None else env)}, capture_output=True, text=True, timeout=30)

        def fires(run):
            mark.unlink(missing_ok=True)
            run()
            return mark.exists()

        git("init", "-q")
        (work / "f").write_text("x")
        git("add", "f")
        git("-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-qm", "x")
        report(fires(lambda: git("status", env={})), "real git: a linked ~/.gitconfig runs core.fsmonitor on plain git status")
        report(not fires(lambda: git("status")), "real git: GIT_CONFIG_GLOBAL=/dev/null and GIT_CONFIG_NOSYSTEM=1 stop it")
        git("remote", "add", "origin", "https://github.com/acme/tap")
        rewritten = git("remote", "get-url", "origin", env={}).stdout.strip()
        guarded = git("remote", "get-url", "origin").stdout.strip()
        report(rewritten == f"{tmp}/attacker", "real git: a linked url.insteadOf silently redirects a clone")
        report(guarded == "https://github.com/acme/tap", "real git: the guard keeps the URL the preview showed")
        zsh = lambda *a: subprocess.run(["zsh", *a], env={"HOME": str(home), "PATH": os.environ["PATH"]}, capture_output=True, timeout=30)
        report(fires(lambda: zsh("-c", "true")), "real zsh: a linked ~/.zshenv runs on zsh -c")
        report(not fires(lambda: zsh("-f", "-c", "true")), "real zsh: zsh -f doesn't run it")

        # Rule 7: a real git write goes through a link into the repo unless the engine checks first.
        clone = home / "Developer" / "dotfiles"
        (clone / "git").mkdir(parents=True)
        (clone / "git" / "gitconfig").write_text("")
        linked = home / ".linked-gitconfig"
        linked.symlink_to(clone / "git" / "gitconfig")
        subprocess.run(["git", "config", "--file", str(linked), "user.email", "you@example.com"], env=base_env, check=True, timeout=30)
        report("you@example.com" in (clone / "git" / "gitconfig").read_text(), "real git: writing a linked config changes the file in the repo")
        report(writes_through_link(linked, clone), "writes-through-link: detects that link")
        report(not writes_through_link(home / ".gitconfig", clone), "writes-through-link: ignores a plain file")

        # Rule 2: repo symlinks that leave the repo.
        (clone / "inside").write_text("")
        (clone / "good").symlink_to("inside")
        (clone / "bad").symlink_to("/etc/passwd")
        (clone / "climb").symlink_to("../../../outside")
        (clone / "dir").symlink_to("/etc")
        got = repo_symlink_problems(clone, {"~/.a": "good", "~/.b": "bad", "~/.c": "climb", "~/.d": "dir/passwd", "~/.e": "inside"}, "bad")
        report(got == ["bad", "climb", "dir/passwd", "bad"], "repo symlinks: only the ones that leave the repo are refused")


# ================================================================ the checks
print("\n== valid examples: must parse strictly, pass the schema and already be in canonical form")
loaded = {}
for path in sorted(root.glob("examples/*.json")):
    c = strict_load(path)
    errors = list(validator.iter_errors(c))
    canon = canonical(c) == path.read_text(encoding="utf-8")
    report(not errors and canon, f"{path.name}  ({len(path.read_text(encoding='utf-8').splitlines())} lines, canonical: {canon})")
    loaded[path.name] = c
    for e in errors:
        print("     ", e.message[:120])

snippet = re.search(r"```json\n(.*?)```", (root / "README.md").read_text(encoding="utf-8"), re.S).group(1)
report(validator.is_valid(strict_parse(snippet.encode())), "README's example Config passes the schema")

print("\n== invalid examples: each must be refused")
for path in sorted(root.glob("examples/invalid/*.json")):
    try:
        c = strict_load(path)
    except ParseRefused as e:
        report(True, f"{path.name}: refused by the strict parser: {e}")
        continue
    errors = list(validator.iter_errors(c))
    where = "/".join(map(str, errors[0].absolute_path)) or "(top level)" if errors else ""
    report(bool(errors), f"{path.name}: refused at {where}" if errors else f"{path.name}: ACCEPTED")

print("\n== conformance: shared with Ajv and Rust")
conformance = json.loads((root / "conformance.json").read_text())


def build_corpus():
    """Expands conformance.json, and adds deterministic fuzz cases that all three validators must answer alike."""
    cases = []
    for case in conformance["schemaCases"]:
        cases.append({"label": case["label"], "valid": case["valid"], "config": {"schemaVersion": 1, "name": "probe", **case["fields"]}})
    for case in conformance["sizeCases"]:
        c = {"schemaVersion": 1, "name": "probe"}
        if case.get("repo"):
            c["dotfiles"] = {"repo": "example/dotfiles"}
        node = c
        for key in case["path"][:-1]:
            node = node.setdefault(key, {})
        names = [case["template"].format(i) for i in range(case["count"])]
        node[case["path"][-1]] = {n: ("x" if case.get("repo") else True) for n in names} if case.get("map") else names
        cases.append({"label": case["label"], "valid": case["valid"], "config": c})
    parser = []
    for case in conformance["parserCases"]:
        text = case["text"] * case.get("repeat", 1)
        text += " " * max(0, case.get("padTo", 0) - len(text.encode("utf-8")))
        parser.append({"label": case["label"], "valid": case["valid"], "text": text})
    rng = random.Random(20261106)
    seeds = {
        "item": ["ghostty", "git-identity", "a", "a-b-c", "gh"],
        "setting": ["dock.autohide", "a.b", "hot-corners.top-left"],
        "formula": ["ripgrep", "openssl@3", "bonnie++", "a/b/c", "homebrew/core/x", "jetbrains/homebrew-utils/x", "evil.rb"],
        "cask": ["firefox", "nikitabobko/tap/aerospace"],
        "repo": ["example/dotfiles", "https://git.example.com:8443/a/b.git", "example/..", "a/.github"],
        "target": ["~/.zshrc", "~/Library/Mobile Documents/com~apple~CloudDocs/x", "~/.config/Übersicht", "~/a (b)/c", "~/.git/config"],
        "source": ["zsh/zshrc", ".zshrc", "a (b)/c", "über/x"],
        "script": ["install.sh", "script/setup", ".git/x"],
        "name": ["Work Mac", "Übersicht", "x"],
        "value": ["left", "35.0.0", "sk-SK", "/Users/me/x", "ghp_abc"],
    }
    nasty = ["\n", "\r", "\t", " ", " ", "\u0000", "\u001b", "\u007f", "\u0085", "\u009b", "­", "​", "‍", "‮", " ", " ", "⁦", "﻿", "\U000e0001", "\U0001f5a5", "Ü", "Ü",
             "/", "//", "..", ".", "~", "-", "--", "_", "@", "+", ":", ";", "$", "`", "\\", "\"", "'", "(", ")", "[", "]", "{", "}", "*", "?", "|", "<", ">", "=", "%", "#", "!", "^", "&", ",",
             ".git", ".GIT", "/.git/", "/.Git", ".rb", "homebrew/", "homebrew-", "A", "z", "0", "https://", "git@", "x" * 70, "x" * 300]

    def mutate(s):
        for _ in range(rng.choice((0, 1, 1, 1, 2, 3))):
            piece, at = rng.choice(nasty), rng.randrange(len(s) + 1)
            kind = rng.random()
            if kind < 0.55:
                s = s[:at] + piece + s[at:]
            elif kind < 0.8 and s:
                s = s[:at] + s[at + 1:]
            elif kind < 0.9:
                s = piece + s
            else:
                s = s + piece
        return s

    def config_for(field, s):
        c = {"schemaVersion": 1, "name": "probe"}
        if field == "item":
            c["items"] = [s]
        elif field == "setting":
            c["settings"] = {s: True}
        elif field == "formula":
            c["packages"] = {"formulae": [s]}
        elif field == "cask":
            c["packages"] = {"casks": [s]}
        elif field == "repo":
            c["dotfiles"] = {"repo": s}
        elif field == "target":
            c["dotfiles"] = {"repo": "a/b", "links": {s: "x"}}
        elif field == "source":
            c["dotfiles"] = {"repo": "a/b", "links": {"~/.x": s}}
        elif field == "script":
            c["dotfiles"] = {"repo": "a/b", "installScript": s}
        elif field == "name":
            c["name"] = s
        else:
            c["settings"] = {"a.b": s}
        return c

    fuzz = []
    fields = sorted(seeds)
    for i in range(40000):
        field = fields[i % len(fields)]
        fuzz.append(config_for(field, mutate(rng.choice(seeds[field]))))
    return {"cases": cases, "parser": parser, "fuzz": fuzz}


corpus = build_corpus()
wrong = [c["label"] for c in corpus["cases"] if validator.is_valid(c["config"]) != c["valid"]]
report(not wrong, f"{len(corpus['cases'])} schema and size cases behaved as expected" + (f": wrong {wrong}" if wrong else ""))
for c in corpus["cases"]:
    if c["label"] in wrong:
        print("     ", c["label"], [e.message[:100] for e in validator.iter_errors(c["config"])][:2])
wrong = []
for case in corpus["parser"]:
    try:
        got = validator.is_valid(strict_parse(case["text"].encode("utf-8", "surrogatepass")))
    except ParseRefused:
        got = False
    if got != case["valid"]:
        wrong.append(case["label"])
report(not wrong, f"{len(corpus['parser'])} parser cases behaved as expected" + (f": wrong {wrong}" if wrong else ""))
verdicts = "".join("1" if validator.is_valid(c) else "0" for c in corpus["fuzz"])
print(f"fuzz: {len(verdicts)} cases, {verdicts.count('1')} accepted, sha256 {hashlib.sha256(verdicts.encode()).hexdigest()}")
if args.corpus:
    pathlib.Path(args.corpus).write_text(json.dumps(corpus, ensure_ascii=True))

print("\n== lint examples: pass the schema, then the rules flag exactly these")
LINT_EXPECT = {
    "secret-shaped-key.json": ["secret-key"],
    "secret-shaped-value.json": ["secret-value"],
    "git-identity-in-settings.json": ["machine-value"],
    "home-folder-value.json": ["home-path-value"],
    "email-value.json": ["email-value"],
    "token-in-name.json": ["secret-value"],
    "link-protected-folder.json": ["link-protected", "link-protected"],
    "link-case-duplicate.json": ["link-duplicate"],
    "link-unicode-duplicate.json": ["link-duplicate"],
    "link-ancestor-pair.json": ["link-overlap"],
    "link-at-clone-location.json": ["link-clone", "link-clone", "link-clone", "link-overlap", "link-overlap", "link-overlap"],
    "write-through-link.json": ["writes-through-link", "writes-through-link"],
    "base-items-listed.json": [],
    "item-package-conflict.json": [],
    "rename-collision.json": [],
    "sign-in-item.json": [],
}
RESOLVE_EXPECT = {
    "base-items-listed.json": ["gh is always installed", "homebrew is always installed", "gh is always installed", "evil/tap/gh has the same name"],
    "item-package-conflict.json": ["conflict: opencode", "conflict: tailscale"],
    "rename-collision.json": ["are one setting with different values"],
    "sign-in-item.json": ["unknown ID gh-sign-in"],
}
lint_files = sorted(root.glob("examples/lint/*.json"))
report({p.name for p in lint_files} == set(LINT_EXPECT), f"{len(lint_files)} lint examples, all with expected results")
for path in lint_files:
    c = strict_load(path)
    schema_ok = validator.is_valid(c)
    got = sorted(f[0] for f in lint_read(c, CATALOG))
    want = sorted(LINT_EXPECT.get(path.name, []))
    notes = resolve(c)["notes"]
    wanted_notes = RESOLVE_EXPECT.get(path.name, [])
    notes_ok = all(sum(w in n for n in notes) >= wanted_notes.count(w) for w in wanted_notes)
    report(schema_ok and got == want and notes_ok, f"{path.name}: {', '.join(got) or 'resolve notes only'}")
    if not (schema_ok and got == want and notes_ok):
        print("     ", got, notes, [e.message[:100] for e in validator.iter_errors(c)][:2])

print("\n== Catalog stub: lint and mutations")
report(not catalog_lint(CATALOG), f"{len(CATALOG)} entries pass Catalog lint" + (f": {catalog_lint(CATALOG)}" if catalog_lint(CATALOG) else ""))
if args.recipe_schema:
    recipe_schema = json.loads(pathlib.Path(args.recipe_schema).read_text())
    recipe_validator = Draft202012Validator(recipe_schema)
    OWNED = ("renamedTo", "deprecated", "retired", "coversPackages", "conflictsWith")
    recipes = [{k: v for k, v in r.items() if k not in OWNED} for r in stub["recipes"] if "renamedTo" not in r]
    errs = list(recipe_validator.iter_errors({"schemaVersion": 1, "recipes": recipes}))
    report(not errs, f"{len(recipes)} recipes pass the Item lifecycle's recipe schema, without the five fields this draft owns" + (f": {errs[0].message[:160]}" if errs else ""))


def mutated(fn):
    cat = copy.deepcopy(CATALOG)
    fn(cat)
    return catalog_lint(cat)


def set_(i, **kw):
    return lambda cat: cat[i].update(kw)


CATALOG_BAD = {
    "a rename cycle": lambda cat: (cat.update({"loop-a": {"id": "loop-a", "renamedTo": "loop-b"}, "loop-b": {"id": "loop-b", "renamedTo": "loop-a"}})),
    "a rename to nowhere": lambda cat: cat.update({"gone": {"id": "gone", "renamedTo": "not-there"}}),
    "a rename from an Item to a setting": lambda cat: cat.update({"was-item": {"id": "was-item", "renamedTo": "dock.autohide"}}),
    "a fourth base Item": set_("ghostty", base=True),
    "a sign-in kind": lambda cat: cat.update({"gh-sign-in": {"id": "gh-sign-in", "kind": "sign-in"}}),
    "an integer setting with no bounds": lambda cat: cat["keyboard.key-repeat"].pop("minimum"),
    "a setting with no read path": lambda cat: cat["dock.autohide"].pop("stores"),
    "a pattern with lookahead": set_("git.default-branch", pattern="^(?!-).+$"),
    "a retired Item with no detection data": lambda cat: [cat["example-retired-tool"].pop("checks"), cat["example-retired-tool"].pop("steps")],
    "a package covered by two Items": set_("ghostty", coversPackages=["cask:ghostty", "formula:opencode"]),
    "a step's package missing from coversPackages": set_("ghostty", coversPackages=[]),
    "a badly spelled conflict": set_("tailscale", conflictsWith=["tailscale"]),
    "a replacement that doesn't exist": lambda cat: cat["example-retired-tool"]["retired"].update(replacedBy="not-there"),
    "a requirement that doesn't exist": lambda cat: cat["dock.apps"].update(requires=["not-there"]),
}
for label, fn in CATALOG_BAD.items():
    problems = mutated(fn)
    report(bool(problems), f"refuses {label}: {problems[0] if problems else 'NOT REFUSED'}")

print("\n== rules the schema can't express, modelled")


def test_catalog(**extra):
    cat = copy.deepcopy(CATALOG)
    cat.update(extra)
    return cat


def tool(i, formula_name="tool", **extra):
    return {"id": i, "kind": "cli", "title": i, "requires": ["homebrew"], "checks": [{"type": "brew-formula", "name": formula_name}],
            "steps": [{"type": "brew-formula", "name": formula_name}], "coversPackages": [f"formula:{formula_name}"], **extra}


CFG = lambda **f: {"schemaVersion": 1, "name": "probe", **f}

# Finding 3: base Items.
gh_machine = {"installed": {"formula:gh"}}
report(offers(loaded["minimal.json"], {"installed": {"formula:gh", "cask:ghostty"}}, {}, CATALOG) == [], "base: minimal.json doesn't want gh, and gh on the Machine is no Drift")
report(offers(CFG(), gh_machine, {"lastApplied": {"gh": True}}, CATALOG) == [], "base: dropping gh from a Config offers no removal")
report(offers(CFG(), gh_machine, {}, CATALOG) == [], "base: gh is never offered to Adopt")
r = resolve(CFG(items=["gh", "ghostty"], packages={"formulae": ["gh"]}))
report("gh" not in r["wanted"] and "formula:gh" not in r["wanted"] and sum("always installed" in n for n in r["notes"]) == 2, "base: entries naming gh are ignored with a note")
after = json.loads(reviewed_write(CFG(items=["gh", "ghostty"], packages={"formulae": ["gh", "jq"]}), CATALOG))
report(after["items"] == ["ghostty"] and after["packages"]["formulae"] == ["jq"], "base: they're dropped at the next reviewed write")
r = resolve(CFG(packages={"formulae": ["evil/tap/gh"]}))
report("formula:evil/tap/gh" in r["wanted"] and any("warning" in n for n in r["notes"]), "base: a third-party gh warns and is still its own package")
report(not import_approvals(CFG(items=["gh", "homebrew"]), CATALOG, None)["taps"] and not resolve(CFG(items=["gh", "homebrew", "command-line-tools"]))["recipes"], "base: Import shows no choice for them")

# Finding 8: no sign-in kind.
report(not [i for i, e in CATALOG.items() if e.get("kind") == "sign-in"] and not {"gh-sign-in", "tailscale-sign-in"} & set(CATALOG), "sign-in: the kind and both IDs are gone")
report(any("Apple Account" in s["text"] for s in CATALOG["xcode"]["steps"] if s["type"] == "needs-you"), "sign-in: Xcode's Apple Account stays a Needs-You step")
report(not any(s["type"] == "needs-you" for s in CATALOG["tailscale"]["steps"]), "sign-in: Tailscale is install-only, with a note and no queue step")

# Finding 4: version skew.
newer = test_catalog(**{"tool-old": {"id": "tool-old", "renamedTo": "tool-new"}, "tool-new": tool("tool-new")})
older = test_catalog(**{"tool-old": tool("tool-old")})
machine = {"installed": {"formula:tool"}}
record = {"lastApplied": {"tool-old": True}}
got = offers(CFG(items=["tool-new"]), machine, record, older)
report(got == [("update-the-app", "tool-new")], "skew: an older app reading a renamed ID offers no removal and no Adopt, only an update")
report(offers(CFG(items=["tool-old"]), machine, record, newer) == [], "skew: a newer app reading the old ID sees no Drift")
before = CFG(items=["tool-old"])
report(json.loads(reviewed_write(before, newer))["items"] == ["tool-old"], "skew: a reviewed write doesn't rewrite the old ID")
report(json.loads(tidy(before, newer))["items"] == ["tool-new"], "skew: rewriting is the explicit Tidy action")
gained = test_catalog(ripgrep=tool("ripgrep", formula_name="ripgrep"))
file_a = canonical(CFG(packages={"formulae": ["ripgrep"]}))
states = []
text = file_a
for _ in range(3):
    text = reviewed_write(json.loads(text), gained)  # newer app
    states.append(text)
    text = reviewed_write(json.loads(text), CATALOG)  # older app
    states.append(text)
report(len(set(states)) == 1 and states[0] == file_a, "skew: a package gaining an Item no longer rewrites back and forth (6 rounds, 1 form)")
report(offers(CFG(packages={"formulae": ["ripgrep"]}), {"installed": {"formula:ripgrep"}}, {"lastApplied": {"formula:ripgrep": True}}, gained) == [], "skew: the record key migrates, so it doesn't read as dropped")
report(offers(CFG(items=["ripgrep"]), {"installed": {"formula:ripgrep"}}, {"lastApplied": {"ripgrep": True}}, CATALOG) == [("update-the-app", "ripgrep")], "skew: an older app holding the new Item offers no package Adopt")
r = resolve(CFG(items=["example-from-a-newer-catalog"]), CATALOG, {"unknownAtReview": []})
report("example-from-a-newer-catalog" in r["unknown"], "skew: an unknown ID is skipped and kept")
later = test_catalog(**{"example-from-a-newer-catalog": tool("example-from-a-newer-catalog")})
r = resolve(CFG(items=["example-from-a-newer-catalog"]), later, {"unknownAtReview": ["example-from-a-newer-catalog"]})
report(r["held"] == ["example-from-a-newer-catalog"] and not r["formulae"], "skew: an ID unknown at Import that later resolves is held for review, and not installed")
r = resolve(CFG(items=["ghosty"]))
report(any("Did you mean ghostty" in n for n in r["notes"]), "typos: a suggestion appears before a Run")

# Finding 5: lint stage, not schema.
known = test_catalog(**{k: {"id": k, "kind": "setting", "title": k, "valueType": "boolean", "stores": [{"type": "builtin", "name": "library-visible"}], "evidence": {"group": "probable", "confirmedOn": []}} for k in ("screensaver.require-password", "safari.autofill-passwords")},
                     **{"system.locale": {"id": "system.locale", "kind": "setting", "title": "Locale", "valueType": "string", "stores": [{"type": "builtin", "name": "library-visible"}], "evidence": {"group": "probable", "confirmedOn": []}}})
report(not secret_findings(CFG(settings={"screensaver.require-password": True, "safari.autofill-passwords": False, "system.locale": "sk-SK"}), known), "lint: sk-SK and Catalog settings with secret-looking names pass")
for bad in ("ghp_abcdefghijklmnop", " ghp_abcdefghijklmnop", "Bearer abcdefghijklmnop", "sk_live_abcdefghijkl", "npm_abcdefghijkl", "hf_abcdefghijkl", "eyJhbGciOiJIUzI1.eyJzdWIiOiIxMjM0.sig", "-----BEGIN OPENSSH PRIVATE KEY-----", "AKIAABCDEFGHIJKLMNOP"):
    report(bool(secret_findings(CFG(settings={"example.value": bad}), CATALOG)), f"lint: flags {bad[:20]!r}")
report(any(f[0] == "secret-key" for f in secret_findings(CFG(settings={"sudo.pass": "x"}), CATALOG)), "lint: flags a key named sudo.pass")
report(any(f[0] == "secret-value" for f in secret_findings(CFG(dotfiles={"repo": "a/b", "links": {"~/.x": "ghp_abcdefghijklmnop"}}), CATALOG)), "lint: flags a token in a link source")
report(bool(lint_write(CFG(), CFG(settings={"example.value": "ghp_abcdefghijklmnop"}), CATALOG)), "lint: an app write adding a token is a hard stop")
stale = CFG(settings={"safari.autofill-passwords": False})
report(not lint_write(stale, {**stale, "items": ["ghostty"]}, CATALOG), "lint: an unrelated write isn't blocked by what's already in the file")
c = CFG(settings={"example.token": "ghp_abcdefghijklmnop", "dock.autohide": True})
report(validator.is_valid(c) and any(f[1] == "example.token" for f in lint_read(c, CATALOG)) and "dock.autohide" in resolve(c)["wanted"], "lint: on read, the flagged entry is reported and the rest of the file still applies")
got, why = adopt_setting(CFG(), "dock.apps", ["/Users/me/Applications/Foo.app"], CATALOG)
report(got is None and "not adoptable" in why, "Adopt: a home-folder Dock tile isn't adoptable, so the file stays valid")
got, why = adopt_setting(CFG(), "dock.apps", ["~/Applications/Foo.app"], CATALOG)
report(got is not None and validator.is_valid(got), "Adopt: the same tile written with ~/ is adoptable")
got, why = adopt_setting(CFG(), "dock.apps", ["/Volumes/X/Foo.app"], CATALOG)
report(got is None, "Adopt: a tile with no valid spelling isn't adoptable")
got, why = adopt_setting(CFG(), "keyboard.key-repeat", -5, CATALOG)
report(got is None, "Adopt: an out-of-range value isn't adoptable")
got, why = adopt_setting(CFG(), "dock.autohide", True, CATALOG)
report(got is not None, "Adopt: a normal value is adoptable")

# Finding 6: unavailable here.
tiles = ["/System/Applications/Launchpad.app", "/Applications/Safari.app"]
machine = {"settings": {"dock.apps": ["/Applications/Safari.app"]}, "unavailableListEntries": {"dock.apps": {tiles[0]: "not on this Mac"}}, "installed": {"formula:dockutil"}}
got = offers(CFG(settings={"dock.apps": tiles}), machine, {"pulledIn": ["formula:dockutil"]}, CATALOG)
report(got == [("unavailable-here", f"dock.apps: {tiles[0]}: not on this Mac")], "unavailable: a missing Dock tile is reported apart from Drift, with no Adopt")
report(adopt_list(tiles, ["/Applications/Safari.app", "/Applications/Zed.app"], {tiles[0]}) == [tiles[0], "/Applications/Safari.app", "/Applications/Zed.app"], "unavailable: Adopt on a list keeps the tile for other Machines")
got = offers(CFG(items=["xcode"]), {"installed": set()}, {"unavailable": {"xcode": "no Apple Account here"}}, CATALOG)
report(got == [("unavailable-here", "xcode: no Apple Account here")], "unavailable: xcode on a Machine with no Apple Account shows no Drift and no Adopt")
report("unavailable" not in schema["properties"], "unavailable: it lives only in the local record")
rng = random.Random(6)
ok = True
for _ in range(300):
    c = CFG(items=rng.sample(["ghostty", "tailscale", "xcode", "junie"], 2))
    rec = {"unavailable": {k: "x" for k in rng.sample(["ghostty", "xcode", "not-in-the-file", "formula:evil/tap/x"], 2)}, "lastApplied": {"not-in-the-file": True}}
    wanted = resolve(c, CATALOG, rec)["wanted"]
    ok = ok and wanted <= set(c["items"])
    ok = ok and not any(x[0] in ("missing", "setting-differs") and x[1] in rec["unavailable"] for x in offers(c, {"installed": set()}, rec, CATALOG))
report(ok, "unavailable: 300 random records only ever suppress entries, and never add one")
report("sidebar" not in " ".join(resolve(CFG(settings={"finder.sidebar-favorites": ["~/Developer"]}))["wanted"]), "unavailable: a broken setting isn't applied")
report(not value_error(CATALOG["hot-corners.top-left"], "mission-control"), "unavailable: a real hot corner now has a spelling")

# Finding 7: writes through a link.
report(not collision_findings(loaded["developer.json"], CATALOG), "write-through: developer.json no longer combines git-identity with a linked ~/.gitconfig")
got = collision_findings(CFG(settings={"dock.autohide": True}, dotfiles={"repo": "a/b", "links": {"~/Library/Preferences/com.apple.dock.plist": "dock.plist"}}), CATALOG)
report(len(got) == 1, "write-through: a linked defaults plist is reported too")

# Findings 12, 16, 17: overlap, collisions, requires.
r = resolve(CFG(items=["junie"], packages={"formulae": ["jetbrains/junie/junie"]}))
report(r["casks"] == {"junie"} and not r["formulae"] - {"jetbrains/junie/junie"} and not r["taps"], "overlap: a covered package installs once, through the Item")
r = resolve(CFG(settings={"dock.show-recents": False, "example.old-setting": False}))
report(any("same value" in n for n in r["notes"]) and "dock.show-recents" in r["wanted"], "rename collision: the same value applies once")
loop = test_catalog(**{"loop-a": {"id": "loop-a", "renamedTo": "loop-b"}, "loop-b": {"id": "loop-b", "renamedTo": "loop-a"}})
report(any("rename cycle" in n for n in resolve(CFG(items=["loop-a"]), loop)["notes"]), "rename cycle: it terminates, skips the entry and reports it")
r = resolve(CFG(settings={"dock.apps": ["/Applications/Safari.app"]}))
report(r["pulledIn"].get("dockutil") == "dock.apps" and "dockutil" in r["formulae"], "requires: dock.apps pulls in dockutil, shown with why")
third = test_catalog(**{"example.tap-setting": {"id": "example.tap-setting", "kind": "setting", "title": "x", "valueType": "boolean", "stores": [{"type": "builtin", "name": "library-visible"}], "evidence": {"group": "probable", "confirmedOn": []}, "requires": ["aerospace"]}})
report("nikitabobko/tap" in resolve(CFG(settings={"example.tap-setting": True}), third)["taps"], "requires: a tap pulled in by a setting still needs approval")

# Finding 14: retired Items.
r = resolve(CFG(items=["example-retired-tool"]))
report(not r["formulae"] and any("retired example-retired-tool" in n for n in r["notes"]), "retired: skipped, and kept in the file")
report(offers(CFG(), {"installed": {"formula:example-retired-formula"}}, {}, CATALOG) == [], "retired: its leftover package isn't offered to Adopt")
report(offers(CFG(items=["example-retired-tool"]), {"installed": {"formula:example-retired-formula"}}, {"lastApplied": {"example-retired-tool": True}}, CATALOG) == [], "retired: an entry in the file isn't read as dropped")
report(any("retired example.retired-setting" in n for n in resolve(CFG(settings={"example.retired-setting": True}))["notes"]), "retired: a retired setting gets the right note")

# Finding 15: aliases.
r = resolve(CFG(packages={"formulae": ["neovim", "nvim"]}))
report(r["formulae"] == {"neovim"}, "alias: nvim and neovim are one package")
report(offers(CFG(packages={"formulae": ["nvim"]}), {"installed": {"formula:neovim"}}, {"lastApplied": {"formula:nvim": True}}, CATALOG) == [], "alias: nvim shows no Drift against neovim")
report(json.loads(tidy(CFG(packages={"formulae": ["nvim"]}), CATALOG))["packages"]["formulae"] == ["neovim"], "alias: only Tidy rewrites it")

# Finding 20: stub bounds.
for setting_id, value in (("keyboard.key-repeat", -5), ("keyboard.initial-key-repeat", 1), ("git.excludes-file", "~/../../../etc/passwd")):
    report(bool(value_error(CATALOG[setting_id], value)), f"stub: {setting_id} refuses {value!r}")
report(not value_error(CATALOG["keyboard.key-repeat"], 2) and not value_error(CATALOG["keyboard.initial-key-repeat"], 15) and not value_error(CATALOG["git.excludes-file"], "~/.gitignore_global"), "stub: the owner's real values still pass")

# Finding 22 and the Dotfiles approval.
report("links" not in json.loads(canonical(CFG(dotfiles={"repo": "a/b", "links": {}})))["dotfiles"], "canonical: empty links are dropped")
report("$schema" not in json.loads(canonical(CFG())), "canonical: the app doesn't add $schema to a file that has none")
for repo_name, login, expected in (("you/dotfiles", "you", True), ("You/dotfiles", "you", True), ("https://github.com/you/dotfiles", "you", True), ("stranger/dotfiles", "you", False),
                                   ("https://github.com.evil.example/you/dotfiles", "you", False), ("https://git.example.com/you/dotfiles", "you", False), ("you/dotfiles", None, False)):
    got = import_approvals(CFG(dotfiles={"repo": repo_name, "links": {"~/.x": "x"}}), CATALOG, login)["dotfiles"]["startsOn"]
    report(got == expected, f"Import: {repo_name} with sign-in {login} starts {'on' if expected else 'off'}")
hostile = CFG(dotfiles={"repo": "stranger/dotfiles", "links": {"~/.zshenv": "zshenv", "~/.gitconfig": "gitconfig", "~/.ssh/authorized_keys": "keys", "~/Library/LaunchAgents/x.plist": "x.plist", "~/.local/bin/sudo": "sudo"}})
got = import_approvals(hostile, CATALOG, "you")["dotfiles"]
report(got["startsOn"] is False and len(got["links"]) == 5, "Import: a hostile Dotfiles block needs approval, even with no taps or script")
report(fill_git_identity({"git.user.name": "A"}, {"git.user.name": "B", "git.user.email": "b@x.dev"}, {"git.user.email": "c@x.dev"}) == ({"git.user.name": "A", "git.user.email": "b@x.dev"}, []), "Git identity: existing config, then the local file, then GitHub")
report(fill_git_identity({}, {}, {"git.user.name": "G"}) == ({"git.user.name": "G"}, ["git.user.email"]), "Git identity: a blank value goes to the queue, with no prompt")

print("\n== real git and zsh, with a sandboxed HOME")
real_process_checks()

print("\n== resolving against the Catalog stub")
for name, c in loaded.items():
    r = resolve(c)
    print(f"{name}: {len(r['formulae'])} formulae, {len(r['casks'])} casks, {len(r['recipes'])} recipes, {len(c.get('settings', {}))} settings, {len(c.get('dotfiles', {}).get('links', {}))} links")
    d = r["dotfiles"]
    print(f"  separate approval: taps {sorted(r['taps'])}, dotfiles {d['repo'] + ' with script ' + str(d['script']) if d else None}")
    print(f"  local values filled: {sorted(r['localValues'])}")
    if r["pulledIn"]:
        print(f"  pulled in: {r['pulledIn']}")
    for note in r["notes"]:
        print("  note:", note)

print("\n== Brewfile coverage: every entry must resolve from the translated Configs")
MOVED = {"azure-cli": "casks", "junie": "casks"}  # azure-cli moved to a cask on 2026-10-06. junie uses the official cask.
BASE_PACKAGES = {"gh"}  # Always installed, so a Config doesn't list it.
UNNEEDED_TAPS = {"jetbrains/junie", "jetbrains/utils"}  # junie no longer needs its tap, and nothing used jetbrains/utils.
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
                found = token in r["taps"] or token in UNNEEDED_TAPS
            elif token in BASE_PACKAGES:
                found = True
            else:
                bucket = MOVED.get(token) or ("formulae" if kind == "brew" else "casks")
                found = any(t == token or t.endswith("/" + token) for t in r[bucket])
            if not found:
                missing.append(line)
    report(not missing, f"{role}: {total} entries, missing {missing or 'none'}")
for item, script in (("kotlin", "scripts/kotlin.sh"), ("kotlin-lsp", "scripts/kotlin-lsp.sh"), ("android-sdk", "scripts/android.sh"), ("android-emulator", "scripts/android.sh")):
    report(all(item in loaded[n]["items"] for n in ("owner-personal.json", "owner-work.json")), f"{script}: Item {item} in both Configs")
report(not any(k.startswith("android.") for n in loaded for k in loaded[n].get("settings", {})), "no android.* settings remain")

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

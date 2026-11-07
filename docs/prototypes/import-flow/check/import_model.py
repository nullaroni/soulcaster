"""PROTOTYPE, not the engine. The Import rules this ticket decides, as small pure functions.

check.py runs these against the sample Configs and writes app/plans.json, so the screens show what the rules
produced rather than hand-typed results. They read no Machine and no network. The strict parser, resolve() and
lint_read() are not reimplemented here: check.py lifts them from the Config format's own check.py at the pinned
commit and passes them in as `cf`.
"""
import hashlib, json, re, unicodedata

# ---------------------------------------------------------------- list sizes
BIG_LIST = 100  # A section longer than this starts off. The owner's real Configs have 16 formulae and 7 casks.

# ---------------------------------------------------------------- what a link does, shown beside it
# This explains. It doesn't decide anything: a link with no note isn't safe, and no list could say which are.
# It's not the blocklist the Config format rejected, because nothing here is refused because of it.
LINK_NOTES = [
    (r"^~/\.zshenv$", "Runs for every zsh, even in scripts"),
    (r"^~/\.(zshrc|zprofile|zlogin|bashrc|bash_profile|profile)$", "Runs whenever a terminal opens"),
    (r"^~/(\.gitconfig|\.config/git/config)$", "Can run programs during git commands and redirect clones"),
    (r"^~/\.ssh/authorized_keys2?$", "Lets whoever holds these keys log in to this Mac"),
    (r"^~/\.ssh/config$", "Decides where SSH connects, and can run programs"),
    (r"^~/Library/LaunchAgents/", "Starts a program every time you log in"),
    (r"^~/\.local/bin/sudo$", "Can stand in for sudo and capture your password"),
    (r"^~/(\.local/bin|bin)/", "Can stand in for a command you type"),
]


def link_note(target):
    for pattern, note in LINK_NOTES:
        if re.search(pattern, target):
            return note
    return None


# ---------------------------------------------------------------- text from the file is data
SCRIPTS = ("LATIN", "CYRILLIC", "GREEK")


def letter_script(ch):
    name = unicodedata.name(ch, "")
    for s in SCRIPTS:
        if name.startswith(s):
            return s
    return None


def lookalikes(text):
    """Letters from Cyrillic or Greek in a string that also has Latin ones. This is UTS #39's mixed-script test, cut down."""
    scripts = {letter_script(c) for c in text if c.isalpha()} - {None}
    if "LATIN" not in scripts or len(scripts) < 2:
        return []
    return [c for c in text if c.isalpha() and letter_script(c) not in ("LATIN", None)]


def escaped(text):
    """The file's text with anything outside printable ASCII written as a code point, so it can't pass as something else."""
    return "".join(c if 0x20 <= ord(c) < 0x7f else f"\\u{ord(c):04x}" for c in text)


CLAIMS = re.compile(r"\b(soulcaster|verified|official|certified|endorsed|approved|trusted)\b", re.I)


def skeleton(text):
    """A rough confusables folding. Real work would use UTS #39's table."""
    s = text.lower()
    for a, b in (("vv", "w"), ("rn", "m"), ("0", "o"), ("1", "l"), ("5", "s")):
        s = s.replace(a, b)
    return s


NAMES_WORTH_FAKING = ("homebrew", "github", "apple", "soulcaster")


def fakes_a_name(owner):
    folded = skeleton(owner)
    return next((n for n in NAMES_WORTH_FAKING if n in folded), None)


# ---------------------------------------------------------------- signs a file is built to mislead
def base_names(cat):
    names = {i for i, r in cat.items() if r.get("base")}
    for r in cat.values():
        if r.get("base"):
            names |= {k.split(":", 1)[1] for k in r.get("coversPackages", [])}
    return names


def signals(c, source, taps, cat):
    """Objective and few. Finding one starts everything off. Finding none says nothing, and the UI never claims it does."""
    out = []
    name = c.get("name", "")
    if lookalikes(name):
        out.append(("name-lookalike", f"The name mixes alphabets so its letters pass for others: {escaped(''.join(lookalikes(name)))}"))
    claim = CLAIMS.search(name)
    if claim:
        out.append(("name-claim", f"The name says \"{claim.group(0)}\". Soulcaster doesn't verify or endorse any Config."))
    faked = fakes_a_name(source["owner"])
    if faked:
        out.append(("source-lookalike", f"The account {source['owner']} has a name like {faked}, and nothing connects it to {faked}."))
    for t in taps:
        faked = fakes_a_name(t["owner"])
        if faked:
            out.append(("tap-lookalike", f"The tap {t['tap']} has a name like {faked}, and isn't from {faked}."))
        for p in t["packages"]:
            if p["sameAsBase"]:
                out.append(("same-as-base", f"{p['name']} shares its name with {p['short']}, which Soulcaster always installs. It's a different program."))
    d = c.get("dotfiles")
    if d:
        host = repo_host(d["repo"])
        if host != "github.com" and "github" in skeleton(host):
            out.append(("host-lookalike", f"The Dotfiles Repo's address starts like GitHub, but the site is {registrable(host)}."))
        for target in d.get("links", {}):
            if lookalikes(target):
                out.append(("link-lookalike", f"A link location mixes alphabets: {escaped(target)}"))
    return out


# ---------------------------------------------------------------- dotfiles identity
def repo_host(repo):
    return repo.split("/")[2].split(":")[0] if repo.startswith("https://") else "github.com"


def repo_owner(repo):
    if repo.startswith("https://github.com/"):
        return repo.split("/")[3]
    if repo.startswith("https://"):
        return None
    return repo.split("/")[0]


def registrable(host):
    """Last two labels. Real work needs the public suffix list."""
    return ".".join(host.split(".")[-2:])


def links_digest(links):
    return hashlib.sha256(json.dumps(links, sort_keys=True, ensure_ascii=False).encode()).hexdigest()[:12]


# ---------------------------------------------------------------- approvals: where each one starts
def is_own(c_owner, repo, signed_in):
    """The ADR's rule, plus one narrowing: someone else's Config naming your repo still wrote that list of links."""
    owner = repo_owner(repo)
    if not signed_in or not owner:
        return False
    return owner.lower() == signed_in.lower() and (c_owner or "").lower() == signed_in.lower()


def starting_state(plan, record, signed_in):
    """What each box shows when the review opens. Off unless this Machine already approved exactly this, or it's your own."""
    state = {"taps": {}, "repo": False, "script": False}
    record = record or {}
    approved = set(record.get("packages", []))
    for t in plan["taps"]:
        state["taps"][t["tap"]] = not t["blocked"] and all(p["name"] in approved for p in t["packages"])
    d = plan.get("dotfiles")
    if d:
        same = record.get("dotfiles") == {"repo": d["repo"], "commit": d["commit"], "links": d["linksDigest"]}
        own = is_own(plan["source"]["owner"], d["repo"], signed_in)
        state["repo"] = bool(d["commit"]) and (same or (own and not plan["signals"]))
        state["script"] = state["repo"] and record.get("script") == {"repo": d["repo"], "commit": d["commit"], "path": d["script"]}
    return state


def can_tick(plan, state, what):
    """Whether a box can change. An approval is never implied by another one."""
    if what == "repo":
        return bool(plan.get("dotfiles") and plan["dotfiles"]["commit"])
    if what == "script":
        d = plan.get("dotfiles")
        return bool(d and d.get("script") and d.get("scriptText") and state["repo"])
    return not next(t for t in plan["taps"] if t["tap"] == what)["blocked"]


def run_contents(plan, state, safe_on=True):
    """What a Run started from this state would do. The one function that turns boxes into work."""
    out = {"taps": [], "packages": [], "linked": [], "script": None, "clone": None, "safe": safe_on}
    for t in plan["taps"]:
        if state["taps"].get(t["tap"]) and not t["blocked"]:
            out["taps"].append(t["tap"])
            out["packages"] += [p["name"] for p in t["packages"]]
    d = plan.get("dotfiles")
    if d and state["repo"] and d["commit"]:
        out["clone"] = f"{d['repo']}@{d['commit']}"
        out["linked"] = [l["target"] for l in d["links"] if not l["refused"]]
        if state["script"] and d.get("script") and d.get("scriptText"):
            out["script"] = d["script"]
    return out


def record_after_run(plan, state, record=None):
    """What Start Run writes to the Machine Record's approvals. Keep a copy writes nothing."""
    out = dict(record or {})
    run = run_contents(plan, state)
    out["packages"] = sorted(set(out.get("packages", [])) | set(run["packages"]))
    d = plan.get("dotfiles")
    if run["clone"]:
        out["dotfiles"] = {"repo": d["repo"], "commit": d["commit"], "links": d["linksDigest"]}
    else:
        out.pop("dotfiles", None)
    if run["script"]:
        out["script"] = {"repo": d["repo"], "commit": d["commit"], "path": d["script"]}
    else:
        out.pop("script", None)
    return out


# ---------------------------------------------------------------- later updates
def dotfiles_status(record, repo, links, script, head, approved_is_ancestor=True, repo_moved_to=None):
    """Where a Dotfiles Repo stands against its approval. Nothing here pulls. The Machine stays at the approved commit."""
    held = record.get("dotfiles")
    if not held:
        return "not-approved"
    if repo_moved_to or held["repo"] != repo:
        return "void-repo-moved"
    if held["links"] != links_digest(links):
        return "void-links-changed"
    if head == held["commit"]:
        s = record.get("script")
        return "current" if not script or (s and s["path"] == script and s["commit"] == head) else "script-not-approved"
    return "behind" if approved_is_ancestor else "history-rewritten"


def update_delta(old, new):
    """What a review of a source's newer version shows. Removal is never a removal, only Drift."""
    def flat(c):
        out = {f"item:{i}": True for i in c.get("items", [])}
        for kind in ("formulae", "casks"):
            out.update({f"{kind}:{n}": True for n in c.get("packages", {}).get(kind, [])})
        out.update({f"setting:{k}": v for k, v in c.get("settings", {}).items()})
        d = c.get("dotfiles") or {}
        if d:
            out["dotfiles.repo"] = d["repo"]
        out.update({f"link:{k}": v for k, v in d.get("links", {}).items()})
        if "installScript" in d:
            out["dotfiles.installScript"] = d["installScript"]
        return out
    a, b = flat(old), flat(new)
    return {
        "added": sorted(set(b) - set(a)),
        "gone": sorted(set(a) - set(b)),
        "changed": sorted(k for k in set(a) & set(b) if a[k] != b[k]),
    }


# ---------------------------------------------------------------- plan: everything the review shows
HUMAN_EFFECT = {"restart-dock": "Dock restarts", "restart-finder": "Finder restarts", "log-out": "After you log out"}
HUMAN_LINT = {
    "link-protected": "This folder holds your data, so a link can't replace it",
    "link-overlap": "Overlaps another link, so one would land inside the other",
    "link-duplicate": "Same location as another link on this Mac",
    "link-clone": "Would write into or over the Dotfiles Repo's own folder",
}


def show_value(v):
    if isinstance(v, bool):
        return "On" if v else "Off"
    return str(v)


def build_plan(sample, raw, source, machine, signed_in, cf, validator):
    """One sample Config through parse, schema, resolve and lint, into what the review shows."""
    plan = {"id": sample.replace(".json", ""), "source": source}

    try:
        c = cf["strict_parse"](raw)
    except cf["ParseRefused"] as e:
        plan["refused"] = {"stage": "parser", "at": "(whole file)", "code": "parser", "detail": str(e)}
        return plan, None

    errors = sorted(validator.iter_errors(c), key=lambda e: list(map(str, e.absolute_path)))
    if errors:
        e = errors[0]
        at = "/".join(map(str, e.absolute_path)) or "(top level)"
        code = "unknown-field" if e.validator == "additionalProperties" else "newer-version" if at == "schemaVersion" else "bad-text" if at == "name" else "schema"
        detail = e.message
        if code == "bad-text":
            detail = escaped(c["name"])
        elif code == "unknown-field":
            found = []
            for err in errors:
                if err.validator == "additionalProperties":
                    prefix = "/".join(map(str, err.absolute_path))
                    allowed = set(err.schema.get("properties", {}))
                    found += [f"{prefix + '/' if prefix else ''}{k}" for k in err.instance if k not in allowed]
            detail = ", ".join(sorted(found))
        plan["refused"] = {"stage": "schema", "at": at, "code": code, "detail": detail, "errors": len(errors)}
        return plan, c

    cat = cf["CATALOG"]
    r = cf["resolve"](c, cat)
    lint = cf["lint_read"](c, cat)
    base = base_names(cat)
    plan["name"] = c["name"]
    plan["nameEscaped"] = escaped(c["name"])
    plan["nameHasLookalikes"] = bool(lookalikes(c["name"]))

    # Third-party code, grouped by tap, with where each came from in the file.
    formulae = c.get("packages", {}).get("formulae", [])
    casks = c.get("packages", {}).get("casks", [])
    taps = {}
    for kind, names, field in (("formula", formulae, "packages.formulae"), ("cask", casks, "packages.casks")):
        for i, name in enumerate(names):
            tap = cf["tap_of"](name)
            if tap:
                short = name.rsplit("/", 1)[-1]
                catalog_twin = short if short in cat and short not in base else None
                taps.setdefault(tap, []).append({
                    "name": name, "short": short, "kind": kind, "via": f"line {i + 1} of {len(names)} in {field}",
                    "sameAsBase": short in base, "sameAsCatalog": catalog_twin,
                })
    for tap, vias in r["taps"].items():
        for via in vias:
            if via.startswith("Item "):
                item = via.split()[1]
                name = next(s["name"] for s in cat[item]["steps"] if s["type"].startswith("brew-") and cf["tap_of"](s["name"]) == tap)
                taps.setdefault(tap, []).append({
                    "name": name, "short": name.rsplit("/", 1)[-1], "kind": "catalog", "via": f"the Catalog's {cat[item].get('title', item)}, listed in this Config",
                    "sameAsBase": False, "sameAsCatalog": None, "item": item,
                })
    plan["taps"] = [
        {"tap": t, "owner": t.split("/")[0], "packages": ps, "blocked": any(p["sameAsBase"] for p in ps),
         "formulaUrl": "https://github.com/{}/homebrew-{}".format(*t.split("/"))}
        for t, ps in sorted(taps.items())
    ]
    plan["signals"] = [{"code": a, "text": b} for a, b in signals(c, source, plan["taps"], cat)]

    # What installs without an approval.
    tap_items = {p.get("item") for t in plan["taps"] for p in t["packages"]}
    catalog, official_f, official_c = [], [], []
    for w in sorted(r["wanted"]):
        if w.startswith("formula:") and "/" not in w:
            official_f.append(w.split(":", 1)[1])
        elif w.startswith("cask:") and "/" not in w:
            official_c.append(w.split(":", 1)[1])
        elif w in cat and cat[w].get("kind") != "setting" and w not in tap_items and w not in r["pulledIn"]:
            catalog.append({"id": w, "title": cat[w].get("title", w), "note": (cat[w].get("notes") or [None])[0]})
    plan["catalog"] = catalog
    plan["officialFormulae"] = official_f
    plan["officialCasks"] = official_c
    plan["bigLists"] = [f for f, names in (("formulae", official_f), ("casks", official_c)) if len(names) > BIG_LIST]
    plan["pulledIn"] = [{"id": k, "title": cat[k].get("title", k), "by": v} for k, v in sorted(r["pulledIn"].items())]

    # Answers the Catalog wants, for what's selected. A Config can never answer them.
    needs = []
    for w in [x["id"] for x in catalog] + [p["id"] for p in plan["pulledIn"]]:
        for s in cat[w].get("steps", []):
            if s["type"] == "accept-license":
                needs.append({"kind": "license", "item": cat[w].get("title", w), "text": s["text"], "link": s.get("link"), "license": s["license"]})
            elif s["type"] == "needs-you":
                needs.append({"kind": "later", "item": cat[w].get("title", w), "text": s["text"]})
    for t in plan["taps"]:
        for p in t["packages"]:
            for s in cat.get(p.get("item"), {}).get("steps", []):
                if s["type"] == "needs-you":
                    needs.append({"kind": "later", "item": cat[p["item"]].get("title"), "text": s["text"], "onlyIf": t["tap"]})
    plan["needs"] = needs

    # Settings: current and new values, and what won't apply.
    wont = []
    settings = []
    lint_by_where = {}
    for code, where, msg in lint:
        lint_by_where.setdefault(where, []).append((code, msg))
    for raw_id, value in c.get("settings", {}).items():
        live = cf["follow"](raw_id, cat)
        e = cat.get(live)
        found = lint_by_where.get(raw_id, [])
        through_link = [f for f in found if f[0] == "writes-through-link"]
        found = [f for f in found if f[0] != "writes-through-link"]
        if found:
            code, msg = found[0]
            text = {"machine-value": "Personal values stay on this Mac, never in a Config. Skipped.", "secret-key": "Its name looks like a secret, so it's skipped. Its value isn't shown."}.get(code, msg)
            wont.append({"kind": code, "id": raw_id, "text": text})
            continue
        if e is None:
            wont.append({"kind": "unknown-setting", "id": raw_id, "text": "Soulcaster doesn't know this setting, so it's skipped. Its value does nothing and isn't shown.", "hiddenValue": escaped(show_value(value))[:256]})
            continue
        if "retired" in e:
            wont.append({"kind": "retired", "id": raw_id, "text": f"{e['retired']['message']} Nothing is uninstalled. Replacement, if you want it: {e['retired']['replacedBy']}."})
            continue
        err = cf["value_error"](e, value)
        if err:
            wont.append({"kind": "bad-value", "id": raw_id, "text": f"The Catalog refuses this value: {err}."})
            continue
        now = machine["settingsNow"].get(live, {"shown": "Not set", "note": ""})
        row = {"id": live, "title": e.get("title", live), "now": now["shown"], "nowNote": now.get("note", ""), "new": show_value(value), "effect": HUMAN_EFFECT.get(e.get("effect"), "Right away"), "same": now["shown"] == show_value(value)}
        if isinstance(value, list):
            keep = {v.rsplit("/", 1)[-1].removesuffix(".app") for v in value}
            row["new"] = f"{len(value)} tiles"
            row["replaces"] = {"leaves": [n for n in now.get("list", []) if n not in keep], "after": sorted(keep)}
        if through_link:
            row["ifRepoOn"] = "Skipped if you turn the Dotfiles Repo on: it would write into a linked file, one push from publishing."
        settings.append(row)
    plan["settings"] = settings

    # Items that won't install. resolve() words these as notes, one per entry.
    for note in r["notes"]:
        if note.startswith("unknown ID") and not any(w["id"] == note.split()[2].rstrip(":") for w in wont):
            nid = note.split()[2].rstrip(":")
            maybe = re.search(r"Did you mean ([^?]+)\?", note)
            text = "Soulcaster doesn't know this ID. It's skipped and kept in your copy." + (f" Did you mean {maybe.group(1)}? It won't be swapped in for you." if maybe else " A newer Soulcaster may know it, and you'll be asked before it installs.")
            wont.append({"kind": "unknown", "id": nid, "text": text})
        elif note.startswith("retired ") and not any(w["id"] == note.split()[1].rstrip(":") for w in wont):
            nid = note.split()[1].rstrip(":")
            e = cat[nid]
            wont.append({"kind": "retired", "id": nid, "text": f"{e['retired']['message']} Nothing is uninstalled. Replacement, if you want it: {e['retired']['replacedBy']}."})
        elif "is always installed" in note:
            nid = note.split()[0]
            wont.append({"kind": "base", "id": nid, "text": "Every Run installs this anyway, so this line does nothing."})
    plan["wontApply"] = wont

    # Dotfiles Repo.
    d = c.get("dotfiles")
    if d:
        host = repo_host(d["repo"])
        refused = {}
        for code, where, msg in lint:
            if code.startswith("link-"):
                refused.setdefault(where, []).append(code)
        # The pair rules name one side. Import skips both, so the file can't choose which one wins.
        names = list(d.get("links", {}))
        for a in names:
            for b in names:
                if a != b and cf["fold"](a) == cf["fold"](b):
                    refused.setdefault(a, []).append("link-duplicate")
        links = [
            {"target": t, "source": src, "note": link_note(t), "lookalike": bool(lookalikes(t)), "targetEscaped": escaped(t),
             "refused": HUMAN_LINT[refused[t][0]] if t in refused else None}
            for t, src in d.get("links", {}).items()
        ]
        links.sort(key=lambda l: (bool(l["refused"]), not (l["note"] or l["lookalike"]), l["target"]))
        plan["dotfiles"] = {
            "repo": d["repo"], "host": host, "isGitHub": host == "github.com",
            "own": is_own(source["owner"], d["repo"], signed_in),
            "ownerLabel": repo_owner(d["repo"]) or host,
            "namesYourRepo": (repo_owner(d["repo"]) or "").lower() == (signed_in or "").lower(),
            "commit": source.get("dotfilesCommit"),
            "links": links, "linksDigest": links_digest(d.get("links", {})),
            "script": d.get("installScript"), "scriptText": source.get("script"),
        }
    else:
        plan["dotfiles"] = None

    plan["counts"] = {
        "install": len(catalog) + len(official_f) + len(official_c) + len(plan["pulledIn"]),
        "settings": len(settings),
        "approvals": sum(1 for t in plan["taps"] if not t["blocked"]) + (1 if d else 0) + (1 if d and d.get("installScript") else 0),
        "wont": len(wont) + (sum(1 for l in plan["dotfiles"]["links"] if l["refused"]) if d else 0),
    }
    return plan, c

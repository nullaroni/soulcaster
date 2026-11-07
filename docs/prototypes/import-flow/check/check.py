#!/usr/bin/env python3
"""PROTOTYPE. Checks the sample Configs for real and tests the Import rules, then writes the data the screens render.

    check.py <docs/prototypes/import-flow> <config-format dir> [--write-plans] [--corrupt NAME]

<config-format dir> holds config.schema.json, catalog-stub.json and check/check.py from prototype/config-format.
The strict parser, resolve() and lint are lifted from that check.py rather than copied, so these results
come from the binding rules. --corrupt breaks one rule on purpose, to show a check catches it.
"""
import argparse, copy, difflib, json, math, pathlib, random, re, sys, unicodedata
from jsonschema import Draft202012Validator

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import import_model as m

args = argparse.ArgumentParser()
args.add_argument("root")
args.add_argument("config_format")
args.add_argument("--write-plans", action="store_true")
args.add_argument("--corrupt")
args = args.parse_args()
root = pathlib.Path(args.root).resolve()
cf_dir = pathlib.Path(args.config_format).resolve()

schema = json.loads((cf_dir / "config.schema.json").read_text(encoding="utf-8"))
Draft202012Validator.check_schema(schema)
validator = Draft202012Validator(schema)
stub = json.loads((cf_dir / "catalog-stub.json").read_text(encoding="utf-8"))
sources = json.loads((root / "samples/sources.json").read_text(encoding="utf-8"))
expected = json.loads((root / "samples/expected.json").read_text(encoding="utf-8"))

# Lift the binding functions out of the Config format's check.py: everything from its strict parser up to its Catalog lint.
text = (cf_dir / "check/check.py").read_text(encoding="utf-8")
start = text.index("# ---------------------------------------------------------------- strict parser")
end = text.index("# ---------------------------------------------------------------- catalog lint")
cf = {
    "json": json, "re": re, "math": math, "difflib": difflib, "unicodedata": unicodedata,
    "CATALOG": {r["id"]: r for r in stub["recipes"]}, "ALIASES": {"nvim": "neovim"}, "DEFAULT_CLONE": "~/Developer/dotfiles",
}
exec(compile(text[start:end], "config-format/check/check.py", "exec"), cf)

if args.corrupt == "no-lookalike-check":
    m.lookalikes = lambda t: []
elif args.corrupt == "tap-starts-on":
    original_start = m.starting_state
    def _on(plan, record, signed_in):
        s = original_start(plan, record, signed_in)
        s["taps"] = {t: True for t in s["taps"]}
        return s
    m.starting_state = _on
elif args.corrupt == "script-without-repo":
    original_run = m.run_contents
    def _run(plan, state, safe_on=True):
        out = original_run(plan, state, safe_on)
        d = plan.get("dotfiles")
        if d and state["script"]:
            out["script"] = d["script"]
        return out
    m.run_contents = _run
    m.can_tick = lambda plan, state, what: True
elif args.corrupt == "blocked-tap-runs":
    original_run2 = m.run_contents
    def _run2(plan, state, safe_on=True):
        out = original_run2(plan, state, safe_on)
        for t in plan["taps"]:
            if state["taps"].get(t["tap"]) and t["tap"] not in out["taps"]:
                out["taps"].append(t["tap"])
        return out
    m.run_contents = _run2
    m.can_tick = lambda plan, state, what: True
elif args.corrupt == "ignore-links-digest":
    m.links_digest = lambda links: "same"
elif args.corrupt == "own-from-anyone":
    m.is_own = lambda c_owner, repo, signed_in: bool(signed_in) and (m.repo_owner(repo) or "").lower() == signed_in.lower()
elif args.corrupt == "keep-approval-on-keep-copy":
    pass  # Handled in the check below.
elif args.corrupt == "no-duplicate-refusal":
    cf["fold"] = lambda p: p + str(id(p))
elif args.corrupt:
    sys.exit(f"unknown corruption {args.corrupt}")

failures = 0


def report(ok, line):
    global failures
    failures += 0 if ok else 1
    print(f"{'PASS' if ok else 'FAIL'} {line}")


print("schema: valid against the 2020-12 metaschema")
print(f"catalog stub: {len(cf['CATALOG'])} entries")

# ---------------------------------------------------------------- every sample, for real
print("\n== samples: strict parser, schema, then resolve and lint")
plans, configs = {}, {}
signed_in = sources["signedInAs"]
for sample in sorted(sources["samples"]):
    raw = (root / "samples" / sample).read_bytes()
    plan, c = m.build_plan(sample, raw, sources["samples"][sample], sources["machine"], signed_in, cf, validator)
    plans[sample], configs[sample] = plan, c
    want = expected[sample]
    if "refused" in plan:
        got = {"verdict": "refused", "stage": plan["refused"]["stage"], "at": plan["refused"]["at"], "code": plan["refused"]["code"]}
        report(got == want, f"{sample}: refused by the {got['stage']} at {got['at']} [{got['code']}]: {plan['refused']['detail'][:96]}")
        if got != want:
            print("     wanted", want, "got", got)
    else:
        lint = sorted({code for code, _, _ in cf["lint_read"](c, cf["CATALOG"])})
        got = {
            "verdict": "accepted",
            "taps": [t["tap"] for t in plan["taps"]],
            "blockedTaps": [t["tap"] for t in plan["taps"] if t["blocked"]],
            "dotfiles": plan["dotfiles"]["repo"] if plan["dotfiles"] else None,
            "script": plan["dotfiles"]["script"] if plan["dotfiles"] else None,
            "lint": lint,
            "refusedLinks": sorted(l["target"] for l in plan["dotfiles"]["links"] if l["refused"]) if plan["dotfiles"] else [],
            "signals": sorted(s["code"] for s in plan["signals"]),
            "wontApply": sorted(w["id"] for w in plan["wontApply"]),
        }
        report(got == want, f"{sample}: accepted. taps {got['taps']}, dotfiles {got['dotfiles']}, script {got['script']}, signals {len(got['signals'])}, won't apply {len(got['wontApply'])}")
        if got != want:
            for k in got:
                if got[k] != want.get(k):
                    print(f"     {k}: wanted {want.get(k)} got {got[k]}")

accepted = [s for s in plans if "refused" not in plans[s]]
print("\n== refusals inside the accepted hostile Config, entry by entry")
h = plans["hostile.json"]
for t in h["taps"]:
    print(f"  tap {t['tap']}: {'NO APPROVAL OFFERED' if t['blocked'] else 'off, can be approved'} ({'; '.join(p['via'] for p in t['packages'])})")
for l in h["dotfiles"]["links"]:
    print(f"  link {l['targetEscaped']}: {'REFUSED: ' + l['refused'] if l['refused'] else 'off with the repo'}{' / note: ' + l['note'] if l['note'] else ''}{' / lookalike' if l['lookalike'] else ''}")
for w in h["wontApply"]:
    print(f"  entry {w['id']}: {w['kind']}")
for s in h["signals"]:
    print(f"  signal {s['code']}: {s['text']}")

# ---------------------------------------------------------------- the click-through property
print("\n== clicking straight through: what a Run does if no box is touched")
for sample in accepted:
    plan = plans[sample]
    state = m.starting_state(plan, None, signed_in)
    run = m.run_contents(plan, state)
    own = bool(plan["dotfiles"] and m.is_own(plan["source"]["owner"], plan["dotfiles"]["repo"], signed_in))
    third_party = bool(run["taps"] or run["script"]) or (bool(run["clone"]) and not own)
    report(not third_party, f"{sample}: no third-party code runs ({'your own Dotfiles Repo stays on' if own else 'nothing outside the Catalog and official Homebrew'})")
report(not m.run_contents(h, m.starting_state(h, None, signed_in))["clone"], "hostile.json: its Dotfiles Repo isn't even cloned")
report(bool(h["signals"]), "hostile.json: its signals start everything off, so a Run has nothing to do until someone selects something")
report(h["bigLists"] == ["formulae"], "hostile.json: its 478 official formulae start off even without the signals")
buried = next(t for t in h["taps"] if t["tap"] == "mesa-tools/tap")
report("line " in buried["packages"][0]["via"], f"hostile.json: the buried line surfaces at the top: {buried['packages'][0]['name']}, {buried['packages'][0]['via']}")

print("\n== random boxes on every sample: an approval is never implied")
rng = random.Random(15)
runs = 0
for _ in range(4000):
    sample = rng.choice(accepted)
    plan = plans[sample]
    state = m.starting_state(plan, None, signed_in)
    what = [t["tap"] for t in plan["taps"]] + (["repo", "script"] if plan["dotfiles"] else [])
    for _ in range(rng.randint(0, 6)):
        if not what:
            break
        w = rng.choice(what)
        if not m.can_tick(plan, state, w):
            continue
        if w in ("repo", "script"):
            state[w] = not state[w]
            if w == "repo" and not state["repo"]:
                state["script"] = False  # Turning the repo off turns its script off too.
        else:
            state["taps"][w] = not state["taps"][w]
    run = m.run_contents(plan, state)
    runs += 1
    bad = []
    if run["script"] and not run["clone"]:
        bad.append("script without its repo")
    for t in plan["taps"]:
        if t["tap"] in run["taps"] and not state["taps"][t["tap"]]:
            bad.append("tap without a tick")
        if t["blocked"] and t["tap"] in run["taps"]:
            bad.append("blocked tap ran")
    d = plan.get("dotfiles")
    if d and run["linked"] and any(l["target"] in run["linked"] and l["refused"] for l in d["links"]):
        bad.append("refused link applied")
    if bad:
        report(False, f"{sample}: {bad}")
        break
else:
    report(True, f"{runs} random box states: no script without its repo, no tap without its own tick, no blocked tap, no refused link")

# ---------------------------------------------------------------- where approvals start, and when they end
print("\n== approvals: where they start and what voids them")
s = plans["stranger-dotfiles.json"]
t1 = plans["one-tap.json"]
o = plans["own-dotfiles.json"]
state = m.starting_state(s, None, signed_in)
report(not state["repo"] and not state["script"], "stranger's Dotfiles Repo and script start off")
report(not m.can_tick(s, state, "script"), "the script can't be ticked while its repo is off")
state["repo"] = True
report(m.can_tick(s, state, "script") and not m.run_contents(s, state)["script"], "with the repo on, the script is available and still off")
report(not m.starting_state(t1, None, signed_in)["taps"]["mobile-dev-inc/tap"], "a tap starts off")
ost = m.starting_state(o, None, signed_in)
report(ost["repo"] and not ost["script"], "your own Dotfiles Repo starts on, and its script still starts off")
report(not m.starting_state(o, None, None)["repo"], "before sign-in, your own repo can't be known as yours, so it starts off")
mine_foreign = copy.deepcopy(o)
mine_foreign["source"] = {**o["source"], "owner": "example-stranger"}
report(not m.starting_state(mine_foreign, None, signed_in)["repo"], "a stranger's Config naming your repo starts off: they wrote that list of links")
report(m.is_own("example-you", "https://github.com/Example-You/dotfiles", signed_in), "your repo written as a GitHub URL, in other case, is still yours")
report(not m.is_own("example-you", "https://github.com.evil.example/example-you/dotfiles", signed_in), "the same path on a lookalike host isn't")
report(not m.is_own("example-you", "https://git.example.com/example-you/dotfiles", signed_in), "the same path on another host isn't")

state = m.starting_state(s, None, signed_in)
state["repo"] = True
state["script"] = True
rec = m.record_after_run(s, state)
again = m.starting_state(s, rec, signed_in)
report(again["repo"] and again["script"], "reviewing the same commit and links again keeps both on")
links = configs["stranger-dotfiles.json"]["dotfiles"]["links"]
script = "script/setup"
repo = s["dotfiles"]["repo"]
commit = s["dotfiles"]["commit"]
report(m.dotfiles_status(rec, repo, links, script, commit) == "current", "same commit: current")
report(m.dotfiles_status(rec, repo, links, script, "e4f5a6b") == "behind", "newer commit: behind, and it stays at the approved one")
report(m.dotfiles_status(rec, repo, links, script, "e4f5a6b", approved_is_ancestor=False) == "history-rewritten", "force-pushed: history rewritten, so no comparison is offered as if it were complete")
report(m.dotfiles_status(rec, repo, {**links, "~/.zshenv": "z/env"}, script, commit) == "void-links-changed", "a link added at the same commit voids the approval")
report(m.dotfiles_status(rec, repo, links, script, commit, repo_moved_to="elsewhere/dotfiles") == "void-repo-moved", "a transferred repo voids it")
report(m.dotfiles_status(rec, "elsewhere/dotfiles", links, script, commit) == "void-repo-moved", "a different repo in the Config voids it")
report(m.dotfiles_status({**rec, "script": None}, repo, links, script, commit) == "script-not-approved", "a script approved for no commit stays off")
moved = copy.deepcopy(s)
moved["dotfiles"]["commit"] = "e4f5a6b"
report(not m.starting_state(moved, rec, signed_in)["repo"], "a stranger's newer commit starts off again")
own_rec = m.record_after_run(o, m.starting_state(o, None, signed_in))
own_moved = copy.deepcopy(o)
own_moved["dotfiles"]["commit"] = "88bb123"
report(m.starting_state(own_moved, own_rec, signed_in)["repo"], "your own newer commit starts on, and nothing happens until you press the button")

tap_rec = m.record_after_run(t1, {"taps": {"mobile-dev-inc/tap": True}, "repo": False, "script": False})
report(tap_rec["packages"] == ["mobile-dev-inc/tap/maestro"], "a tap approval records the package, not the whole tap")
report(m.starting_state(t1, tap_rec, signed_in)["taps"]["mobile-dev-inc/tap"], "the same package keeps its approval")
grown = copy.deepcopy(t1)
grown["taps"][0]["packages"].append({"name": "mobile-dev-inc/tap/other", "short": "other", "kind": "formula", "via": "", "sameAsBase": False, "sameAsCatalog": None})
report(not m.starting_state(grown, tap_rec, signed_in)["taps"]["mobile-dev-inc/tap"], "a new package from an approved tap starts off")
if args.corrupt != "keep-approval-on-keep-copy":
    keep_copy_record = {}  # Keep a copy never calls record_after_run.
else:
    keep_copy_record = tap_rec
report(keep_copy_record == {}, "keep a copy records no approvals")

saved = copy.deepcopy(t1)
saved["source"] = {**saved["source"], "owner": signed_in}
report(not m.starting_state(saved, None, signed_in)["taps"]["mobile-dev-inc/tap"], "saving a copy to your own Config Repo doesn't approve its tap on another Mac")
saved_s = copy.deepcopy(s)
saved_s["source"] = {**saved_s["source"], "owner": signed_in}
report(not m.starting_state(saved_s, None, signed_in)["repo"], "saving it doesn't make a stranger's Dotfiles Repo yours")

print("\n== an update to an imported Config")
old = configs["one-tap.json"]
new = copy.deepcopy(old)
new["packages"]["formulae"].append("mobile-dev-inc/tap/flashlight")
new["settings"]["dock.autohide"] = False
new["items"].remove("kotlin")
new["dotfiles"] = {"repo": "example-dev/dotfiles", "links": {"~/.zshrc": "zshrc"}}
delta = m.update_delta(old, new)
report("formulae:mobile-dev-inc/tap/flashlight" in delta["added"] and "dotfiles.repo" in delta["added"], "added entries are found, including a new package and a Dotfiles Repo")
report(delta["changed"] == ["setting:dock.autohide"], "changed values are found")
report(delta["gone"] == ["item:kotlin"], "a line removed upstream is found, and it's Drift, never an uninstall")

print("\n== text from the file")
report(m.escaped("Аррle") == "\\u0410\\u0440\\u0440le", "lookalike letters escape to their code points")
report(m.lookalikes("仕事用 Mac") == [] and m.lookalikes("Рабочий") == [], "a single-script or CJK-with-Latin name isn't flagged as a lookalike")
report(m.lookalikes("Аррle") != [], "Cyrillic mixed with Latin is")
report(m.fakes_a_name("homebrevv") == "homebrew", "homebrevv reads as homebrew")
report(m.fakes_a_name("s0ulcaster-official") == "soulcaster", "s0ulcaster-official reads as soulcaster")
report(m.fakes_a_name("mobile-dev-inc") is None, "mobile-dev-inc reads as itself")
report(m.CLAIMS.search("Plain dev laptop") is None and m.CLAIMS.search("Mobile dev laptop") is None, "ordinary names make no claim")
report(all(not plans[x]["signals"] for x in ("clean.json", "one-tap.json", "stranger-dotfiles.json", "own-dotfiles.json")), "none of the four ordinary samples trips a signal")

if args.write_plans:
    out = {"signedInAs": signed_in, "plans": {k.replace(".json", ""): v for k, v in plans.items()}}
    for v in out["plans"].values():
        v.pop("errors", None)
    (root / "app/plans.json").write_text(json.dumps(out, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    print("\nwrote app/plans.json")

print("\nresult: " + ("all checks passed" if not failures else f"{failures} FAILED"))
sys.exit(1 if failures else 0)

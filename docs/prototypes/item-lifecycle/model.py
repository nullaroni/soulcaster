#!/usr/bin/env python3
"""PROTOTYPE, not the engine. Runs the Item lifecycle over the real recipes in recipes-examples.json.

It tries failures, stalls, steps that lie, surprise sudo needs and crashes, then checks the invariants below.
Each mutation turns off one rule, and that rule's invariant must fire, so the checks can't pass by doing nothing.

    python3 model.py [recipes-examples.json]
"""
import json, pathlib, random, sys

OK = {"satisfied", "unknown"}
TERMINAL = OK | {"needs-you", "failed", "blocked", "skipped"}
OUTCOMES = ["ok", "fail", "lie", "stall", "surprise-admin", "crash-between-steps"]
BASE_ORDER = ["command-line-tools", "homebrew", "gh"]
RETRY_CLASS = {"needs-admin", "stalled", "install-it-yourself"}


def third_party(name):
    return name.count("/") == 2


class Recipe:
    def __init__(self, raw):
        self.id = raw["id"]
        self.kind = raw["kind"]
        self.raw = raw
        self.requires = list(raw.get("requires", []))
        self.steps = raw.get("steps", [])
        self.base = raw.get("base", False)
        self.broken = raw.get("evidence", {}).get("group") == "broken"
        self.third_party = any(third_party(s.get("name", "")) for s in self.steps if s["type"].startswith("brew"))
        self.handoff_check = any(p.get("name") == "gh-handed-off" for p in raw.get("checks", []))

    def is_admin_step(self, step):
        return step["type"] == "pkg-install" or step.get("admin", False)


class World:
    """What survives between Runs: the Machine and the Machine Record. The password is never in here."""

    def __init__(self, base_present=False, account_admin=True):
        self.account_admin = account_admin
        self.installed = set(BASE_ORDER[:2]) if base_present else set()
        self.handed_off = False
        self.bootstrap = False
        self.active_gh = None
        self.signed_in = False
        self.person_done = set()  # (item, step id) a person finished or approved
        self.open_queue = set()  # (item, step id) open Needs-You entries
        self.learned_admin = set()  # In the Machine Record: Items that turned out to need admin
        self.truly_admin = set()  # A fact about the Machine: these fail without sudo on every Run
        self.ever_opened = set()  # Every queue entry the engine has opened
        self.last = {}  # item -> last terminal state


class Violation(Exception):
    pass


class Crash(Exception):
    """The app died, or the power went, part way through an Item."""


class Sim:
    def __init__(self, recipes, selected, mutations=frozenset()):
        self.recipes = recipes
        self.selected = selected
        self.m = mutations
        self.dependents = {i: set() for i in recipes}
        for r in recipes.values():
            for q in r.requires:
                self.dependents[q].add(r.id)

    def closure(self, ids):
        out, todo = set(), list(ids)
        while todo:
            i = todo.pop()
            if i not in out:
                out.add(i)
                todo += self.recipes[i].requires
        return out

    def full_plan(self):
        return self.closure(set(self.selected) | {i for i, r in self.recipes.items() if r.base})

    def scoped_plan(self, world, scope):
        if "no-closure" in self.m:
            return set(scope)
        plan = self.full_plan()
        out, todo = set(), list(scope)
        while todo:  # downward: anything waiting on this
            i = todo.pop()
            if i in out or i not in plan:
                continue
            out.add(i)
            todo += self.dependents[i]
        for i in list(out):  # upward: whatever it needs and doesn't have yet
            out |= {q for q in self.closure([i]) if world.last.get(q) not in OK}
        return out & plan

    def downward(self, scope):
        """What should move once this scope finishes: the scope and everything waiting on it."""
        out, todo = set(), list(scope)
        while todo:
            i = todo.pop()
            if i not in out:
                out.add(i)
                todo += self.dependents[i]
        return out & self.full_plan()

    def passes_checks(self, world, r):
        if r.id not in world.installed:
            return False
        if r.handoff_check and not world.handed_off:
            return False
        return True

    def gate_open(self, world, r, step):
        if step["type"] not in ("needs-you", "accept-license"):
            return False
        return (r.id, step["id"]) not in world.person_done

    def needs_password_now(self, world, r):
        """Would this Item reach an admin step in this Run, with no open gate before it?"""
        if self.passes_checks(world, r) or r.broken:
            return False
        if r.id in world.learned_admin and not r.third_party:
            return True
        for s in r.steps:
            if self.gate_open(world, r, s):
                return False
            if r.is_admin_step(s):
                return True
        return False

    def run(self, world, outcomes, scope=None, github_ok=True, crash_at=None):
        plan = self.full_plan() if scope is None else self.scoped_plan(world, scope)
        state = {i: "pending" for i in plan}
        self.last_scope_closure = self.downward(scope) if scope is not None else set(plan)
        trace = {"asks": 0, "admin_after_discard": [], "third_party_while_held": []}
        needs = {i: self.needs_password_now(world, self.recipes[i]) for i in plan}

        # Refuse before writing anything if a standard account has no base to work with.
        if not world.account_admin and not {"command-line-tools", "homebrew"} <= world.installed:
            return {i: "skipped" for i in plan}, dict(trace, refused=True)

        # First minute. A full Run always asks. A scoped Run asks only if it has admin work.
        ask = world.account_admin and (scope is None or any(needs.values()))
        held = False
        if ask:
            trace["asks"] += 1
            held = True
        if "gh" not in world.installed or not world.handed_off:
            world.bootstrap = world.bootstrap or "gh" not in world.installed
            world.active_gh = "brew" if world.handed_off else "bootstrap"
        if github_ok:
            world.signed_in = True

        # Rule: check every Item before assigning stages, so an Item that's already fine can't look
        # unfinished to something in an earlier stage. Skipping this was the first bug the model found.
        if "no-check-before-stages" not in self.m:
            for i in sorted(plan):
                r = self.recipes[i]
                open_here = {k for k in world.open_queue if k[0] == i and k not in world.person_done}
                if self.passes_checks(world, r) and not open_here:
                    state[i] = "satisfied"
                    world.last[i] = "satisfied"
                    world.open_queue -= {k for k in world.open_queue if k[0] == i}

        # Stage order: Admin Stage first, then discard, then everything else.
        admin_items = [i for i in plan if needs[i] and state[i] == "pending"]
        user_items = [i for i in plan if not needs[i] and state[i] == "pending"]
        order = self.topo(admin_items) + ["<discard>"] + self.topo(user_items)
        if "discard-at-run-end" in self.m:
            order.remove("<discard>")

        crashed = False
        count = 0
        for i in order:
            if i == "<discard>":
                held = False
                continue
            if crash_at is not None and count == crash_at:
                crashed = True
                break
            count += 1
            try:
                state[i] = self.run_item(world, i, state, outcomes.get(i, "ok"), held, trace)
            except Crash:
                crashed = True
                break
            world.last[i] = state[i]
            if "stop-on-failure" in self.m and state[i] == "failed":
                break

        if crashed:
            # Next launch reconciles: anything unfinished is recorded as failed, kind "interrupted".
            for i in plan:
                if state[i] == "pending":
                    state[i] = "failed"
                    world.last[i] = "failed"
            held = False
        elif "stop-on-failure" not in self.m:
            held = False
        if "delete-bootstrap-at-end" in self.m:
            world.bootstrap = False
            if not world.handed_off:
                world.active_gh = None
        trace["held_at_end"] = held
        trace["planned"] = plan
        trace["crashed"] = crashed
        return state, trace

    def topo(self, ids):
        ids = set(ids)
        out, seen = [], set()

        def visit(i):
            if i in seen:
                return
            seen.add(i)
            for q in self.recipes[i].requires:
                if q in ids:
                    visit(q)
            out.append(i)

        base_first = sorted(ids, key=lambda i: (BASE_ORDER.index(i) if i in BASE_ORDER else 99, i))
        for i in base_first:
            visit(i)
        return out

    def run_item(self, world, i, state, outcome, held, trace):
        r = self.recipes[i]
        if r.broken:
            return "skipped"
        for q in r.requires:
            if q in state and state[q] not in OK:
                return "blocked"
            if q not in state and world.last.get(q) not in OK and q not in world.installed:
                return "blocked"

        # Rule: an open entry holds the Item until a person finishes it, even if its checks pass.
        open_here = {k for k in world.open_queue if k[0] == i and k not in world.person_done}
        if self.passes_checks(world, r):
            if open_here and "ignore-open-queue" not in self.m:
                return "needs-you"
            world.open_queue -= {k for k in world.open_queue if k[0] == i}
            return "satisfied"

        if r.kind == "machine-values":
            if (i, "values") not in world.person_done:
                world.open_queue.add((i, "values"))
                return "needs-you"
            world.installed.add(i)
            world.open_queue.discard((i, "values"))
            return "satisfied"

        steps = r.steps or [{"type": "apply-setting"}]
        for n, s in enumerate(steps):
            if outcome == "crash-between-steps" and n == 1:
                raise Crash()
            if self.gate_open(world, r, s):
                world.open_queue.add((i, s["id"]))
                return "needs-you"
            if s["type"] in ("needs-you", "accept-license"):
                world.open_queue.discard((i, s["id"]))
                if not r.is_admin_step(s):
                    continue
            admin = r.is_admin_step(s) or (i in world.learned_admin and s["type"].startswith("brew"))
            if admin and not held:
                trace["admin_after_discard"].append(i)
                world.open_queue.add((i, "needs-admin"))
                return "needs-you"
            if r.third_party and held:
                trace["third_party_while_held"].append(i)
            if s["type"] == "builtin" and s["name"] == "gh-hand-off":
                world.handed_off = True
                world.active_gh = "brew"
                world.bootstrap = False
                continue
            if outcome == "fail":
                return "failed"
            if outcome == "stall":
                # A watchdog kills the step. Only admin steps can be stuck on a person.
                if admin:
                    world.open_queue.add((i, "stalled"))
                    return "needs-you"
                return "failed"
            if outcome == "surprise-admin" and s["type"].startswith("brew"):
                world.truly_admin.add(i)
            if i in world.truly_admin and not admin and s["type"].startswith("brew"):
                if r.third_party:
                    world.open_queue.add((i, "install-it-yourself"))
                    return "needs-you"
                if "no-learned-admin" not in self.m:
                    world.learned_admin.add(i)
                world.open_queue.add((i, "needs-admin"))
                return "needs-you"
            if outcome == "lie":
                if "trust-exit-code" in self.m:
                    return "satisfied"  # the bug: no verify after acting
                return "failed"  # verify re-read the checks and they still fail
            if s["type"].startswith("brew") or s["type"] in ("pkg-install", "download", "apply-setting"):
                world.installed.add(i)
            if s.get("name") == "command-line-tools":
                world.installed.add(i)
            if s.get("name") == "xcode-first-launch":
                world.installed.add(i)
        world.installed.add(i)
        world.open_queue -= {k for k in world.open_queue if k[0] == i}
        world.person_done -= {k for k in world.person_done if k[0] == i and k[1] in ("needs-admin", "stalled")}
        return "satisfied"


def check_run(sim, world, state, trace, outcomes):
    """Invariants for one Run."""
    world.ever_opened |= world.open_queue
    if trace.get("refused"):
        if world.installed - set(BASE_ORDER):
            raise Violation("I0 a refused Run changed the Machine")
        return
    planned = trace["planned"]
    for i in sorted(planned):
        if state[i] not in TERMINAL and not trace["crashed"]:
            raise Violation(f"I1 {i} never reached a terminal state: {state[i]}")
    if trace["asks"] > 1:
        raise Violation("I2 the password was asked for more than once in a Run")
    if trace["third_party_while_held"]:
        raise Violation(f"I3 third-party tap code ran while the password was held: {sorted(trace['third_party_while_held'])}")
    if trace["held_at_end"]:
        raise Violation("I4 the password was still held when the Run ended")
    if world.account_admin:
        if trace["admin_after_discard"]:
            late = sorted(i for i in trace["admin_after_discard"] if i not in world.learned_admin)
            if late:
                raise Violation(f"I5 an admin step was reached after the password was discarded: {late}")
    if trace["crashed"]:
        return
    for i in sorted(planned):
        r = sim.recipes[i]
        if state[i] == "blocked":
            # Mirrors run_item: a requirement in this Run counts by its state, even if its package landed
            # before a person-gate stopped it, as android-sdk's cask does before its license.
            bad = [q for q in r.requires
                   if (state[q] not in OK if q in state else world.last.get(q) not in OK and q not in world.installed)]
            if not bad or all(state.get(q) == "pending" for q in bad):
                raise Violation(f"I6 {i} is blocked but no requirement ended badly")
        if state[i] == "satisfied" and i not in world.installed:
            raise Violation(f"I7 {i} reported satisfied while its checks would fail")
    if world.active_gh is None:
        raise Violation("I8 the Run ended with no working gh")
    if "gh" in world.installed and world.handed_off and world.bootstrap:
        raise Violation("I8 the bootstrap gh was left behind after hand-off")
    # One failure must only touch what depends on it.
    failed = {i for i in planned if state[i] == "failed"}
    downstream = set(failed)
    changed = True
    while changed:
        changed = False
        for i in planned:
            if i not in downstream and any(q in downstream for q in sim.recipes[i].requires):
                downstream.add(i)
                changed = True
    for i in sorted(planned - downstream):
        if state[i] == "pending":
            raise Violation(f"I9 {i} was never tried because something else failed")
    # Only entries a person must finish. A retry may rightly close "stalled" or "needs-admin" by succeeding.
    for k in sorted(world.ever_opened):
        if k[1] in RETRY_CLASS:
            continue
        if state.get(k[0]) == "satisfied" and k not in world.person_done:
            raise Violation(f"I10 {k[0]} became satisfied before a person finished {k[1]}")


def check_retry_reached_everything(sim, world, last, plan):
    """Nothing stays blocked by a requirement that has since finished."""
    for i in sorted(plan):
        if last[i] != "blocked" or i not in world.last:
            continue
        if all(last.get(q, world.last.get(q)) in OK for q in sim.recipes[i].requires):
            # A retry that named only an unrelated Item may leave this one for later.
            if world.last.get(i) == "blocked" and i in sim.last_scope_closure:
                raise Violation(f"I12 {i} stayed blocked after its requirements finished")


def person_clears_queue(world):
    for k in list(world.open_queue):
        world.person_done.add(k)
        if k[1] == "install-it-yourself":
            world.installed.add(k[0])
    world.open_queue = {k for k in world.open_queue if k[1] not in RETRY_CLASS}


def scenario(sim, rng, full_rerun):
    base_present = rng.random() < 0.2
    world = World(base_present=base_present, account_admin=rng.random() > 0.1)
    plan = sim.full_plan()
    outcomes = {i: rng.choice(OUTCOMES) if rng.random() < 0.35 else "ok" for i in sorted(plan)}
    crash = rng.randrange(len(plan)) if rng.random() < 0.15 else None
    state, trace = sim.run(world, outcomes, github_ok=rng.random() > 0.1, crash_at=crash)
    check_run(sim, world, state, trace, outcomes)
    if trace.get("refused"):
        return
    last = {i: world.last.get(i, state[i]) for i in plan}
    for _round in range(40):  # Afterwards nothing fails again, and the person does what's asked.
        unfinished = {i for i in plan if last[i] not in OK and not sim.recipes[i].broken}
        if not unfinished:
            break
        if _round >= 4 or rng.random() < 0.7:  # Sometimes the person hasn't got to the queue yet.
            person_clears_queue(world)
        # A retry names everything unfinished, or just one Item when someone presses Retry on it.
        scope = None if full_rerun else (unfinished if rng.random() < 0.5 else {rng.choice(sorted(unfinished))})
        state, trace = sim.run(world, {}, scope=scope)
        check_run(sim, world, state, trace, {})
        last.update({i: state[i] for i in state})
        if world.account_admin:
            check_retry_reached_everything(sim, world, last, plan)
    else:
        if world.account_admin:
            raise Violation(f"I11 reruns never finished: {sorted(i for i in plan if last[i] not in OK)}")
    if world.account_admin:
        stuck = {i for i in plan if last[i] not in OK and not sim.recipes[i].broken}
        if stuck:
            raise Violation(f"I11 reruns left these unfinished: {sorted(stuck)}")
        if world.bootstrap or world.active_gh != "brew":
            raise Violation(f"I8 after convergence: bootstrap={world.bootstrap}, active gh={world.active_gh}")


def sweep(recipes, mutations=frozenset(), runs=6000, seed=20261106, extra=None):
    rs = dict(recipes)
    if extra:
        rs = extra(rs)
    selected = set(rs)
    sim = Sim(rs, selected, mutations)
    rng = random.Random(seed)
    for n in range(runs):
        try:
            scenario(sim, rng, full_rerun=n % 2 == 0)
        except Violation as v:
            return str(v), n
    return None, runs


def main():
    path = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else pathlib.Path(__file__).with_name("recipes-examples.json")
    raw = json.loads(path.read_text())["catalog"]["recipes"]
    recipes = {r["id"]: Recipe(r) for r in raw}

    result, n = sweep(recipes)
    status = "PASS" if result is None else "FAIL"
    print(f"{status} recipes as written: {n} random scenarios" + (f" -> {result}" if result else ""))
    failed = result is not None

    def tailscale_needs_openjdk(rs):
        raw2 = json.loads(json.dumps(rs["tailscale"].raw))
        raw2["requires"].append("openjdk")
        rs["tailscale"] = Recipe(raw2)
        return rs

    def gh_check_without_hand_off(rs):
        raw2 = json.loads(json.dumps(rs["gh"].raw))
        raw2["checks"] = [p for p in raw2["checks"] if p.get("name") != "gh-handed-off"]
        rs["gh"] = Recipe(raw2)
        return rs

    print("\nmutations: each must trip an invariant")
    cases = [
        ("discard-at-run-end", {"discard-at-run-end"}, None, "I3"),
        ("no-learned-admin", {"no-learned-admin"}, None, "I11"),
        ("no-closure", {"no-closure"}, None, "I12"),
        ("trust-exit-code", {"trust-exit-code"}, None, "I7"),
        ("stop-on-failure", {"stop-on-failure"}, None, "I1"),
        ("no-check-before-stages", {"no-check-before-stages"}, None, "I6"),
        ("delete-bootstrap-at-end", {"delete-bootstrap-at-end"}, None, "I8"),
        ("ignore-open-queue", {"ignore-open-queue"}, None, "I10"),
        ("gh-check-without-hand-off", set(), gh_check_without_hand_off, "I8"),
        ("admin-item-needing-a-user-item", set(), tailscale_needs_openjdk, "I6"),
    ]
    for name, muts, extra, expected in cases:
        result, n = sweep(recipes, frozenset(muts), extra=extra)
        tripped = bool(result) and result.startswith(expected)
        failed = failed or not tripped
        print(f"{'PASS' if tripped else 'FAIL'} {name}: expected {expected}, got {result!r} after {n} scenarios")

    print("\nresult: " + ("problems found" if failed else "all invariants held and all mutations were caught"))
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Check catalog.json against live Homebrew JSON and the tap files it names.

Downloads formula.json and cask.json. Does not install anything and does not run brew.
"""

import json
import re
import sys
import urllib.request
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent
CATALOG = ROOT / "catalog.json"
FORMULA_URL = "https://formulae.brew.sh/api/formula.json"
CASK_URL = "https://formulae.brew.sh/api/cask.json"
ID_RE = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*$")
BASE_IDS = {"homebrew", "command-line-tools", "gh"}
UA = "soulcaster-catalog-check"


def fetch(url):
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    with urllib.request.urlopen(req, timeout=120) as response:
        return response.read(), response.status, response.geturl()


def arm_formula(formula):
    files = list((((formula.get("bottle") or {}).get("stable") or {}).get("files") or {}).keys())
    if "all" in files:
        return True
    return any(name.startswith("arm64_") and "linux" not in name for name in files)


def arm_cask(cask):
    if cask.get("caveats_rosetta"):
        return False
    plats = cask.get("supported_platforms") or []
    return any(str(p).startswith("arm64_") for p in plats)


def artifact_admin(cask):
    arts = cask.get("artifacts") or []
    blob = json.dumps(arts)
    return any(isinstance(a, dict) and "pkg" in a for a in arts) or (
        '"sudo": true' in blob or '"sudo":true' in blob
    )


def live_conflicts(record, kind):
    if kind == "formula":
        return [("formula", name) for name in (record.get("conflicts_with") or [])]
    raw = record.get("conflicts_with") or {}
    if not isinstance(raw, dict):
        return []
    out = []
    for part in ("formula", "cask"):
        for name in raw.get(part) or []:
            out.append((part, name))
    return out


def main():
    errors = []
    catalog = json.loads(CATALOG.read_text())
    fetched_at = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    print(f"catalog {CATALOG}")
    print(f"live fetch started {fetched_at}")
    formula_bytes, formula_status, _ = fetch(FORMULA_URL)
    cask_bytes, cask_status, _ = fetch(CASK_URL)
    if formula_status != 200 or cask_status != 200:
        print(f"FAIL download status formula={formula_status} cask={cask_status}")
        return 1
    formulas = json.loads(formula_bytes)
    casks = json.loads(cask_bytes)
    by_f = {x["name"]: x for x in formulas}
    aliases = {}
    for formula in formulas:
        for alias in (formula.get("aliases") or []) + (formula.get("oldnames") or []):
            aliases[alias] = formula["name"]
    by_c = {x["token"]: x for x in casks}
    print(f"formulae {len(by_f)} casks {len(by_c)} bytes {len(formula_bytes)} {len(cask_bytes)}")

    items = catalog["items"]
    ids = [item["id"] for item in items]
    if len(ids) != len(set(ids)):
        errors.append("duplicate item ids")
    sections = {section["id"] for section in catalog["sections"]}
    seen_tokens = set()

    for item in items:
        item_id = item["id"]
        if item_id in BASE_IDS:
            errors.append(f"{item_id} is a base Item and must not be selectable")
        if not ID_RE.match(item_id):
            errors.append(f"{item_id} is not a lowercase kebab-case product id")
        if item["section"] not in sections:
            errors.append(f"{item_id} section {item['section']} is not declared")
        if not item.get("description") or "\n" in item["description"]:
            errors.append(f"{item_id} description is empty or multiline")
        if not item.get("homepage", "").startswith("https://"):
            errors.append(f"{item_id} homepage is not https")
        install = item["install"]
        kind = install["kind"]
        token = install.get("token")
        if token == "gh" and kind == "formula":
            errors.append("gh formula is selectable")
        if kind in {"formula", "cask"}:
            key = (kind, token)
            if key in seen_tokens:
                errors.append(f"duplicate install token {kind} {token}")
            seen_tokens.add(key)

        if kind == "app-store":
            if token:
                errors.append(f"{item_id} app-store item has a Homebrew token")
            continue

        if item.get("tap"):
            url = install.get("sourceUrl")
            needle = install.get("sourceMustContain")
            if not url or not needle:
                errors.append(f"{item_id} tap item is missing sourceUrl or sourceMustContain")
                continue
            try:
                body, status, final = fetch(url)
            except Exception as exc:
                errors.append(f"{item_id} tap fetch failed: {exc}")
                continue
            text = body.decode("utf-8", "replace")
            if status != 200 or needle not in text:
                errors.append(f"{item_id} tap check failed status={status} final={final}")
            if item["arm64"].get("status") != "yes":
                errors.append(f"{item_id} arm64 status is not yes")
            continue

        if kind == "formula":
            if token in by_f:
                formula = by_f[token]
                if install.get("resolvesTo"):
                    errors.append(f"{item_id} sets resolvesTo but {token} is already a formula name")
            elif token in aliases:
                formula = by_f[aliases[token]]
                if install.get("resolvesTo") != aliases[token]:
                    errors.append(
                        f"{item_id} resolvesTo {install.get('resolvesTo')} but live alias is {aliases[token]}"
                    )
            else:
                errors.append(f"{item_id} formula token {token} is not in live Homebrew data")
                continue
            if formula.get("deprecated") or formula.get("disabled"):
                errors.append(f"{item_id} formula {formula['name']} is deprecated or disabled")
            if not arm_formula(formula):
                errors.append(f"{item_id} formula {formula['name']} has no Apple Silicon bottle")
            if formula.get("homepage") != item["homepage"]:
                errors.append(f"{item_id} homepage does not match live formula data")
            expected = set(live_conflicts(formula, "formula"))
            recorded = {
                (entry["kind"], entry["token"])
                for entry in item["conflictsWith"]
                if entry.get("source") == "conflicts_with"
            }
            if recorded != expected:
                errors.append(f"{item_id} formula conflicts differ: live {sorted(expected)} catalog {sorted(recorded)}")
        elif kind == "cask":
            cask = by_c.get(token)
            if cask is None:
                errors.append(f"{item_id} cask token {token} is not in live Homebrew data")
                continue
            if cask.get("deprecated") or cask.get("disabled"):
                errors.append(f"{item_id} cask {token} is deprecated or disabled")
            if not arm_cask(cask):
                errors.append(f"{item_id} cask {token} is Intel-only or needs Rosetta")
            if cask.get("homepage") != item["homepage"]:
                errors.append(f"{item_id} homepage does not match live cask data")
            expected = set(live_conflicts(cask, "cask"))
            recorded = {
                (entry["kind"], entry["token"])
                for entry in item["conflictsWith"]
                if entry.get("source") == "conflicts_with"
            }
            if recorded != expected:
                errors.append(f"{item_id} cask conflicts differ: live {sorted(expected)} catalog {sorted(recorded)}")
            admin = artifact_admin(cask)
            recorded_admin = any(
                entry.get("source") == "cask-artifact" and entry.get("kind") == "admin-password"
                for entry in item["needsFromPerson"]
            )
            if admin != recorded_admin:
                errors.append(f"{item_id} admin-password from cask artifact is {recorded_admin}, live pkg/sudo is {admin}")
        else:
            errors.append(f"{item_id} unknown install kind {kind}")

        if item["arm64"].get("status") != "yes":
            errors.append(f"{item_id} arm64 status is not yes")

    counts = Counter(item["section"] for item in items)
    print(f"items {len(items)}")
    for section in catalog["sections"]:
        print(f"  {section['id']:22} {counts[section['id']]}")
    if errors:
        print(f"FAIL {len(errors)}")
        for error in errors:
            print(f"  {error}")
        return 1
    print("PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())

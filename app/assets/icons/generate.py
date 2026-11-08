#!/usr/bin/env python3
"""Writes app/src/brand_icons.rs from the Catalog, Simple Icons and a few Devicon SVGs.

    python3 -I app/assets/icons/generate.py \
        catalog/catalog.json <simple-icons package dir> <dir of Devicon SVGs> app/src/brand_icons.rs

Pinned sources: simple-icons 15.22.0 (npm) and Devicon v2.17.0.

An Item gets a logo only if the logo is its own or its vendor's. An independent tool built for
someone else's platform gets a letter tile instead, so `lazygit` doesn't wear Git's mark.
"""
import json
import os
import re
import sys

catalog_path, simple_dir, devicon_dir, out_path = sys.argv[1:5]

# A word that also names an unrelated brand. `delta` is the Git pager, not the airline.
NOT_THIS_BRAND = {"delta"}

# Item ID to Simple Icons slug: official products under their vendor's or project's mark.
ALIASES = {
    "node": "nodedotjs",
    "temurin": "eclipseadoptium",
    "safari-technology-preview": "safari",
    "jetbrains-toolbox": "jetbrains",
    "intellij-idea-oss": "intellijidea",
    "pycharm-oss": "pycharm",
    "zed": "zedindustries",
    "claude-code": "claude",
    "codex": "openai",
    "chatgpt": "openai",
    "copilot-cli": "githubcopilot",
    "cursor-cli": "cursor",
    "fish": "fishshell",
    "docker-desktop": "docker",
    "podman-desktop": "podman",
    "mongodb-compass": "mongodb",
    "redis-insight": "redis",
    "dbeaver-community": "dbeaver",
    "glab": "gitlab",
    "sublime-merge": "sublimetext",
    "github-desktop": "github",
    "android-sdk": "android",
    "android-platform-tools": "android",
    "gcloud": "googlecloud",
    "kubectl": "kubernetes",
    "minikube": "kubernetes",
    "kind": "kubernetes",
    "flyctl": "flydotio",
    "netlify-cli": "netlify",
    "wrangler": "cloudflare",
    "cloudflared": "cloudflare",
    "stripe-cli": "stripe",
    "firebase-cli": "firebase",
    "wireguard-tools": "wireguard",
    "1password-cli": "1password",
    "jetbrains-mono": "jetbrains",
    "geist-mono": "vercel",
    # Base Items, which the Catalog lists apart from the rest.
    "homebrew": "homebrew",
    "gh": "github",
    "command-line-tools": "apple",
}

# Item ID to (Devicon file, tile color), for marks Simple Icons doesn't carry.
DEVICON = {
    "visual-studio-code": ("vscode-plain", "007ACC"),
    "azure-cli": ("azure-plain", "0078D4"),
    "maven": ("maven-plain", "C71A36"),
    "powershell": ("powershell-plain", "2671BE"),
    "emacs": ("emacs-original", "7F5AB6"),
    "awscli": ("amazonwebservices-plain-wordmark", "232F3E"),
}

# Heading to Simple Icons slug. Only headings that name one language or platform get a mark.
GROUPS = {
    "JavaScript": "javascript",
    "Node.js version managers": "nodedotjs",
    "Python": "python",
    "Python version and project tools": "python",
    "Ruby": "ruby",
    "Go": "go",
    "Rust": "rust",
    "JVM": "openjdk",
    ".NET": "dotnet",
    "PHP": "php",
    "Swift": "swift",
    "JetBrains": "jetbrains",
    "Apple": "apple",
    "Android": "android",
    "Kubernetes": "kubernetes",
    "Git": "git",
}


def load_simple():
    raw = json.load(open(os.path.join(simple_dir, "data/simple-icons.json")))
    icons = raw["icons"] if isinstance(raw, dict) else raw
    by_slug = {}
    for icon in icons:
        slug = icon.get("slug") or title_slug(icon["title"])
        by_slug[slug] = icon
    return by_slug


def title_slug(title):
    text = title.lower().replace("+", "plus").replace(".", "dot").replace("&", "and")
    return re.sub(r"[^a-z0-9]", "", text)


def norm(text):
    return re.sub(r"[^a-z0-9]", "", text.lower())


def svg_parts(path):
    svg = open(path).read()
    view = re.search(r'viewBox="0 0 (\d+(?:\.\d+)?) ', svg)
    if not view:
        sys.exit(f"{path}: no square viewBox from the origin")
    tags = set(re.findall(r"<([a-zA-Z]+)", svg)) - {"svg", "title"}
    if tags != {"path"}:
        sys.exit(f"{path}: Rune's icons draw paths here, and this has {sorted(tags)}")
    paths = re.findall(r'<path[^>]*\sd="([^"]+)"', svg)
    return float(view.group(1)), paths


def luminance(hex_color):
    def channel(value):
        value /= 255
        return value / 12.92 if value <= 0.03928 else ((value + 0.055) / 1.055) ** 2.4

    r, g, b = (int(hex_color[i : i + 2], 16) for i in (0, 2, 4))
    return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)


def glyph_is_light(hex_color):
    """Paper wherever it's readable, as app icons do, and ink on tiles too light for it."""
    tile = luminance(hex_color)
    paper = luminance("FBF6EA")
    return (paper + 0.05) / (tile + 0.05) >= 3.0


def const_name(key):
    name = re.sub(r"[^A-Z0-9]", "_", key.upper())
    return f"I_{name}"


class Writer:
    def __init__(self):
        self.statics = {}

    def add(self, key, path, tile):
        if key not in self.statics:
            view, paths = svg_parts(path)
            body = "".join(f'<path d="{d}"/>' for d in paths)
            self.statics[key] = (view, body, tile.upper())
        return key

    def brand(self, key):
        view, body, tile = self.statics[key]
        return tile, glyph_is_light(tile)


def main():
    catalog = json.load(open(catalog_path))
    simple = load_simple()
    writer = Writer()
    item_keys = {}
    listed_ids = {item["id"] for item in catalog["items"]}
    known_ids = listed_ids | {recipe["id"] for recipe in catalog["recipes"]}

    def from_simple(slug):
        return writer.add(slug, os.path.join(simple_dir, "icons", f"{slug}.svg"), simple[slug]["hex"])

    for item_id, target in ALIASES.items():
        if item_id not in known_ids:
            sys.exit(f"alias for an Item the Catalog doesn't have: {item_id}")
        if target not in simple:
            sys.exit(f"alias to a slug Simple Icons doesn't have: {target}")
    for item_id in DEVICON:
        if item_id not in known_ids:
            sys.exit(f"Devicon entry for an Item the Catalog doesn't have: {item_id}")

    for item in catalog["items"]:
        item_id = item["id"]
        if item_id in DEVICON:
            name, tile = DEVICON[item_id]
            item_keys[item_id] = writer.add(name, os.path.join(devicon_dir, f"{name}.svg"), tile)
        elif item_id in ALIASES:
            item_keys[item_id] = from_simple(ALIASES[item_id])
        elif item_id not in NOT_THIS_BRAND:
            for candidate in (norm(item_id), norm(item["name"])):
                if candidate in simple:
                    item_keys[item_id] = from_simple(candidate)
                    break

    # Base Items are recipes without a Catalog listing, so the loop above doesn't reach them.
    for item_id, target in ALIASES.items():
        item_keys.setdefault(item_id, from_simple(target))

    group_keys = {}
    for heading, slug in GROUPS.items():
        if slug not in simple:
            sys.exit(f"group mark Simple Icons doesn't have: {slug}")
        group_keys[heading] = from_simple(slug)

    lines = [
        "//! Generated by app/assets/icons/generate.py, which says where the marks come from.",
        "//! Don't edit by hand: change the script's tables and run it again.",
        "",
        "use rune::Icon;",
        "",
        "/// A logo and the tile it sits on.",
        "pub struct Brand {",
        "    pub icon: &'static Icon,",
        "    /// The brand's own color, as `#RRGGBB`.",
        "    pub tile: &'static str,",
        "    /// Paper on this tile reads better than ink.",
        "    pub light_glyph: bool,",
        "}",
        "",
    ]
    for key in sorted(writer.statics):
        view, body, tile = writer.statics[key]
        name = const_name(key)
        light = "true" if glyph_is_light(tile) else "false"
        lines.append(
            f'static {name}: Icon = Icon {{ name: "brand-{key}", view: {view:.1f}, body: r#"{body}"# }};'
        )
        lines.append(
            f'static B{name[1:]}: Brand = Brand {{ icon: &{name}, tile: "#{tile}", light_glyph: {light} }};'
        )
    lines.append("")

    def table(fn_name, doc, keys):
        lines.append(f"/// {doc}")
        lines.append(f"pub fn {fn_name}(key: &str) -> Option<&'static Brand> {{")
        lines.append("    Some(match key {")
        for key in sorted(keys):
            lines.append(f'        "{key}" => &B{const_name(keys[key])[1:]},')
        lines.append("        _ => return None,")
        lines.append("    })")
        lines.append("}")
        lines.append("")

    table("for_item", "The mark for an Item, by its ID.", item_keys)
    table("for_group", "The mark for a group heading, by its words.", group_keys)

    with open(out_path, "w") as out:
        out.write("\n".join(lines).rstrip() + "\n")

    missing = sorted(listed_ids - set(item_keys))
    print(f"{len(writer.statics)} marks, {len(listed_ids & set(item_keys))} of {len(listed_ids)} listed Items have one")
    print("letter tiles:", ", ".join(missing))


main()

#!/usr/bin/env bash
# Install the Black Factory Claude Code harness into a project.
# Usage: harness/claude/install.sh <project-dir>
set -euo pipefail

if [ $# -ne 1 ] || [ ! -d "$1" ]; then
  echo "usage: install.sh <project-dir>" >&2
  echo "  <project-dir> must be an existing git repository." >&2
  exit 1
fi

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$here/../.." && pwd)"
project="$(cd "$1" && pwd -P)"

if [ ! -e "$project/.git" ]; then
  echo "refused: $project is not a git repository (no .git inside it)." >&2
  echo "  1. Run: git init   inside the project, or pick the project's root folder." >&2
  echo "  2. Run this script again." >&2
  exit 2
fi

# Never write through a symbolic link: it could lead outside the project.
for p in "$project/.claude" "$project/.claude/skills" "$project/.claude/skills/factory" \
         "$project/.claude/agents" "$project/.claude/settings.json"; do
  if [ -L "$p" ]; then
    echo "refused: $p is a symbolic link; it could point outside the project." >&2
    echo "  1. Replace it with a real folder or file." >&2
    echo "  2. Run this script again." >&2
    exit 2
  fi
done

changed=0
note() { echo "  $1"; }

# install_file <source> <destination>: copy when missing or different; keep a .bak of a replaced file.
install_file() {
  local src="$1" dst="$2"
  mkdir -p "$(dirname "$dst")"
  if [ -f "$dst" ]; then
    if cmp -s "$src" "$dst"; then
      note "unchanged  ${dst#$project/}"
      return
    fi
    cp "$dst" "$dst.bak"
    cp "$src" "$dst"
    note "updated    ${dst#$project/}  (old copy: ${dst#$project/}.bak)"
  else
    cp "$src" "$dst"
    note "created    ${dst#$project/}"
  fi
  changed=1
}

echo "Installing into $project"
install_file "$here/skills/factory/SKILL.md" "$project/.claude/skills/factory/SKILL.md"
for a in builder reviewer verifier; do
  install_file "$here/agents/$a.md" "$project/.claude/agents/$a.md"
done

# The page template the skill renders pages with.
tpl="$repo_root/views/templates/page.html"
if [ ! -f "$tpl" ]; then
  found=("$repo_root"/views/templates/*.html)
  if [ "${#found[@]}" -eq 1 ] && [ -f "${found[0]}" ]; then tpl="${found[0]}"; else tpl=""; fi
fi
if [ -n "$tpl" ]; then
  install_file "$tpl" "$project/.claude/skills/factory/page.html"
else
  echo "  warning    no page template found in views/templates/; pages cannot be rendered until .claude/skills/factory/page.html exists" >&2
fi

# Merge the hook and permission fragment into settings.json without removing anything.
python3 - "$here/settings.fragment.json" "$project/.claude/settings.json" "$project" <<'PY'
import json, os, shutil, sys

frag_path, settings_path, project = sys.argv[1:4]
rel = os.path.relpath(settings_path, project)

with open(frag_path) as f:
    frag = json.load(f)

# The engine's state folder: $BF_STATE_HOME, else $XDG_STATE_HOME/blackfactory, else ~/.local/state/blackfactory.
home = os.path.expanduser("~")
state = os.environ.get("BF_STATE_HOME")
if not state:
    xdg = os.environ.get("XDG_STATE_HOME")
    state = os.path.join(xdg, "blackfactory") if xdg else os.path.join(home, ".local", "state", "blackfactory")
state = os.path.abspath(state)
if state == home or state == "/":
    print("refused: the state folder resolves to %s; set BF_STATE_HOME to a folder of its own." % state, file=sys.stderr)
    sys.exit(2)
if state.startswith(home + os.sep):
    pat = "~/" + os.path.relpath(state, home) + "/**"
else:
    pat = "//" + state.lstrip("/") + "/**"
wanted_deny = list(frag.get("permissions", {}).get("deny", []))
for tool in ("Read", "Edit"):
    rule = "%s(%s)" % (tool, pat)
    if rule not in wanted_deny:
        wanted_deny.append(rule)

settings = {}
existed = os.path.exists(settings_path)
if existed:
    with open(settings_path) as f:
        text = f.read()
    if text.strip():
        try:
            settings = json.loads(text)
        except ValueError as e:
            print("refused: %s is not valid JSON (%s)." % (rel, e), file=sys.stderr)
            print("  1. Fix or move that file.", file=sys.stderr)
            print("  2. Run this script again.", file=sys.stderr)
            sys.exit(2)
    if not isinstance(settings, dict):
        print("refused: %s is not a JSON object." % rel, file=sys.stderr)
        sys.exit(2)

added = []

# Permission deny rules: add the missing ones, keep everything else.
perms = settings.setdefault("permissions", {})
if not isinstance(perms, dict):
    print("refused: 'permissions' in %s is not an object." % rel, file=sys.stderr)
    sys.exit(2)
deny = perms.setdefault("deny", [])
if not isinstance(deny, list):
    print("refused: 'permissions.deny' in %s is not a list." % rel, file=sys.stderr)
    sys.exit(2)
for rule in wanted_deny:
    if rule not in deny:
        deny.append(rule)
        added.append("deny rule: " + rule)

hooks = settings.setdefault("hooks", {})
if not isinstance(hooks, dict):
    print("refused: 'hooks' in %s is not an object." % rel, file=sys.stderr)
    sys.exit(2)

def key(cmd):
    # "bf guard --root x" and "bf guard" are the same hook: compare the first two words.
    return " ".join((cmd or "").split()[:2])

for event, entries in frag.get("hooks", {}).items():
    current = hooks.setdefault(event, [])
    for entry in entries:
        want = [h.get("command") for h in entry.get("hooks", [])]
        wkeys = [key(c) for c in want]
        holders = [c for c in current
                   if any(key(h.get("command")) in wkeys for h in c.get("hooks", []))]
        if holders:
            # Already wired. Bring an older matcher or command up to date, touch nothing else.
            for c in holders:
                if "matcher" in entry and c.get("matcher") != entry["matcher"]:
                    c["matcher"] = entry["matcher"]
                    added.append("%s: matcher of %s now %s" % (event, ", ".join(want), entry["matcher"]))
                for h in c.get("hooks", []):
                    for w in entry.get("hooks", []):
                        if key(h.get("command")) == key(w.get("command")) and h.get("command") != w.get("command"):
                            h["command"] = w["command"]
                            added.append("%s: command now %s" % (event, w["command"]))
            continue
        current.append(entry)
        added.append("%s: %s" % (event, ", ".join(want)))

if not added:
    print("  unchanged  %s  (hooks and deny rules already present)" % rel)
    sys.exit(0)

os.makedirs(os.path.dirname(settings_path), exist_ok=True)
if existed:
    bak = settings_path + ".bak"
    n = 0
    while os.path.exists(bak):
        n += 1
        bak = "%s.bak.%d" % (settings_path, n)
    shutil.copy2(settings_path, bak)
    print("  backup     %s" % os.path.relpath(bak, project))
with open(settings_path, "w") as f:
    json.dump(settings, f, indent=2)
    f.write("\n")
print("  %s %s" % ("updated   " if existed else "created   ", rel))
for a in added:
    print("    added  " + a)
PY

echo
if command -v bf >/dev/null 2>&1; then
  echo "bf found at $(command -v bf)."
else
  echo "warning: 'bf' is not on your PATH. The hooks call it by name. Build it with 'cargo build --release' and put target/release/bf on your PATH." >&2
fi
echo "Next: cd into the project, run 'bf init --name <project>', start Claude Code there and type /factory."

#!/usr/bin/env bash
# Install the Black Factory Claude Code harness into a project folder or into a workspace.
# Usage: harness/claude/install.sh [--workspace] <dir>
#   <dir> is either a project (a git repository that holds the code) or a workspace
#   (a folder made with `bf init --workspace`, which holds only the factory).
#   Everything goes into <dir>/.claude. A code repository that a workspace points at is never touched:
#   pass the workspace, not the repository. With --workspace the script refuses a folder that is not one.
set -euo pipefail

workspace_flag=0
if [ "${1:-}" = "--workspace" ]; then
  workspace_flag=1
  shift
fi
if [ $# -ne 1 ] || [ ! -d "$1" ]; then
  echo "usage: install.sh [--workspace] <dir>" >&2
  echo "  <dir> must be an existing git repository (a project) or a workspace made with 'bf init --workspace'." >&2
  exit 1
fi

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$here/../.." && pwd)"
project="$(cd "$1" && pwd -P)"

mode="project"
if [ -f "$project/factory/workspace.toml" ]; then
  mode="workspace"
fi
if [ "$workspace_flag" = 1 ] && [ "$mode" != "workspace" ]; then
  echo "refused: $project is not a workspace (no factory/workspace.toml in it)." >&2
  echo "  1. Make the workspace first: mkdir <name> && cd <name> && bf init --workspace --name <name>" >&2
  echo "  2. Run this script again with that folder." >&2
  exit 2
fi

# A folder inside a workspace is a code repository, not the place for the factory's files.
if [ "$mode" = "project" ]; then
  d="$(dirname "$project")"
  while [ "$d" != "/" ] && [ -n "$d" ]; do
    if [ -f "$d/factory/workspace.toml" ]; then
      echo "refused: $project is inside the workspace $d; its own files must stay free of the factory's." >&2
      echo "  1. Run this script with the workspace instead: install.sh --workspace $d" >&2
      echo "  2. Start Claude Code in the workspace folder." >&2
      exit 2
    fi
    d="$(dirname "$d")"
  done
fi

if [ ! -e "$project/.git" ]; then
  echo "refused: $project is not a git repository (no .git inside it)." >&2
  if [ "$workspace_flag" = 1 ]; then
    echo "  1. Run: bf init --workspace --name <name>   inside the workspace folder (it makes the folder a git repository)." >&2
  else
    echo "  1. Run: git init   inside the project, or pick the project's root folder. For a workspace run bf init --workspace first." >&2
  fi
  echo "  2. Run this script again." >&2
  exit 2
fi

# Never write through a symbolic link: it could lead outside the project.
for p in "$project/.claude" "$project/.claude/skills" "$project/.claude/skills/factory" \
         "$project/.claude/skills/factory-panel" "$project/.claude/agents" "$project/.claude/settings.json"; do
  if [ -L "$p" ]; then
    echo "refused: $p is a symbolic link; it could point outside the project." >&2
    echo "  1. Replace it with a real folder or file." >&2
    echo "  2. Run this script again." >&2
    exit 2
  fi
done

# Check every place the panel would write, and every .bak, for a symbolic link before writing anything.
panel_check_src="$here/panel"
panel_check_dst="$project/.claude/skills/factory-panel"
if [ -d "$panel_check_src" ]; then
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    d="$panel_check_dst/$(dirname "$f")"
    while [ "$d" != "$panel_check_dst" ] && [ "$d" != "." ] && [ "$d" != "/" ]; do
      if [ -L "$d" ]; then
        echo "refused: $d is a symbolic link; it could point outside the project." >&2
        echo "  1. Replace it with a real folder." >&2
        echo "  2. Run this script again." >&2
        exit 2
      fi
      d="$(dirname "$d")"
    done
    for p in "$panel_check_dst/$f" "$panel_check_dst/$f.bak"; do
      if [ -L "$p" ]; then
        echo "refused: $p is a symbolic link; it could point outside the project." >&2
        echo "  1. Replace it with a real file or remove it." >&2
        echo "  2. Run this script again." >&2
        exit 2
      fi
    done
  done <<EOF_CHECK
$(cd "$panel_check_src" && find . -type f -not -path './tests/*' -not -path './fixtures/*' | sed 's|^\./||' | LC_ALL=C sort)
EOF_CHECK
fi

changed=0
not_installed=0
note() { echo "  $1"; }

# install_file <source> <destination>: copy when missing or different; keep a .bak of a replaced file.
install_file() {
  local src="$1" dst="$2"
  if [ -L "$dst" ]; then
    echo "refused: $dst is a symbolic link; it could point outside the project." >&2
    echo "  1. Replace it with a real file or remove it." >&2
    echo "  2. Run this script again." >&2
    exit 2
  fi
  mkdir -p "$(dirname "$dst")"
  if [ -f "$dst" ]; then
    if cmp -s "$src" "$dst"; then
      note "unchanged  ${dst#$project/}"
      return
    fi
    if [ -L "$dst.bak" ]; then
      echo "refused: $dst.bak is a symbolic link; it could point outside the project." >&2
      echo "  1. Remove it." >&2
      echo "  2. Run this script again." >&2
      exit 2
    fi
    if cp "$dst" "$dst.bak" && cp "$src" "$dst"; then
      note "updated    ${dst#$project/}  (old copy: ${dst#$project/}.bak)"
    else
      echo "  not installed  ${dst#$project/}  (the copy failed)" >&2
      not_installed=$((not_installed + 1))
      return
    fi
  else
    if cp "$src" "$dst"; then
      note "created    ${dst#$project/}"
    else
      echo "  not installed  ${dst#$project/}  (the copy failed)" >&2
      not_installed=$((not_installed + 1))
      return
    fi
  fi
  changed=1
}

# render_rules <relative path below harness/claude> <source> <output>: write the source with the bullets of its
# "## Worker rules" section (or "## Worker rules of the conductor") taken from worker-rules.toml, the rows that name the file.
# A line of prose in that section is kept after the bullets.
render_rules() {
  python3 - "$here/worker-rules.toml" "$1" "$2" "$3" <<'PYR'
import sys, json, re
data, rel, src, out = sys.argv[1:5]
# worker-rules.toml has a fixed shape: [[rule]] tables of key = "string" and key = ["string", ...] (an array may span
# lines), plus # comment lines. Read it with the standard library only, so that Python 3.9 works.
def value(v):
    return json.loads(re.sub(r",\s*\]", "]", v))
rules, cur, pending = [], None, None
for raw in open(data).read().split("\n"):
    line = raw.strip()
    if pending is not None:
        pending[1] += " " + line
        if pending[1].count("[") == pending[1].count("]"):
            cur[pending[0]] = value(pending[1])
            pending = None
        continue
    if not line or line.startswith("#"):
        continue
    if line == "[[rule]]":
        cur = {}
        rules.append(cur)
        continue
    k, _, v = line.partition("=")
    k, v = k.strip(), v.strip()
    if cur is None or not k:
        sys.exit("worker-rules.toml: unexpected line: " + raw)
    if v.startswith("[") and v.count("[") != v.count("]"):
        pending = [k, v]
    else:
        cur[k] = value(v)
bullets = ["- " + r["text"] for r in rules if rel in r["files"]]
lines = open(src).read().split("\n")
res, i = [], 0
while i < len(lines):
    line = lines[i]
    res.append(line)
    i += 1
    if line in ("## Worker rules", "## Worker rules of the conductor"):
        j = i
        while j < len(lines) and not lines[j].startswith("## "):
            j += 1
        prose = [l for l in lines[i:j] if l.strip() and not l.startswith("- ")]
        res.append("")
        res.extend(bullets)
        if prose:
            res.append("")
            res.extend(prose)
        res.append("")
        i = j
text = "\n".join(res)
if not text.endswith("\n"):
    text += "\n"
text = text.replace("\n\n\n", "\n\n")
open(out, "w").write(text)
PYR
}

rendered="$(mktemp -d)"
trap 'rm -rf "$rendered"' EXIT

echo "Installing into $project ($mode)"
# The conductor's skill and the reference files beside it: every .md file of skills/factory/.
for s in "$here"/skills/factory/*.md; do
  if [ ! -f "$s" ]; then
    echo "  not installed  skills/factory/*.md  (no skill file found in the harness folder)" >&2
    not_installed=$((not_installed + 1))
    break
  fi
  if [ "$(basename "$s")" = "SKILL.md" ]; then
    render_rules "skills/factory/SKILL.md" "$s" "$rendered/SKILL.md"
    install_file "$rendered/SKILL.md" "$project/.claude/skills/factory/SKILL.md"
  else
    install_file "$s" "$project/.claude/skills/factory/$(basename "$s")"
  fi
done
# Every role file of the harness folder.
agent_names=""
for a in "$here"/agents/*.md; do
  if [ ! -f "$a" ]; then
    echo "  not installed  agents/*.md  (no role file found in the harness folder)" >&2
    not_installed=$((not_installed + 1))
    break
  fi
  agent_names="$agent_names$(basename "$a")
"
  render_rules "agents/$(basename "$a")" "$a" "$rendered/$(basename "$a")"
  install_file "$rendered/$(basename "$a")" "$project/.claude/agents/$(basename "$a")"
done
# A role file the project holds that the harness no longer has (for example one that was renamed) is named and left.
if [ -d "$project/.claude/agents" ]; then
  for g in "$project"/.claude/agents/*.md; do
    [ -f "$g" ] || continue
    if ! printf '%s' "$agent_names" | grep -qxF -- "$(basename "$g")"; then
      note "left       .claude/agents/$(basename "$g")  (the harness no longer has it; remove it by hand if you do not use it)"
    fi
  done
fi

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

# The panel for the Claude desktop app: every file of panel/ but tests/ and fixtures/.
panel_src="$here/panel"
panel_dst="$project/.claude/skills/factory-panel"
if [ -d "$panel_src" ]; then
  panel_files="$(cd "$panel_src" && find . -type f -not -path './tests/*' -not -path './fixtures/*' | sed 's|^\./||' | LC_ALL=C sort)"
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    install_file "$panel_src/$f" "$panel_dst/$f"
  done <<EOF_PANEL
$panel_files
EOF_PANEL
  # A file the installed folder holds that the panel no longer has is named and left.
  if [ -d "$panel_dst" ]; then
    while IFS= read -r g; do
      [ -n "$g" ] || continue
      case "$g" in *.bak) continue ;; esac
      if ! printf '%s\n' "$panel_files" | grep -qxF -- "$g"; then
        note "left       .claude/skills/factory-panel/$g  (the panel no longer has it)"
      fi
    done <<EOF_OLD
$(cd "$panel_dst" && find . -type f | sed 's|^\./||' | LC_ALL=C sort)
EOF_OLD
  fi
else
  echo "  warning    no panel found in harness/claude/panel/; the panel is not installed" >&2
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
if [ "$not_installed" -gt 0 ]; then
  echo "warning: $not_installed file(s) could not be installed; each is named above as 'not installed'." >&2
fi
if command -v bf >/dev/null 2>&1; then
  echo "bf found at $(command -v bf)."
else
  echo "warning: 'bf' is not on your PATH. The hooks call it by name. Build it with 'cargo build --release' and put target/release/bf on your PATH." >&2
fi
if [ "$mode" = "workspace" ]; then
  echo "Next: commit the new files in the workspace, start Claude Code in the workspace folder (not in a code repository) and type /factory, then say what you want built."
else
  echo "Next: cd into the project, run 'bf init --name <project>', commit the new files, start Claude Code there, type /factory, then say what you want built."
fi
if [ "$not_installed" -gt 0 ]; then
  exit 1
fi

#!/bin/sh
# Spike (D-140, U-126): do the git and sandbox-exec processes of the suite and of a unit lifecycle load
# macOS opendirectoryd, and would in-process git save it? Measurement only: this script changes no
# engine code, no test and no setting. Run from the root of the working copy.
# Modes: micro [N], lifecycle, suite, verify. Prints only key = value lines.
# Environment: N (calls per variant, default 1000), PAR (parallel width, default 12).

REC=tools/measures/git-spawn-spike.toml
DOC=tools/measures/git-spawn-spike.md
N=${N:-1000}
PAR=${PAR:-12}
GIT=${GIT:-/usr/bin/git}
SBX=/usr/bin/sandbox-exec

kv() { printf '%s = %s\n' "$1" "$2"; }
q() { printf '"%s"' "$1"; }
load_now() { sysctl -n vm.loadavg | awk '{print $2}'; }
odpid() { pgrep -x opendirectoryd | head -1; }
# ps cputime ([[h:]m:]s.cs) as seconds
cpu_of() {
  p=$(odpid)
  [ -n "$p" ] || { echo 0; return; }
  ps -o cputime= -p "$p" | awk '{n=split($1,a,":"); s=0; for(i=1;i<=n;i++) s=s*60+a[i]; printf "%.2f\n", s}'
}
sub() { awk -v a="$1" -v b="$2" 'BEGIN{printf "%.2f\n", a-b}'; }
per() { awk -v a="$1" -v n="$2" 'BEGIN{ if(n>0) printf "%.4f\n", a/n; else print 0 }'; }
per1000() { awk -v a="$1" -v n="$2" 'BEGIN{ if(n>0) printf "%.2f\n", a*1000/n; else print 0 }'; }

# ---------------------------------------------------------------- verify
verify() {
  [ -f "$REC" ] || { kv spike "$(q "missing $REC")"; exit 6; }
  [ -f "$DOC" ] || { kv spike "$(q "missing $DOC")"; exit 6; }
  for k in recommendation recommendation_read_only; do
    v=$(awk -v k="$k" '$1==k && $2=="="{print $3; exit}' "$REC")
    case "$v" in '"go"' | '"no-go"') ;; *) kv spike "$(q "missing $k")"; exit 6 ;; esac
  done
  for k in read_only_share_lifecycle read_only_share_suite load_before suite_git_peak \
    lifecycle_git_starts odd_cpu_per_1000_plain odd_cpu_per_1000_env; do
    grep -q -E "^$k = " "$REC" || { kv spike "$(q "missing $k")"; exit 6; }
  done
  c=$(grep -c -E '^## ' "$DOC")
  [ "$c" -ge 7 ] || { kv spike "$(q "fewer than seven sections in $DOC")"; exit 6; }
  kv spike '"ok"'
}

# ---------------------------------------------------------------- micro
# One variant: n calls of `git status`, plain or under sandbox-exec with the engine's git profile form,
# one at a time (width 1) or width PAR at a time. Children's CPU (git, plus sandbox-exec when used) comes
# from /usr/bin/time -p; opendirectoryd's CPU change is read before and after.
# variant <label> <sandbox 0|1> <width> <git program> <env exports>
variant() {
  label=$1; sbx=$2; width=$3; gitbin=$4; envs=$5
  if [ "$sbx" = 1 ]; then
    call="$SBX -p \"\$PROFILE\" $gitbin status"
  else
    call="$gitbin status"
  fi
  per_worker=$((N / width))
  [ "$per_worker" -ge 1 ] || per_worker=1
  total=$((per_worker * width))
  body="$envs; cd \"$SCR/repo\" || exit 1; PROFILE='$PROFILE'; w=0
while [ \$w -lt $width ]; do
  ( i=0; while [ \$i -lt $per_worker ]; do $call >/dev/null 2>&1; i=\$((i+1)); done ) &
  w=\$((w+1))
done
wait"
  b=$(cpu_of)
  /usr/bin/time -p sh -c "$body" 2>"$SCR/time.txt"
  a=$(cpu_of)
  cpu=$(awk '$1=="user"||$1=="sys"{s+=$2} END{printf "%.2f", s}' "$SCR/time.txt")
  kv "micro_${label}_calls" "$total"
  kv "micro_${label}_cpu_per_call" "$(per "$cpu" "$total")"
  kv "micro_${label}_odd_cpu_per_1000" "$(per1000 "$(sub "$a" "$b")" "$total")"
}

micro() {
  [ -x "$GIT" ] || { kv spike "$(q "no git at $GIT")"; exit 2; }
  SCR=$(mktemp -d "${TMPDIR:-/tmp}/git-spawns.XXXXXX") || exit 2
  trap 'rm -rf "$SCR"' EXIT
  mkdir -p "$SCR/repo" "$SCR/state" "$SCR/home"
  ( cd "$SCR/repo" && "$GIT" init -q -b main && echo x >README && "$GIT" add -A &&
    "$GIT" -c user.name=a -c user.email=a@b commit -q -m init ) >/dev/null 2>&1
  PROFILE="(version 1)
(allow default)
(deny network*)
(deny file-read* file-write* (subpath \"$SCR/state\"))
(deny file-write* (subpath \"$SCR/repo/factory\"))
"
  REAL=$(xcrun -f git 2>/dev/null)
  kv date "$(q "$(date -u +%Y-%m-%dT%H:%M:%SZ)")"
  kv load_before "$(load_now)"
  kv odpid_found "$( [ -n "$(odpid)" ] && echo true || echo false )"
  kv calls_per_variant "$N"
  kv width "$PAR"
  ID="export GIT_AUTHOR_NAME=a GIT_AUTHOR_EMAIL=a@b GIT_COMMITTER_NAME=a GIT_COMMITTER_EMAIL=a@b"
  for sbx in 0 1; do
    s=plain; [ "$sbx" = 1 ] && s=sbx
    for w in 1 "$PAR"; do
      m=seq; [ "$w" != 1 ] && m=par$w
      variant "${s}_${m}_base" "$sbx" "$w" "$GIT" ":"
      variant "${s}_${m}_home" "$sbx" "$w" "$GIT" "export HOME=\"$SCR/home\""
      variant "${s}_${m}_nosystem" "$sbx" "$w" "$GIT" "export GIT_CONFIG_NOSYSTEM=1"
      variant "${s}_${m}_ident" "$sbx" "$w" "$GIT" "$ID"
      [ -n "$REAL" ] && variant "${s}_${m}_noxcrun" "$sbx" "$w" "$REAL" ":"
      variant "${s}_${m}_all" "$sbx" "$w" "${REAL:-$GIT}" "export HOME=\"$SCR/home\" GIT_CONFIG_NOSYSTEM=1; $ID"
    done
  done
  kv load_after "$(load_now)"
}

# ---------------------------------------------------------------- lifecycle
# One unit's start, evidence run, two reports and land, in a throwaway project made with the installed
# release bf, whose policy names a logging wrapper as its git. Each step prints the git starts it made;
# sandbox-exec starts equal the git starts made under the sandbox (one wrapper per git call) plus one
# for the evidence run's command, so that count is marked as derived.
lifecycle() {
  command -v bf >/dev/null || { kv spike '"no bf installed"'; exit 2; }
  T=$(mktemp -d "${TMPDIR:-/tmp}/git-life.XXXXXX") || exit 2
  trap 'rm -rf "$T"' EXIT
  T=$(cd "$T" && pwd -P)
  ROOT=$T/proj; mkdir -p "$ROOT/factory" "$T/state" "$T/tr" "$T/gitlog"
  LOG=$T/gitlog/calls.log; : >"$LOG"
  printf '#!/bin/sh\nprintf "x\\n" >> "%s"\nexec "%s" "$@"\n' "$LOG" "$GIT" >"$T/gitlog/git"
  chmod 755 "$T/gitlog/git"
  export BF_STATE_HOME=$T/state
  ( cd "$ROOT" && "$GIT" init -q -b main && echo "# toy" >README.md && "$GIT" add -A &&
    "$GIT" -c user.name=a -c user.email=a@b commit -q -m init ) >/dev/null 2>&1
  # the policy of a fresh project, with the git program swapped for the logging wrapper
  ( cd "$ROOT" && bf init --name toy ) >/dev/null 2>&1 || { kv spike '"bf init failed"'; exit 2; }
  sed -e "s|^git = .*|git = \"$T/gitlog/git\"|" \
      -e 's|^tokens = true|tokens = false|' -e 's|^in_order = true|in_order = false|' \
      -e "s|^transcript_root = .*|transcript_root = \"$T/tr\"|" "$ROOT/factory/policy.toml" >"$T/policy.new"
  cp "$T/policy.new" "$ROOT/factory/policy.toml"
  hum() {
    printf '{"session_id":"0b6f1c3e-aaaa-bbbb-cccc-1234567890ab","transcript_path":"%s/tr/transcript.jsonl","cwd":"%s","permission_mode":"default","hook_event_name":"UserPromptSubmit","prompt":"%s"}' \
      "$T" "$ROOT" "$1" | ( cd "$ROOT" && bf human-turn ) >"$T/last.out" 2>&1 || fail "the human turn of a setup step"
    printf '{"type":"user","message":{"role":"user","content":"%s"}}\n' "$1" >>"$T/tr/transcript.jsonl"
  }
  # every setup command must succeed, or the mode stops and prints which one failed
  # bf's own output (what, why, steps) is printed to stderr before the key = value line
  fail() { # fail <what>
    sed 's/^/bf: /' "$T/last.out" >&2
    kv spike "$(q "failed: $1")"; exit 3
  }
  B() { ( cd "$ROOT" && bf "$@" ) >"$T/last.out" 2>&1 || fail "bf $*"; }
  hum "Yes, I accept the new policy."; B policy accept --quote "accept the new policy"
  # as the test fixture does right after init: every named check switched off, decided by the human's words
  ( cd "$ROOT" && bf checks list --json ) >"$T/checks.json" 2>"$T/last.out" || fail "bf checks list --json"
  CH=$(grep -o '"name": *"[^"]*"' "$T/checks.json" | sed 's/.*: *"\(.*\)"/\1/' | sort -u)
  [ -n "$CH" ] || { cp "$T/checks.json" "$T/last.out"; fail "no check names in bf checks list --json"; }
  set --
  for c in $CH; do set -- "$@" "$c=off:the spike does not exercise it"; done
  B checks propose "$@"
  hum "Yes, switch those checks off for this spike."; B checks set --quote "switch those checks off for this spike"
  put() { printf '%s\n' "$2" >"$T/rec.toml"; B record put "$1" --file "$T/rec.toml"; }
  put requirement 'id="R-1"
kind="requirement"
title="A greeting exists"
acceptance=["src/hello.txt exists"]'
  put unit 'id="U-1"
kind="unit"
title="Greeting"
delivers=["R-1"]
files=["src/hello.txt"]
checkpoint="K-1"
estimate=1
full_suite="fixture"'
  put check 'id="X-1"
kind="check"
title="Greeting file"
how="command"
command="test -f src/hello.txt"
proves=["R-1"]
scope="unit"
unit="U-1"'
  put checkpoint 'id="K-1"
kind="checkpoint"
stop_why="Spike."
title="Greeting"
after=["U-1"]
proves=["R-1"]
checks=["X-1"]'
  ( cd "$ROOT" && "$GIT" add -A && "$GIT" -c user.name=a -c user.email=a@b commit -q -m records ) >/dev/null 2>&1
  hum "yes, make this a test run with simulated approvals"
  B test-mode grant --quote "make this a test run with simulated approvals"
  B run start
  B checks wave propose none; hum "No wave checks for this run."; B checks wave set --quote "No wave checks"
  B checks unit propose none; hum "No checks for every unit in this run."; B checks unit set --quote "No checks for every unit"
  B gate open plan; B gate approve plan --simulated
  WC=$ROOT/.bf/work/U-1
  : >"$LOG"
  kv load_before "$(load_now)"
  ODB=$(cpu_of); prev=0; total_od_start=$ODB
  step() { # step <name> <command...>
    nm=$1; shift
    b=$(cpu_of)
    "$@" >"$T/last.out" 2>&1 || fail "step $nm"
    a=$(cpu_of)
    n=$(wc -l <"$LOG" | tr -d ' ')
    kv "lifecycle_${nm}_git_starts" "$((n - prev))"
    kv "lifecycle_${nm}_sandbox_starts_derived" "$((n - prev))"
    kv "lifecycle_${nm}_odd_cpu" "$(sub "$a" "$b")"
    prev=$n
  }
  step start sh -c "cd '$ROOT' && bf unit start U-1"
  mkdir -p "$WC/src"; echo hi >"$WC/src/hello.txt"
  step evidence sh -c "cd '$WC' && bf evidence run X-1 --unit U-1 -- test -f src/hello.txt"
  step report_builder sh -c "cd '$ROOT' && bf report U-1 --result built"
  step report_reviewer sh -c "cd '$ROOT' && bf report U-1 --result built --role reviewer"
  step land sh -c "cd '$ROOT' && bf unit land U-1"
  kv lifecycle_git_starts "$prev"
  kv lifecycle_odd_cpu_total "$(sub "$(cpu_of)" "$total_od_start")"
  kv lifecycle_sandbox_starts_kind '"derived from the git starts"'
}

# ---------------------------------------------------------------- suite
# One extended suite run (after an untimed --no-run), sampled every 0.5 s for running git and
# sandbox-exec processes. Git starts of the whole suite: the set of distinct pids seen by the sampler is a
# lower bound (short processes can fall between samples); the estimate of a lifecycle's starts times the
# lifecycles the suite runs is the other way, and the record says which it used.
suite() {
  command -v cargo-nextest >/dev/null || command -v cargo >/dev/null || { kv spike '"no cargo"'; exit 2; }
  S=$(mktemp -d "${TMPDIR:-/tmp}/git-suite.XXXXXX") || exit 2
  trap 'rm -rf "$S"' EXIT
  cargo nextest run --package=bf --profile extended --no-run >/dev/null 2>&1 || { kv spike '"build failed"'; exit 2; }
  kv load_before "$(load_now)"
  : >"$S/samples"; : >"$S/pids"
  b=$(cpu_of)
  /usr/bin/time -l cargo nextest run --package=bf --profile extended --no-fail-fast >/dev/null 2>"$S/time.txt" &
  RUNPID=$!
  while kill -0 "$RUNPID" 2>/dev/null; do
    ps -axo pid=,comm= | awk -v out="$S/pids" '
      { n=split($2,a,"/"); c=a[n] }
      c=="git" { g++; print $1 >> out }
      c=="sandbox-exec" { s++ }
      END { print g+0, s+0 }' >>"$S/samples"
    sleep 0.5
  done
  wait "$RUNPID"; rc=$?
  a=$(cpu_of)
  kv suite_exit "$rc"
  awk '{ if($1>gp)gp=$1; if($2>sp)sp=$2; g+=$1; s+=$2; n++ }
       END { printf "suite_git_peak = %d\nsuite_git_mean = %.2f\nsuite_sandbox_peak = %d\nsuite_sandbox_mean = %.2f\nsuite_samples = %d\n", gp, g/n, sp, s/n, n }' "$S/samples"
  kv suite_git_starts_seen "$(sort -u "$S/pids" | wc -l | tr -d ' ')"
  kv suite_git_starts_kind '"lower bound from samples"'
  awk '/real/ && $2=="real"{} {for(i=1;i<=NF;i++){ if($i=="real")w=$(i-1); if($i=="user")u=$(i-1); if($i=="sys")y=$(i-1)}}
       END { printf "suite_wall_seconds = %.2f\nsuite_cpu_seconds = %.2f\n", w, u+y }' "$S/time.txt"
  kv suite_odd_cpu "$(sub "$a" "$b")"
  kv load_after "$(load_now)"
}

case "${1:-}" in
  verify) verify ;;
  micro) [ -n "${2:-}" ] && N=$2; micro ;;
  lifecycle) lifecycle ;;
  suite) suite ;;
  *) kv usage '"git-spawns.sh micro [N] | lifecycle | suite | verify"'; exit 2 ;;
esac

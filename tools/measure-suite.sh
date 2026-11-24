#!/bin/sh
# Measures the extended suite of the bf package (D-140). Run from the root of the working copy.
# Modes: baseline run|finish, coverage, compare run|finish, verify, build.
# Prints only key = value lines.

CMD='cargo nextest run --package=bf --profile extended --no-fail-fast'
LISTCMD='cargo nextest list --package=bf --profile extended --message-format oneline'
BASE=tools/measures/suite-baseline.toml
NAMES=tools/measures/suite-tests.txt
RUNS=target/measure-runs
PROBE='(version 1)(allow default)(deny network*)'
SERIAL_PAT='controls::the_sandbox_costs_a_little_and_the_policy_can_switch_it_off'

E_PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
E_CARGO="${CARGO_HOME:-$HOME/.cargo}"
E_RUSTUP="${RUSTUP_HOME:-$HOME/.rustup}"
E_TMP=$(getconf DARWIN_USER_TEMP_DIR)
# the recorded environment holds no home path: the home folder is written as $HOME
homefree() { case "$1" in "$HOME") printf '%s' '$HOME' ;; "$HOME"/*) printf '%s' '$HOME'"${1#"$HOME"}" ;; *) printf '%s' "$1" ;; esac; }
ENVTXT="PATH=\$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin HOME=\$HOME CARGO_HOME=$(homefree "$E_CARGO") RUSTUP_HOME=$(homefree "$E_RUSTUP") TMPDIR=DARWIN_USER_TEMP_DIR"

# untimed command under the fixed environment
cleanenv() {
  env -i PATH="$E_PATH" HOME="$HOME" CARGO_HOME="$E_CARGO" RUSTUP_HOME="$E_RUSTUP" TMPDIR="$E_TMP" "$@"
}
# timed command under the fixed environment
timedenv() {
  /usr/bin/time -l env -i PATH="$E_PATH" HOME="$HOME" CARGO_HOME="$E_CARGO" RUSTUP_HOME="$E_RUSTUP" TMPDIR="$E_TMP" "$@"
}

load_now() { sysctl -n vm.loadavg | awk '{print $2}'; }
num_le() { awk -v a="$1" -v b="$2" 'BEGIN{exit !(a<=b)}'; }
median() { sort -n | awk '{a[NR]=$1} END{if(NR==0)exit; if(NR%2)print a[(NR+1)/2]; else printf "%.2f\n",(a[NR/2]+a[NR/2+1])/2}'; }
# value of a key in the top part of a file (before the first [[runs]])
topval() { awk -v k="$1" '/^\[\[runs\]\]/{exit} $1==k && $2=="="{sub(/^[^=]*= /,""); print; exit}' "$2"; }
# values of a key in the [[runs]] tables of a file
runvals() { awk -v k="$1" '/^\[\[runs\]\]/{r=1;next} r && $1==k && $2=="="{sub(/^[^=]*= /,""); print}' "$2"; }

refuse() { # refuse <code-exit> key=value...
  c=$1; shift
  for kv in "$@"; do printf '%s\n' "$kv"; done
  exit "$c"
}

probe_plain() { /usr/bin/sandbox-exec -p "$PROBE" /usr/bin/true >/dev/null 2>&1; }
probe_nested() { /usr/bin/sandbox-exec -p "$PROBE" /usr/bin/sandbox-exec -p "$PROBE" /usr/bin/true >/dev/null 2>&1; }
tf() { if "$@"; then echo true; else echo false; fi; }

# wait for a load the check allows: wait_load <mode> <limit> [center]
wait_load() {
  n=0
  while :; do
    l=$(load_now)
    if [ "$1" = max ]; then
      num_le "$l" "$2" && return 0
      lim="limit = $2"
    else
      awk -v a="$l" -v c="$3" 'BEGIN{d=a-c; if(d<0)d=-d; exit !(d<=1.0)}' && return 0
      lim="band = \"within 1.0 of $3\""
    fi
    if [ "$n" -ge 18 ]; then
      refuse 3 'refused = "load"' "load = $l" "$lim"
    fi
    n=$((n+1)); sleep 10
  done
}

# one timed run, appended to <file>: timed_run <file> <mode> <limit-or-center>
timed_run() {
  file=$1; mode=$2; arg=$3
  mkdir -p "$RUNS"
  if [ "$mode" = baseline ]; then
    probe_plain || refuse 3 'refused = "sandbox"' 'sandbox_probe = false'
  fi
  rm -rf target/tmp/bf-fixture-templates
  if [ "$mode" = baseline ]; then wait_load max "$arg"; else wait_load band "" "$arg"; fi
  lb=$(load_now)
  sp=$(tf probe_plain); sn=$(tf probe_nested)
  if [ "$mode" = baseline ] && [ "$sp" != true ]; then
    refuse 3 'refused = "sandbox"' 'sandbox_probe = false'
  fi
  out=$(mktemp "${TMPDIR:-/tmp}/measure.XXXXXX")
  # shellcheck disable=SC2086
  timedenv $CMD >"$out" 2>&1
  code=$?
  la=$(load_now)
  wall=$(awk '/ real .* user .* sys/{print $1; exit}' "$out")
  user=$(awk '/ real .* user .* sys/{print $3; exit}' "$out")
  sys=$(awk '/ real .* user .* sys/{print $5; exit}' "$out")
  cpu=$(awk -v u="$user" -v s="$sys" 'BEGIN{printf "%.2f", u+s}')
  sm=$(sed -n 's/.*Summary.* \([0-9][0-9]*\) tests\{0,1\} run: \([0-9][0-9]*\) passed.*/\1 \2/p' "$out" | tail -1)
  run=${sm% *}; pass=${sm#* }
  [ -n "$run" ] || { run=0; pass=0; }
  ser=$(awk -v pat="$SERIAL_PAT" '
    /PASS \[/ {
      if (match($0, /PASS \[ *[0-9.]+s\]/)) {
        t=substr($0,RSTART,RLENGTH); gsub(/[^0-9.]/,"",t)
        n=$NF
        if (n ~ /(^|::)timing::/ || n ~ /(^|::)timing_/ || index(n,pat)>0) s+=t
      }
    } END{printf "%.2f", s+0}' "$out")
  rm -f "$out"
  {
    echo '[[runs]]'
    echo "load_before = $lb"
    echo "load_after = $la"
    echo "wall_seconds = $wall"
    echo "cpu_seconds = $cpu"
    echo "user_seconds = $user"
    echo "sys_seconds = $sys"
    echo "exit = $code"
    echo "tests_run = $run"
    echo "tests_passed = $pass"
    echo "serial_seconds = $ser"
    echo "sandbox_probe = $sp"
    echo "sandbox_nested = $sn"
  } >>"$file"
  {
    echo "load_before = $lb"; echo "load_after = $la"; echo "wall_seconds = $wall"
    echo "cpu_seconds = $cpu"; echo "exit = $code"; echo "tests_run = $run"
    echo "tests_passed = $pass"; echo "serial_seconds = $ser"
    echo "sandbox_probe = $sp"; echo "sandbox_nested = $sn"
  }
  if [ "$mode" = baseline ] && ! num_le "$lb" 1.0; then echo 'load_aim = "above 1.0"'; fi
  [ "$code" -eq 0 ] || exit 4
  exit 0
}

# spread check on a key of a runs file: spread_ok <file> <key>
spread_bad() {
  runvals "$2" "$1" | sort -n | awk '{a[NR]=$1} END{
    m=(NR%2)?a[(NR+1)/2]:(a[NR/2]+a[NR/2+1])/2
    exit !((a[NR]-a[1]) > 0.15*m)}'
}
check_spread() { # file
  for k in wall_seconds cpu_seconds; do
    if spread_bad "$1" "$k"; then refuse 3 'refused = "spread"' "field = \"$k\""; fi
  done
}

list_names() { cleanenv $LISTCMD 2>/dev/null | awk 'NF{print}' | sort; }
count_asserts() { git grep -h -o -E '(^|[^A-Za-z0-9_])(debug_)?assert(_eq|_ne)?!' -- crates/bf | wc -l | tr -d ' '; }

baseline_finish() {
  f=$RUNS/baseline.toml
  [ -f "$f" ] || refuse 3 'refused = "setup"' 'field = "runs"'
  [ "$(runvals exit "$f" | grep -c '^0$')" -ge 3 ] || refuse 3 'refused = "setup"' 'field = "exit"'
  [ "$(runvals tests_run "$f" | sort -u | wc -l | tr -d ' ')" -eq 1 ] || refuse 3 'refused = "setup"' 'field = "tests_run"'
  for k in sandbox_probe sandbox_nested; do
    [ "$(runvals $k "$f" | sort -u | wc -l | tr -d ' ')" -eq 1 ] || refuse 3 'refused = "setup"' "field = \"$k\""
  done
  check_spread "$f"
  sp=$(runvals sandbox_probe "$f" | head -1); sn=$(runvals sandbox_nested "$f" | head -1)
  [ "$sp" = true ] || refuse 3 'refused = "sandbox"' 'sandbox_probe = false'
  names=$(list_names)
  [ -n "$names" ] || refuse 3 'refused = "setup"' 'field = "tests"'
  mkdir -p tools/measures
  printf '%s\n' "$names" >"$NAMES"
  nt=$(wc -l <"$NAMES" | tr -d ' ')
  {
    echo "command = \"$CMD\""
    echo 'profile = "extended"'
    echo "env = \"$ENVTXT\""
    echo "cores = $(sysctl -n hw.ncpu)"
    echo "machine = \"$(sysctl -n hw.model)\""
    echo "os = \"$(sw_vers -productVersion)\""
    echo "cargo_version = \"$(cleanenv cargo --version)\""
    echo "nextest_version = \"$(cleanenv cargo nextest --version | head -1)\""
    echo "commit = \"$(git rev-parse HEAD)\""
    echo "date = \"$(date -u +%Y-%m-%dT%H:%M:%SZ)\""
    echo "load_before = $(runvals load_before "$f" | median)"
    echo "wall_seconds = $(runvals wall_seconds "$f" | median)"
    echo "cpu_seconds = $(runvals cpu_seconds "$f" | median)"
    echo "serial_seconds = $(runvals serial_seconds "$f" | median)"
    echo "sandbox_probe = $sp"
    echo "sandbox_nested = $sn"
    echo "tests = $nt"
    echo "tests_run = $(runvals tests_run "$f" | head -1)"
    echo "asserts = $(count_asserts)"
    echo "suite_passed = true"
    cat "$f"
  } >"$BASE"
  echo "baseline = \"$BASE\""
  echo "tests = $nt"
  exit 0
}

coverage() {
  names=$(list_names)
  t=$(printf '%s\n' "$names" | awk 'NF' | wc -l | tr -d ' ')
  bt=$(wc -l <"$NAMES" | tr -d ' ')
  miss=$(printf '%s\n' "$names" | comm -23 "$NAMES" - | wc -l | tr -d ' ')
  a=$(count_asserts)
  ba=$(topval asserts "$BASE")
  echo "tests = $t"; echo "baseline_tests = $bt"; echo "missing = $miss"
  echo "asserts = $a"; echo "baseline_asserts = $ba"
  if [ "$miss" -eq 0 ] && [ "$t" -ge "$bt" ] && [ "$a" -ge "${ba:-0}" ]; then
    echo 'coverage = "kept"'; exit 0
  fi
  echo 'coverage = "fallen"'; exit 5
}

compare_run() {
  [ -f "$BASE" ] || refuse 3 'refused = "setup"' 'field = "baseline"'
  cur_machine="\"$(sysctl -n hw.model)\""
  cur_cores=$(sysctl -n hw.ncpu)
  cur_os="\"$(sw_vers -productVersion)\""
  cur_cargo="\"$(cleanenv cargo --version)\""
  cur_next="\"$(cleanenv cargo nextest --version | head -1)\""
  cur_probe=$(tf probe_plain)
  for pair in "machine:$cur_machine" "cores:$cur_cores" "os:$cur_os" "cargo_version:$cur_cargo" \
    "nextest_version:$cur_next" "command:\"$CMD\"" 'profile:"extended"' "env:\"$ENVTXT\"" "sandbox_probe:$cur_probe"; do
    k=${pair%%:*}; v=${pair#*:}
    [ "$(topval "$k" "$BASE")" = "$v" ] || refuse 3 'refused = "setup"' "field = \"$k\""
  done
  timed_run "$RUNS/compare.toml" compare "$(topval load_before "$BASE")"
}

compare_finish() {
  f=$RUNS/compare.toml
  [ -f "$f" ] && [ "$(runvals exit "$f" | wc -l | tr -d ' ')" -ge 3 ] || refuse 3 'refused = "setup"' 'field = "runs"'
  btr=$(topval tests_run "$BASE")
  bad=$(runvals exit "$f" | grep -vc '^0$')
  low=$(runvals tests_run "$f" | awk -v b="$btr" '$1<b' | wc -l | tr -d ' ')
  if [ "$bad" -ne 0 ] || [ "$low" -ne 0 ]; then
    refuse 7 'pass = false' 'failed = "suite"'
  fi
  check_spread "$f"
  nw=$(runvals wall_seconds "$f" | median); nc=$(runvals cpu_seconds "$f" | median)
  ns=$(runvals serial_seconds "$f" | median)
  bw=$(topval wall_seconds "$BASE"); bc=$(topval cpu_seconds "$BASE"); bs=$(topval serial_seconds "$BASE")
  wr=$(awk -v a="$bw" -v b="$nw" 'BEGIN{printf "%.2f", a/b}')
  cr=$(awk -v a="$bc" -v b="$nc" 'BEGIN{printf "%.2f", a/b}')
  echo "baseline_wall_seconds = $bw"; echo "wall_seconds = $nw"
  echo "baseline_cpu_seconds = $bc"; echo "cpu_seconds = $nc"
  echo "baseline_serial_seconds = $bs"; echo "serial_seconds = $ns"
  echo "wall_ratio = $wr"; echo "cpu_ratio = $cr"
  if awk -v a="$wr" -v b="$cr" 'BEGIN{exit !(a>=1.5 && b>=1.5)}'; then echo 'pass = true'; else echo 'pass = false'; fi
  if awk -v a="$wr" -v b="$cr" 'BEGIN{exit !(a>=2.0 && b>=2.0)}'; then echo 'target = true'; else echo 'target = false'; fi
  exit 0
}

verify() {
  bad() { echo "baseline = \"wrong\""; echo "field = \"$1\""; exit 6; }
  [ -f "$BASE" ] || bad file
  for k in command profile env cores machine os cargo_version nextest_version commit date load_before \
    wall_seconds cpu_seconds serial_seconds sandbox_probe sandbox_nested tests tests_run asserts suite_passed; do
    [ -n "$(topval $k "$BASE")" ] || bad "$k"
  done
  num_le "$(topval load_before "$BASE")" 3.0 || bad load_before
  [ "$(runvals exit "$BASE" | grep -c '^0$')" -ge 3 ] || bad runs
  [ "$(runvals exit "$BASE" | grep -vc '^0$')" -eq 0 ] || bad exit
  [ "$(topval suite_passed "$BASE")" = true ] || bad suite_passed
  [ "$(topval sandbox_probe "$BASE")" = true ] || bad sandbox_probe
  case $(topval sandbox_nested "$BASE") in true|false) ;; *) bad sandbox_nested ;; esac
  t=$(topval tests "$BASE")
  [ "$t" -gt 0 ] 2>/dev/null || bad tests
  [ -f "$NAMES" ] && [ "$(wc -l <"$NAMES" | tr -d ' ')" -eq "$t" ] || bad tests
  echo 'baseline = "ok"'
  exit 0
}

case "$1 $2" in
  "baseline run") timed_run "$RUNS/baseline.toml" baseline "3.0" ;;
  "baseline finish") baseline_finish ;;
  "compare run") compare_run ;;
  "compare finish") compare_finish ;;
  "build "*) cleanenv cargo nextest run --package=bf --profile extended --no-run >/dev/null 2>&1; echo "exit = $?" ;;
  "coverage "*) coverage ;;
  "verify "*) verify ;;
  *) echo 'refused = "mode"'; exit 2 ;;
esac

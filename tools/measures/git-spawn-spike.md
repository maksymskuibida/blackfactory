# Git spawn spike (U-126, D-140)

Measurement only. Every number and its kind (measured, derived or estimated) is in `git-spawn-spike.toml`.
The micro run had a one-minute load of 2.78 before it, the lifecycle run 2.02 and the suite run 2.41.

## What was measured and how

`sh tools/spike/git-spawns.sh micro` ran 1000 `git status` calls per variant in a scratch repository: plain
and under `sandbox-exec` with the engine's git profile form, one at a time and 12 at a time, each with the
cures `HOME` set, `GIT_CONFIG_NOSYSTEM=1`, the four identity variables, a git that skips the `xcrun` shim,
and all together. Git CPU is the children's user plus system time from `/usr/bin/time -p` (for the sandbox
variants it includes `sandbox-exec`). The `opendirectoryd` CPU change is `ps -o cputime=` before and after.
No root-free lookup count exists, so its CPU time is the measure.

`lifecycle` ran one unit's start, evidence run, two reports and land in a throwaway project whose policy
names a logging wrapper as its git. `suite` ran one `cargo nextest run --package=bf --profile extended` after
an untimed `--no-run`, sampling the running `git` and `sandbox-exec` processes every 0.5 s. The human ran both
modes, because a builder may not. The wrapper counts calls, not their arguments, so the split of calls by kind
(read-only or not) was not measured. The suite's total git starts cannot be counted without a code change, so
that figure is an estimate from the samples.

## Does opendirectoryd load scale with spawns?

Per call, yes, and the cost is small. Per 1000 plain calls `opendirectoryd` used 0.92 s of CPU (one at a time)
against 11.2 s of git CPU, about 8 percent; with the sandbox 1.06 s per 1000. A whole lifecycle (72 git starts)
moved it by 0.07 s (start 0.01, evidence 0.01, builder report 0.01, reviewer report 0.00, land 0.04), which
matches about 1 ms per call.

The suite does not follow the git count. The sampler saw at most 3 git processes at once and 0.08 on average
(53 distinct git pids seen, a lower bound), and never a `sandbox-exec` (the suite's own runs have the sandbox
off). So the claim of about 300 git processes at once is not what happened here: the peak was 3. Git starts
for the suite are estimated at about 2500 (mean 0.08 times 444 s, over about 0.015 s of life per call), about
45 s of 2958 s CPU (1.5 percent). Yet `opendirectoryd` used 94.16 s during the suite (3.2 percent of the CPU),
about 40 times what 2500 git calls explain (about 2.5 s). Its load comes mostly from something other than git:
the test processes and the `bf` binary the tests start, which do the same user and home lookups. Load rose to
25 during the run, so the machine was busy with the suite itself.

## What each environment-only cure saved

Plain, one at a time, git CPU per call and `opendirectoryd` CPU per 1000 calls (measured):

- none: 0.0112 s, 0.92 s
- `HOME` to a scratch folder: 0.0104 s, 0.90 s (7 percent of git CPU, nothing at the daemon)
- `GIT_CONFIG_NOSYSTEM=1`: 0.0104 s, 0.98 s (the same)
- identity variables: 0.0105 s, 0.98 s (the same)
- a git that skips the `xcrun` shim: 0.0050 s, 0.00 s (55 percent of git CPU, all of the daemon load)
- all together: 0.0041 s, 0.07 s

Under the sandbox the shim-free git is 0.0123 s against 0.0179 s (31 percent less), and 0.0173 s against
0.0274 s at 12 at a time (37 percent less). `/usr/bin/git` here is the shim and the engine's default policy
names it. Naming the real git program (the path `xcrun -f git` prints) in the policy is the one cure that
matters for git itself. The other three save at most 7 percent. This is only a recommendation: the policy
change is an amendment, and no unit of K-5 takes it without one. It is named for the human here. Its saving
is small: about 0.4 s per lifecycle and about 14 s of 2958 s per suite (0.5 percent, estimated).

## Estimated saving of in-process git

One lifecycle: 72 git starts, about 1.3 s of CPU (estimated from the measured 0.0179 s under the sandbox). The
most in-process git could remove is that 1.3 s plus the daemon's 0.07 s. One suite: about 45 s of git CPU and
about 3 s of daemon CPU from git (estimated), 1.6 percent of the 2958 s the suite uses. A K-5 goal of 1.5 times
lighter needs a third of the CPU gone; in-process git cannot approach that. The suite's own fixtures also call
the git program directly, so an engine-only change would reach only part of even that.

## Costs of in-process git, each named

- The sandbox boundary: a library call inside the engine's own process has no `sandbox-exec` around it, so the
  engine's git calls lose the denied state folder, the denied network and the read-only `factory/`.
- The hardening of running the git program: the fixed `PATH`, the `[exec] git` choice of program and the
  guard's view of it all go away for a library call.
- Behaviour differences the equal-output proofs would have to cover: content hashes, the index, racy-file
  handling, merges and conflict output.
- A permissive-licence crate with an entry in `NOTICE.md`. `gix` (gitoxide) is MIT or Apache-2.0 and fits, but
  it is a large dependency tree and its merge support is incomplete. `git2` is MIT or Apache-2.0 itself, but it
  links libgit2 (GPLv2 with a linking exception) and a C build, which `NOTICE.md` would have to explain.

## Recommendation: full in-process git

no-go. The measured numbers do not support the hypothesis: the suite peaked at 3 git processes, git is about
1.5 percent of the suite's CPU, and the daemon load of a git call is about 1 ms. The costs above are real. The
suite's daemon load (94 s) is mostly not git, so a later amendment on speed should look at the processes the
tests start, not at git.

## Read-only variant

In-process content hashes, `rev-list --count`, `for-each-ref` and `show-ref` only. Writes, merges, worktree acts
and the sandbox stay on the real git program.

- Share removed: not measured by kind. Assuming half the calls, about 36 of 72 per lifecycle (50 percent) and
  about 1250 of 2500 per suite (50 percent), estimated. The upper bound on the saving is about 0.65 s of CPU per
  lifecycle and about 22 s of 2958 s per suite (0.7 percent). The share would have to be wrong by a factor of
  more than 10 to matter, and the land step (44 of 72) is mostly writes, merges and worktree acts.
- Proof a later amendment needs: a differential test against real git over the same repositories, and content
  hashes byte-identical to today's, because they sit in the hash-chained log and in receipts. Any difference
  breaks old receipts and the meaning of the chain.
- Self-hosting risk: a regression in the git layer breaks the engine that builds itself. A later amendment
  would keep the old path behind a policy switch, off by default until the differential proof holds.
- Crates: `gix` (MIT or Apache-2.0, fits `NOTICE.md`) for hashes and ref reads; `git2` (MIT or Apache-2.0, but
  libgit2 carries GPLv2 with a linking exception, so it fits only with a note). `sha2` is already a dependency,
  and plain blob content hashes need no git library.

Recommendation for the variant: no-go. The saving is under 1 percent of suite CPU at best, for a risk to the
hash chain and to self-hosting.

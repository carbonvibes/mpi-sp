# Coverage Improvement Plan

## Current State

- **AFL++ edges covered:** ~1700 / 11072 (~15%)
- **Plateau:** edge count has stopped growing despite long campaigns
- **Binary:** `nm1sr5r2gzckh90y68avwa6fzp8hq83i-crun-harness-1.23.1` (non-ASAN, production)

The 11072 edges are AFL++ control-flow transitions (basic block pairs), not source lines.
15% after plateau means the current grammar + rootfs mutations are structurally incapable of
reaching the remaining 85% — random mutation alone won't get there.

**Rule: diagnose before fixing.** Never guess what's unreached. Build the coverage tool,
look at the report, then target the specific gaps.

---

## Phase 1 — LLVM Source Coverage Analysis

### Why a separate build

The AFL++ binary uses `afl-clang-lto` instrumentation to count edge transitions (the 11072
number). LLVM source coverage uses `-fprofile-instr-generate -fcoverage-mapping` to count
which source lines/functions/branches were executed. These are different instrumentation
schemes and produce different metrics — they are not meant to match.

The coverage build is a **read-only diagnostic tool**. You run your saved corpus through it,
get an HTML report showing which lines in `src/libcrun/*.c` were never touched, then go fix
the grammar/harness to hit those lines. You never fuzz with it.

Why build it through Nix rather than manually: the AFL++ Nix build uses specific optimization
flags, link-time optimizations, and the same library versions. A hand-built binary outside
Nix might inline functions differently, making the coverage report misleading. Same toolchain
= representative results.

### Step 1.1 — Create the crun-harness-cov Nix package

Create `SemanticSanitizer/nix/packages/by-name/artifact-eval/crun-harness-cov/package.nix`:

```nix
{ callPackage, clang, ... }:
let
  base = callPackage ../crun-harness/package.nix { };
in
base.overrideAttrs (old: {
  pname = "crun-harness-cov";

  # Replace afl-clang-lto with plain clang for coverage.
  # AFL++ instrumentation and -fprofile-instr-generate conflict — use clang directly.
  # Keep the same optimization level as the production build for representative results.
  preConfigure = ''
    export CC=${clang}/bin/clang
    export CXX=${clang}/bin/clang++
  '';

  env = (old.env or { }) // {
    NIX_CFLAGS_COMPILE = "-fprofile-instr-generate -fcoverage-mapping -g -O1";
    NIX_LDFLAGS = "-fprofile-instr-generate -lcriu";
  };
})
```

### Step 1.2 — Build it

```bash
cd /home/arjun/mpi-sp/SemanticSanitizer
git add nix/packages/by-name/artifact-eval/crun-harness-cov/
/nix/var/nix/profiles/default/bin/nix build .#"artifact-eval.crun-harness-cov"
COV_CRUN=$(readlink -f result/bin/crun)
echo $COV_CRUN
```

### Step 1.3 — Collect the corpus

The corpus from all 6 instances shares entries (LibAFL saves them to disk). Collect the union:

```bash
mkdir -p /tmp/cov_corpus
for i in 0 1 2 3 4 5; do
    cp /tmp/c3_$i/corpus/combined_* /tmp/cov_corpus/ 2>/dev/null || true
done
# Deduplicate by content hash
cd /tmp/cov_corpus
for f in *; do md5sum "$f"; done | sort | awk '!seen[$1]++{print $2}' > /tmp/unique_inputs.txt
echo "Unique corpus entries: $(wc -l < /tmp/unique_inputs.txt)"
```

### Step 1.4 — Write the replay harness

The coverage binary is the same crun harness (persistent mode stripped out for replay — or
just run it normally once per input). Write a shell script that runs each corpus entry:

```bash
cat > /tmp/run_coverage.sh << 'EOF'
#!/usr/bin/env bash
set -e

COV_CRUN=$1
CORPUS_DIR=$2
PROFDIR=/tmp/cov_profiles
REPLAY_DIR=/tmp/cov_replay

mkdir -p "$PROFDIR" "$REPLAY_DIR"
mkdir -p "$REPLAY_DIR/rootfs" "$REPLAY_DIR/rootfs/tmp" \
         "$REPLAY_DIR/rootfs/proc" "$REPLAY_DIR/rootfs/sys" \
         "$REPLAY_DIR/rootfs/dev"

i=0
while IFS= read -r input_file; do
    # Each input is a CombinedInput (msgpack). The harness reads config from argv[1].
    # We already have config.json written by the fuzzer for each corpus entry —
    # use the sidecar JSON to extract the config.
    sidecar="${input_file%.json}.json"
    config_json="/tmp/cov_corpus/${input_file%.combined_*}.json"

    LLVM_PROFILE_FILE="$PROFDIR/crun_%p_${i}.profraw" \
        timeout 5 "$COV_CRUN" "$config_json" 2>/dev/null || true

    i=$((i + 1))
    [[ $((i % 50)) -eq 0 ]] && echo "  replayed $i inputs..."
done < /tmp/unique_inputs.txt

echo "Done. Merging profiles..."
/nix/store/*/bin/llvm-profdata merge -sparse "$PROFDIR"/*.profraw \
    -o /tmp/cov_merged.profdata

echo "Generating HTML report..."
/nix/store/*/bin/llvm-cov show "$COV_CRUN" \
    -instr-profile=/tmp/cov_merged.profdata \
    -format=html \
    -output-dir=/tmp/cov_report \
    -show-line-counts-or-regions \
    -show-branches=count

echo "Report at: /tmp/cov_report/index.html"
EOF
chmod +x /tmp/run_coverage.sh
```

**Note:** the replay harness needs adjusting because CombinedInput is a binary format
(serde + msgpack). The actual config.json sidecar for each corpus entry is written by
`write_corpus_sidecar()` in `fuzz_combined_afl.rs` as `combined_N.json` in the corpus dir.
Use those directly:

```bash
# Simpler approach: use the existing sidecars
mkdir -p /tmp/cov_profiles
for dir in /tmp/c3_{0,1,2,3,4,5}/corpus; do
    for sidecar in "$dir"/combined_*.json; do
        [[ -f "$sidecar" ]] || continue
        config=$(python3 -c "import json,sys; d=json.load(open('$sidecar')); print(d['config'])" 2>/dev/null) || continue
        echo "$config" > /tmp/replay_config.json
        prof_file="/tmp/cov_profiles/crun_$(basename $sidecar .json).profraw"
        LLVM_PROFILE_FILE="$prof_file" timeout 5 "$COV_CRUN" /tmp/replay_config.json 2>/dev/null || true
    done
done
```

### Step 1.5 — Generate and read the report

```bash
# Merge all .profraw into one .profdata
llvm-profdata merge -sparse /tmp/cov_profiles/*.profraw \
    -o /tmp/cov_merged.profdata

# Summary by file
llvm-cov report "$COV_CRUN" \
    -instr-profile=/tmp/cov_merged.profdata \
    | sort -k4 -rn \
    | head -40

# Full HTML report (open in browser)
llvm-cov show "$COV_CRUN" \
    -instr-profile=/tmp/cov_merged.profdata \
    -format=html \
    -output-dir=/tmp/cov_report \
    -show-line-counts-or-regions

python3 -m http.server 8099 --directory /tmp/cov_report
# open http://localhost:8099
```

### Step 1.6 — What to look for in the report

Sort the `llvm-cov report` output by **missed lines** (column 4 descending). Files with the
most uncovered lines are the highest-value targets. Expected findings based on crun's
architecture:

| File | What it covers | Why likely uncovered |
|------|---------------|----------------------|
| `src/libcrun/seccomp.c` | seccomp filter setup | Grammar generates no `linux.seccomp` field |
| `src/libcrun/cgroup.c` | cgroup device/memory/cpu controllers | Grammar generates minimal or no `linux.resources` |
| `src/libcrun/linux.c` | namespace setup, pivot_root, capabilities | Some namespace combos never generated |
| `src/libcrun/hooks.c` | OCI lifecycle hooks | Grammar generates no `hooks` field |
| `src/libcrun/criu.c` | checkpoint/restore | Requires CRIU daemon, probably never reached |
| `src/libcrun/network.c` | network namespace config | Grammar generates no network namespace details |
| `src/libcrun/status.c` | state dir management | Partial — some error paths unreached |

---

## Phase 2 — Grammar Expansion (DO NOT START — depends on Phase 1 findings)

**Stop here until Phase 1 is complete and the coverage report has been read in full.**

The whole point of Phase 1 is to find out which of the three buckets the unreached code
falls into:

1. **Structurally unreachable with current grammar** — the OCI config never contains the
   field that triggers this code path. Fix: expand grammar. (Phase 2 applies here.)
2. **Reachable but requires specific combinations** — the grammar can generate the right
   fields but mutation hasn't found the right combination yet. Fix: corpus seeding or
   targeted seeds. (Phase 3 applies here.)
3. **Genuinely hard to reach** — CRIU checkpoint/restore, network namespace with external
   daemon, error paths that require specific kernel failures. Fix: may require harness
   changes or may not be worth pursuing at all.

Everything in Phase 2 and Phase 3 below is **an educated guess based on crun's known
architecture**. The coverage report may tell a completely different story. Read the report
first, then come back and decide which of the items below are actually relevant.

Once the coverage report confirms which code is unreached, expand the Nautilus grammar.
The grammar file is at:
`/nix/store/2hpav3yiv5fffrs9g3mf0lx21y7dxk41-crun-fuzzer-0.0.1/share/grammar.py`

**WARNING**: the grammar is baked into the Nix store. To modify it, edit the source in
`SemanticSanitizer/case-studies/oci/` and rebuild `crun-fuzzer`.

### High-priority grammar additions (to verify against coverage report first)

#### 2.1 — Seccomp

Currently: grammar likely generates no `linux.seccomp` at all.

Target code: `src/libcrun/seccomp.c` — filter construction, syscall table lookup, arch
handling.

Add to grammar:
```json
"linux": {
  "seccomp": {
    "defaultAction": "SCMP_ACT_ALLOW" | "SCMP_ACT_ERRNO" | "SCMP_ACT_KILL",
    "architectures": ["SCMP_ARCH_X86_64", "SCMP_ARCH_X86", "SCMP_ARCH_AARCH64"],
    "syscalls": [
      {
        "names": ["read", "write", "open", "close", "execve", ...],
        "action": "SCMP_ACT_ALLOW" | "SCMP_ACT_ERRNO" | "SCMP_ACT_KILL",
        "args": [
          { "index": 0, "value": 1, "op": "SCMP_CMP_EQ" }
        ]
      }
    ]
  }
}
```

Seccomp with specific syscall lists + arg filters exercises a LOT of code in
`seccomp.c` that is currently completely dead.

#### 2.2 — Linux resources (cgroup controllers)

Currently: grammar may generate `linux.resources.memory.limit` (we already clamp this
to 128MiB) but probably not the full resources tree.

Target code: `src/libcrun/cgroup.c`, `src/libcrun/cgroup2.c`.

Add:
```json
"linux": {
  "resources": {
    "memory":  { "limit": N, "swap": N, "swappiness": N, "disableOOMKiller": bool },
    "cpu":     { "shares": N, "quota": N, "period": N, "cpus": "0-3", "mems": "0" },
    "pids":    { "limit": N },
    "devices": [
      { "allow": bool, "type": "c"|"b"|"a", "major": N, "minor": N, "access": "rwm" }
    ],
    "blockIO": { "weight": N, "weightDevice": [...] }
  }
}
```

#### 2.3 — Hooks

Currently: grammar generates no hooks at all.

Target code: `src/libcrun/hooks.c`.

Add:
```json
"hooks": {
  "prestart":  [{ "path": "/bin/true", "args": [...], "env": [...], "timeout": N }],
  "poststart": [{ "path": "/bin/true", ... }],
  "poststop":  [{ "path": "/bin/true", ... }],
  "createRuntime": [...],
  "createContainer": [...],
  "startContainer": [...]
}
```

Note: hooks execute binaries. Since our rootfs has `/bin/true` populated, hook paths
pointing there will exercise the hook execution code without hanging.

#### 2.4 — Capabilities

Currently: grammar may not generate `linux.capabilities` at all, or only partially.

Target code: capability setup in `src/libcrun/linux.c`.

Add:
```json
"linux": {
  "capabilities": {
    "bounding":    ["CAP_CHOWN", "CAP_DAC_OVERRIDE", "CAP_FSETID", ...],
    "effective":   [...],
    "permitted":   [...],
    "inheritable": [...],
    "ambient":     [...]
  }
}
```

Vary which caps are present/absent in each set — the diff between bounding and effective
triggers specific code paths.

#### 2.5 — User namespace UID/GID mappings

Currently: grammar injects a user namespace (via our mount namespace injection) but
probably generates no `linux.uidMappings` or `linux.gidMappings`.

Target code: `src/libcrun/linux.c` user namespace setup.

Add:
```json
"linux": {
  "uidMappings": [{ "containerID": 0, "hostID": 1000, "size": 1 }],
  "gidMappings": [{ "containerID": 0, "hostID": 1000, "size": 1 }]
}
```

#### 2.6 — Specific mount types and options

Currently: grammar generates basic mounts (proc, tmpfs, sysfs) but probably not:
- `bind` mounts with specific options
- `overlay` mounts
- `devpts` with specific options
- `hugetlbfs`
- Mounts with `propagation` settings (shared/slave/private/unbindable)

Add mount entries targeting these types. Each exercises different code in
`src/libcrun/mounts.c`.

#### 2.7 — Process fields

Currently: grammar may not fully exercise:
- `process.rlimits` — each rlimit type exercises `setrlimit` path
- `process.noNewPrivileges` — affects prctl path
- `process.apparmorProfile` — AppArmor path (may be compiled out)
- `process.selinuxLabel` — SELinux path

---

## Phase 3 — Real Corpus Seeding (DO NOT START — depends on Phase 1 findings)

Real production OCI configs exercise field combinations that a grammar never generates
organically because real engineers discovered them through actual use.

### Step 3.1 — Extract from Docker images

```bash
# Pull a few common images and extract their OCI configs
for img in nginx:alpine redis:alpine alpine:latest busybox:latest python:3-alpine; do
    docker pull "$img"
    docker inspect "$img" > /tmp/docker_inspect_$(echo $img | tr '/:' '__').json
done

# Docker's config format is not exactly OCI — convert:
# The "ContainerConfig" and "Config" sections need to be mapped to OCI process fields.
# Use skopeo to get the actual OCI image manifest:
for img in nginx:alpine redis:alpine alpine:latest; do
    name=$(echo $img | tr '/:' '__')
    skopeo copy docker://$img oci:/tmp/oci_$name
    cat /tmp/oci_$name/blobs/sha256/* | python3 -m json.tool 2>/dev/null | head -100
done
```

### Step 3.2 — Convert to crun-compatible configs

Real OCI configs need `root.path` overridden to the FUSE mount and may need other
adjustments. Write a converter:

```python
import json, sys, os, glob

def adapt_config(cfg, fuse_rootfs):
    """Strip fields crun can't handle in our environment, fix root path."""
    cfg['root'] = {'path': fuse_rootfs, 'readonly': False}
    # Remove network namespace (we don't have a network setup)
    if 'linux' in cfg:
        ns = cfg['linux'].get('namespaces', [])
        cfg['linux']['namespaces'] = [n for n in ns if n.get('type') != 'network']
        # Ensure mount namespace present
        types = [n.get('type') for n in cfg['linux']['namespaces']]
        if 'mount' not in types:
            cfg['linux']['namespaces'].append({'type': 'mount'})
    return cfg
```

Adapted real configs go into the corpus directory as additional seeds before the
next campaign run. LibAFL will pick them up via `--resume` or as initial seeds.

---

## Phase 4 — Measurement and Iteration

After implementing changes, measure impact:

```bash
# Start a fresh campaign (not --resume) so the new grammar seeds the corpus from scratch
# Watch edge count for the first 2 hours

# Compare:
# Before:  1700 edges plateau
# Target:  3000+ edges after grammar expansion (rough estimate)
```

### What "success" looks like

- Coverage report shows seccomp.c going from 0% to >30% coverage
- Coverage report shows hooks.c going from 0% to >50% coverage
- AFL++ edge count breaks the 1700 plateau and climbs to 2500-3500
- New crash objectives found in previously unreachable code paths

---

## Implementation Order

Phase 1 is the only thing to do right now. Phases 2 and 3 cannot be planned until
Phase 1 is done and the report is read.

### Phase 1 — Do this now

| Step | Action | Effort |
|------|--------|--------|
| 1 | Create `crun-harness-cov` Nix package | 30min |
| 2 | Build it | 10min |
| 3 | Run corpus replay through it | 30min |
| 4 | Generate HTML report | 5min |
| 5 | Read the report — sort by missed lines, identify top unreached files | 1-2h |
| 6 | Classify each unreached block into bucket 1 / 2 / 3 (grammar gap / combination / hard) | 1h |

**Output of Phase 1:** a prioritised list of specific code regions that are unreached,
with a classification of WHY they're unreached. Only then does it make sense to decide
what to do next.

### Phase 2 and 3 — Decide after Phase 1

The items listed in Phase 2 (grammar expansion) and Phase 3 (corpus seeding) are
possibilities, not a plan. After reading the coverage report:

- If the unreached code is in bucket 1 (grammar gap) → Phase 2 is relevant
- If it is in bucket 2 (combination not found) → Phase 3 or targeted seeds are relevant
- If it is in bucket 3 (genuinely hard) → decide whether it's worth pursuing at all

---

## Phase 1 — Results (completed 2026-06-02)

### Coverage numbers

| Metric | Value |
|---|---|
| Corpus entries | 1561 across 6 instances |
| Profiles collected | 1493 .profraw files |
| Line coverage | 17.41% (3642 / 20918 lines) |
| Function coverage | 30.38% (243 / 800 functions) |
| Branch coverage | 11.52% (1249 / 10839 branches) |
| AFL++ edges (for reference) | ~1688 / 11072 (15.24%) |

The AFL++ edge % and LLVM line % being close is expected — they measure the same
reachability from different angles.

### Critical insight — fork/exec blindspot

LLVM coverage is **structurally blind to child-process code**. crun's execution model:

1. Parent parses config, creates state dir
2. Parent forks a child
3. Child sets up namespaces, mounts, capabilities, seccomp, cgroups
4. Child **exec()s** the container entrypoint (e.g. `/bin/true`)

When `exec()` replaces the child's process image, the LLVM profiling runtime's atexit
flush is destroyed. All coverage from the child is lost. This means functions like
`libcrun_set_mounts`, `do_mount`, `libcrun_init_caps`, `init_container`, `libcrun_apply_seccomp`
show 0% in LLVM even though they run on every corpus entry.

**AFL++ is unaffected** — it uses shared memory that persists across fork, so child-side
edges ARE counted in the 1688 number.

Consequence: the LLVM report is reliable for identifying **parent-side grammar gaps**
(seccomp filter building, hooks dispatch, cgroup resource config, sysctl validation) but
understates coverage for **child-side code** (mount setup, namespace setup, capability
application). Do not interpret child-side 0% as "never executed" — interpret it as
"executed but unmeasurable by LLVM".

Is fixing this worth it? Technically fixable via LD_PRELOAD wrapper that intercepts
exec() and calls `__llvm_profile_write_file()` before delegating. Not worth it — the
grammar gap analysis is complete and actionable from the parent-side data alone.

### Per-file breakdown

| File | Lines | Missed | Cover% | Classification |
|---|---|---|---|---|
| `linux.c` | 4259 | 3607 | 15.31% | Partial — parent-side: caps/sysctl/hostname gaps; child-side: mount/namespace (exec blindspot) |
| `container.c` | 3396 | 2747 | 19.11% | Partial — hooks, exec-process, seccomp-receiver all 0% (grammar gaps) |
| `cgroup-systemd.c` | 1587 | 1587 | 0% | Hard — requires systemd as PID 1. Skip. |
| `utils.c` | 1934 | 1549 | 19.91% | Partial — called from many paths, improves as grammar expands |
| `cgroup-resources.c` | 1086 | 849 | 21.82% | Grammar gap — no cpu/pids/devices/blockIO in grammar |
| `criu.c` | 734 | 734 | 0% | Hard — requires CRIU daemon. Skip. |
| `cgroup-utils.c` | 762 | 506 | 33.60% | Partial — improves with resources grammar additions |
| `seccomp.c` | 595 | 461 | 22.52% | Grammar gap — no `linux.seccomp.syscalls[]` in grammar |
| `ebpf.c` | 356 | 356 | 0% | Grammar gap — needs seccomp with BPF programs |
| `cgroup-setup.c` | 359 | 325 | 9.47% | Grammar gap — minimal cgroup config generated |
| `net_device.c` | 298 | 298 | 0% | Hard — needs real network namespace with external config. Skip. |
| `error.c` | 351 | 295 | 15.95% | Error paths — improves indirectly |
| `signals.c` | 270 | 270 | 0% | Grammar gap — no `process.stopSignal` in grammar |
| `mount_flags.c` | 255 | 255 | 0% | Grammar gap — no mount propagation flags in grammar |
| `cloned_binary.c` | 268 | 247 | 7.84% | Child-side exec blindspot — likely executed |
| `status.c` | 570 | 246 | 56.84% | Reasonably covered |
| `intelrdt.c` | 218 | 218 | 0% | Hard — Intel RDT hardware feature. Skip. |
| `seccomp_notify.c` | 135 | 131 | 2.96% | Grammar gap — no `SCMP_ACT_NOTIFY` action |
| `custom-handler.c` | 155 | 122 | 21.29% | Grammar gap — hooks trigger this |
| `chroot_realpath.c` | 98 | 98 | 0% | Internal rootfs path resolution, hard to target |
| CLI files (`exec.c`, `kill.c`, `ps.c`, etc.) | ~900 total | ~900 | 0% | Structurally unreachable — harness only calls `libcrun_container_run()`. Permanent skip. |

### Key zero-coverage functions (parent-side, confirmed grammar gaps)

**seccomp.c — entire file is parent-side, 0%:**
- `libcrun_apply_seccomp`, `get_seccomp_action`, `get_seccomp_operator`
- `store_seccomp_cache`, `evict_cache`, `seccomp_action_supports_errno`

**container.c — hook dispatch is parent-side, 0%:**
- `container.c:do_hooks`, `open_hooks_output`, `handle_notify_socket`
- `container.c:get_seccomp_receiver_fd_payload` (seccomp notify receiver)

**linux.c — parent-side gaps (not child-exec blindspot):**
- `libcrun_set_sysctl`, `validate_sysctl` — no `linux.sysctl` in grammar
- `libcrun_set_hostname`, `libcrun_set_domainname` — no hostname field in grammar
- `libcrun_configure_network` — no network device config in grammar

**cgroup-resources.c — parent-side cgroup resource writing, 0%:**
- `update_cgroup_v1_resources`, `write_blkio_resources`
- `write_devices_resources`, `write_network_resources`, `write_hugetlb_resources`
- `write_devices_resources_v1`, `write_devices_resources_v2`

### Bucket classification

**Bucket 1 — Grammar gaps (grammar expansion will fix):**
`seccomp.c`, `ebpf.c`, `seccomp_notify.c`, `cgroup-resources.c`, `signals.c`,
`mount_flags.c`, `custom-handler.c` (via hooks), hook code in `container.c`,
sysctl/hostname/caps/rlimits paths in `linux.c`, `cgroup-setup.c`

**Bucket 2 — Combination not found (grammar can express it, fuzzer hasn't found path):**
Most of the remaining unreached branches in `container.c`, `utils.c`, `linux.c`

**Bucket 3 — Genuinely hard / skip:**
`cgroup-systemd.c`, `criu.c`, `intelrdt.c`, `net_device.c`, all CLI subcommand files

---

## Phase 2 — Grammar Expansion (START NOW)

Phase 1 is done. The report confirmed the grammar gaps. Implement in priority order.

### Where the grammar lives

The grammar is a Python file baked into the Nix store. To modify it:
1. Find the source in `SemanticSanitizer/case-studies/oci/` — look for `grammar.py`
2. Edit it there
3. Rebuild: `nix build .#"artifact-eval.crun-fuzzer"` from `SemanticSanitizer/`
4. Update `GRAMMAR` path in `launch_campaigns.sh` and `run_coverage.sh` to the new store path

### 2.1 — Seccomp (HIGHEST PRIORITY)

**Why first:** `seccomp.c` is entirely parent-side, entirely 0%, and seccomp filter
parsing is historically a rich bug source. Unlocks seccomp.c (595 lines), ebpf.c (356),
seccomp_notify.c (131) = 1082 lines of new code reached.

Add to grammar (make each field independently optional so mutation varies):
```python
# defaultAction variants
SECCOMP_DEFAULT_ACTIONS = [
    "SCMP_ACT_ALLOW", "SCMP_ACT_ERRNO", "SCMP_ACT_KILL",
    "SCMP_ACT_KILL_PROCESS", "SCMP_ACT_LOG", "SCMP_ACT_TRACE",
]

# Per-syscall action variants (SCMP_ACT_NOTIFY triggers seccomp_notify.c)
SECCOMP_SYSCALL_ACTIONS = [
    "SCMP_ACT_ALLOW", "SCMP_ACT_ERRNO", "SCMP_ACT_KILL",
    "SCMP_ACT_NOTIFY", "SCMP_ACT_LOG",
]

# Comparison operators for arg filters
SECCOMP_OPS = [
    "SCMP_CMP_NE", "SCMP_CMP_LT", "SCMP_CMP_LE",
    "SCMP_CMP_EQ", "SCMP_CMP_GE", "SCMP_CMP_GT", "SCMP_CMP_MASKED_EQ",
]

# Example shape:
{
    "linux": {
        "seccomp": {
            "defaultAction": "<SECCOMP_DEFAULT_ACTIONS>",
            "architectures": ["SCMP_ARCH_X86_64"],
            "syscalls": [
                {
                    "names": ["read", "write", "mmap", "open", "close",
                              "execve", "exit_group", "brk", "mprotect"],
                    "action": "<SECCOMP_SYSCALL_ACTIONS>",
                    "args": [
                        {"index": 0, "value": 1, "op": "<SECCOMP_OPS>"}
                    ]
                }
            ]
        }
    }
}
```

### 2.2 — Hooks

**Why:** `do_hooks` and `open_hooks_output` in container.c are parent-side and 0%.
Hooks also trigger `custom-handler.c`. The rootfs already has `/bin/true` so hooks
pointing there will execute safely without hanging.

```python
HOOK_TYPES = [
    "prestart", "createRuntime", "createContainer",
    "startContainer", "poststart", "poststop"
]

# Each hook entry shape:
{
    "path": "/bin/true",
    "args": ["/bin/true", "<optional_arg>"],
    "env": ["FOO=bar"],
    "timeout": 5
}

# Grammar should optionally include any subset of hook types,
# each with 1-3 hook entries.
```

### 2.3 — Linux resources (cgroup controllers)

**Why:** `cgroup-resources.c` is 21.82% covered but 17/31 functions are 0%.
`write_blkio_resources`, `write_devices_resources`, `write_network_resources`,
`write_hugetlb_resources`, `update_cgroup_v1_resources` all never called.

```python
# Make each sub-field independently optional:
{
    "linux": {
        "resources": {
            "memory": {
                "limit": <int_128mb>,
                "swap": <int>,
                "swappiness": <0-100>,
                "disableOOMKiller": <bool>
            },
            "cpu": {
                "shares": <int>,
                "quota": <int>,
                "period": 100000,
                "cpus": "0",
                "mems": "0"
            },
            "pids": {"limit": <int>},
            "devices": [
                {
                    "allow": <bool>,
                    "type": "c" | "b" | "a",
                    "major": <int>,
                    "minor": <int>,
                    "access": "rwm" | "rw" | "r"
                }
            ],
            "blockIO": {
                "weight": <100-1000>,
                "leafWeight": <int>
            },
            "hugepageLimits": [
                {"pageSize": "2MB", "limit": <int>}
            ]
        }
    }
}
```

### 2.4 — Process capabilities

**Why:** `populate_capabilities` in container.c is 0% (parent-side). Capability
manipulation is a historically common privilege-escalation surface.

```python
CAPS = [
    "CAP_CHOWN", "CAP_DAC_OVERRIDE", "CAP_DAC_READ_SEARCH",
    "CAP_FSETID", "CAP_FOWNER", "CAP_MKNOD", "CAP_NET_RAW",
    "CAP_SETGID", "CAP_SETUID", "CAP_SETFCAP", "CAP_SETPCAP",
    "CAP_NET_BIND_SERVICE", "CAP_SYS_CHROOT", "CAP_KILL",
    "CAP_AUDIT_WRITE", "CAP_SYS_PTRACE", "CAP_NET_ADMIN",
]

# Grammar should generate all 5 sets with varying subsets of CAPS:
{
    "process": {
        "capabilities": {
            "bounding":    <subset of CAPS>,
            "effective":   <subset of CAPS>,
            "permitted":   <subset of CAPS>,
            "inheritable": <subset of CAPS>,
            "ambient":     <subset of CAPS>
        }
    }
}
```

The diff between bounding and effective triggers distinct paths in `set_required_caps`
and `libcrun_init_caps`.

### 2.5 — Process rlimits and noNewPrivileges

**Why:** `get_rlimit_resource` in linux.c is 0% (parent-side validates rlimit types).
`noNewPrivileges` triggers a distinct prctl path.

```python
RLIMIT_TYPES = [
    "RLIMIT_NOFILE", "RLIMIT_NPROC", "RLIMIT_CORE", "RLIMIT_CPU",
    "RLIMIT_DATA", "RLIMIT_FSIZE", "RLIMIT_MEMLOCK", "RLIMIT_MSGQUEUE",
    "RLIMIT_NICE", "RLIMIT_RSS", "RLIMIT_RTPRIO", "RLIMIT_SIGPENDING",
    "RLIMIT_STACK",
]

{
    "process": {
        "noNewPrivileges": True,
        "rlimits": [
            {"type": "<RLIMIT_TYPE>", "hard": <int>, "soft": <int>}
        ]
    }
}
```

### 2.6 — Sysctl

**Why:** `libcrun_set_sysctl` and `validate_sysctl` in linux.c are 0% (parent-side).
One field addition, unlocks 74 regions.

```python
SYSCTL_KEYS = [
    "net.ipv4.ip_forward",
    "net.ipv4.conf.all.forwarding",
    "kernel.shm_rmid_forced",
    "kernel.msgmax",
    "kernel.shmmax",
]

{
    "linux": {
        "sysctl": {
            "<SYSCTL_KEY>": "<string_value>"
        }
    }
}
```

### 2.7 — Hostname and domainname

**Why:** `libcrun_set_hostname` and `libcrun_set_domainname` are 0%. These run in
the child but the config field is parsed in the parent. Simple single-field addition.

```python
{
    "hostname": "<random_string>",
    "domainname": "<random_string>"
}
```

### 2.8 — Mount propagation flags

**Why:** `mount_flags.c` is 0% (255 lines). It's a flag lookup table hit when mounts
specify propagation options. Grammar currently generates mounts without propagation.

```python
PROPAGATION_TYPES = ["shared", "slave", "private", "unbindable"]

# Add to mount entries:
{
    "mounts": [
        {
            "destination": "/tmp",
            "type": "tmpfs",
            "source": "tmpfs",
            "options": ["rprivate", "nosuid", "nodev"]
        },
        {
            "destination": "/proc",
            "type": "proc",
            "source": "proc",
            "options": ["<PROPAGATION_TYPE>"]
        }
    ]
}
```

### 2.9 — User namespace UID/GID mappings

**Why:** `libcrun_container_setgroups`, `uidgidmap_helper`, `can_setgroups`,
`deny_setgroups` in linux.c are all 0%. User namespace mapping code is a classic
privilege-confusion surface.

```python
{
    "linux": {
        "namespaces": [{"type": "user"}],
        "uidMappings": [{"containerID": 0, "hostID": 1000, "size": 1}],
        "gidMappings": [{"containerID": 0, "hostID": 1000, "size": 1}]
    }
}
```

### 2.10 — process.stopSignal

**Why:** `signals.c` is 0% (270 lines). It is used when killing a container.
The stop signal is a single string field.

```python
SIGNALS = [
    "SIGTERM", "SIGKILL", "SIGHUP", "SIGINT",
    "SIGUSR1", "SIGUSR2", "SIGQUIT",
]

{
    "process": {
        "stopSignal": "<SIGNAL>"
    }
}
```

---

### What needs rebuilding after grammar changes

1. Edit grammar source in `SemanticSanitizer/case-studies/oci/`
2. Rebuild crun-fuzzer: `nix build .#"artifact-eval.crun-fuzzer"` from `SemanticSanitizer/`
3. Get new grammar path: `readlink -f result/share/grammar.py`
4. Update `GRAMMAR=` in `launch_campaigns.sh` and `run_coverage.sh`
5. **Start fresh campaigns** (not --resume) — new grammar generates new initial seeds
6. After 2-4 hours, run `sudo bash run_coverage.sh` again to measure improvement

---

### Expected impact after Phase 2

| File | Before | Expected after |
|---|---|---|
| `seccomp.c` | 22% | 70-80% (parent-side filter building fully exercisable) |
| `cgroup-resources.c` | 22% | 50-65% |
| `container.c` | 19% | 35-45% (hooks path is large) |
| `signals.c` | 0% | 40-60% |
| `mount_flags.c` | 0% | 60-80% |
| `linux.c` | 15% | 20-25% (sysctl/hostname/caps parent-side; child-side still exec-blind) |
| AFL++ edges | ~1688 | 3000-4500 estimated |

New crash surface opened: seccomp filter parsing, capability manipulation,
hook external process execution, cgroup resource limit application.

---

## Phase 2 — Actual Results (measured 2026-06-03)

### What actually happened vs expectations

| File | Expected | Actual | Notes |
|---|---|---|---|
| `seccomp.c` | 70-80% | 24.87% | Filter BUILDING still mostly missed — see below |
| `ebpf.c` | — | **57.30%** (was 0%) | Surprise win from RESOURCES_DEVICES |
| `cgroup-resources.c` | 50-65% | 34.62% (was 22%) | +12pp, cgroup v1 paths unreachable on v2 system |
| `container.c` | 35-45% | 23.88% (was 19%) | +4.77pp, hooks partial |
| `signals.c` | 40-60% | 0% (unchanged) | stopSignal only fires on kill, harness never kills |
| `mount_flags.c` | 60-80% | 0% (unchanged) | Child-side exec blindspot |
| `linux.c` | 20-25% | 18.69% (was 15%) | +3.38pp |
| AFL++ edges | 3000-4500 | TBD (campaigns running) | |

Overall: lines 17.41% → **21.05%**, functions 30.38% → **35.12%**, branches 11.52% → **14.31%**

### Why seccomp.c barely moved

`get_seccomp_action`, `get_seccomp_operator`, `make_lowercase` are still 0%. These parse
per-syscall action strings and are called during filter building in the parent. The problem:
configs with restrictive `defaultAction` (ERRNO/KILL) block execve — container dies, entry
goes to crashes not corpus. The corpus (successful runs) is dominated by default-ALLOW configs
with no per-syscall rules, so the action parsing code never runs.

### cgroup-utils.c regression (33.6% → 24.3%)

Pure corpus restart effect — campaigns restarted fresh with new grammar. Old paths not yet
rediscovered. Expected to self-correct after 12-24h of new campaign running.

### Custom-handler.c still at 21%

`custom-handler.c` is crun's module/wasm custom runtime delegation system, NOT standard OCI
lifecycle hooks. OCI hooks (our grammar addition) go through `do_hooks` in container.c.
`custom-handler.c` requires annotations like `run.oci.handler: wasm` — different code path entirely.

---

## Phase 2b — Grammar Patch 2 (2026-06-03)

### Changes made in this patch

**New grammar path:** `/nix/store/p3bm9g848p95kr4avw01ags8gf1rp8nk-crun-fuzzer-0.0.1/share/grammar.py`

| Change | Target | Rationale |
|---|---|---|
| 4 safe-ALLOW seccomp static rules | `seccomp.c` `get_seccomp_action` | defaultAction=ALLOW + syscall rules → container runs → entry goes to corpus → action parsing code runs |
| `run.oci.handler`, `module.wasm.image/variant` annotations | `custom-handler.c` | These annotations trigger wasm/custom runtime delegation, not OCI hooks |
| 3 more LINUX variants with `cgroupsPath` | `cgroup-setup.c` | cgroupsPath exercises different cgroup path setup code |
| 2 more `cgroupsPath` values | `cgroup-setup.c` | More path diversity |

### Structurally unreachable — stop pursuing

| File | Reason | Decision |
|---|---|---|
| `cgroup-systemd.c` (0%) | systemd as PID 1 required | Skip |
| `criu.c` (0%) | CRIU daemon required | Skip |
| `signals.c` (0%) | stopSignal only fires on container kill — harness runs to completion | Skip |
| `mount_flags.c` (0%) | Child-side exec blindspot; AFL++ covers it | Skip |
| `intelrdt.c` (0%) | Hardware-specific | Skip |
| cgroup v1 paths | System runs cgroup v2 | Skip |
| CLI subcommands | Harness only calls `libcrun_container_run` | Skip |

### Next measurement

Run `sudo bash run_coverage.sh` after 24h with new grammar (patch 2) to see:
- Did `get_seccomp_action` finally show coverage?
- Did `custom-handler.c` move from 21%?
- Did `cgroup-utils.c` recover from regression?
- Did `cgroup-setup.c` improve above 9.47%?

---

## Phase 3 — Real Corpus Seeding (next priority)

Now that grammar gaps are largely addressed, real production OCI configs are the
highest remaining ROI. Real configs exercise field combinations that a grammar never
generates organically.

### What to seed

Extract OCI configs from real container images and adapt them for our harness:

```bash
# Pull images and extract configs
for img in nginx:alpine redis:alpine alpine:latest busybox:latest; do
    name=$(echo $img | tr '/:' '__')
    skopeo copy docker://$img oci:/tmp/oci_$name
done

# Each OCI image has a config blob — extract and adapt:
# 1. Set root.path to the FUSE mount
# 2. Ensure mount namespace present in linux.namespaces
# 3. Remove network namespace (no network setup in harness)
# 4. Convert to CombinedInput format and add to corpus
```

Real configs from nginx, redis, alpine exercise:
- Specific capability sets that production containers actually use
- Real seccomp profiles (docker default profile has 300+ syscall rules)
- Real mount lists (devpts, mqueue, etc.)
- Process user/group combinations that hit uid mapping paths

This is Phase 3 from the original plan — not started yet, depends on Phase 2b results.

# Building & Running the Combined crun Fuzzer

This is the build/run guide for the **combined fuzzer** (`fuzz_combined_afl`, a.k.a.
Campaign 3 / OrcFuzz). It is a coverage-guided fuzzer that mutates **both** halves
of an OCI container input at once:

- the **`config.json`** — via a Nautilus grammar, and
- the **rootfs** — via an in-process **FUSE** virtual filesystem (mutated per input
  by an `FsDelta`),

and runs them against an **AFL-instrumented `crun`** through the AFL forkserver. The
launcher runs **6 instances** pinned to cores 0–5, all sharing one corpus.

> Paths below assume the repo lives at `/home/arjun/mpi-sp`. Adjust if you cloned
> it elsewhere (the launcher uses absolute paths).

---

## 1. Prerequisites

### Toolchains

| Tool | Why | Install |
|------|-----|---------|
| **Rust** (stable, tested on 1.95) | builds the fuzzer (`mutator/`) | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| **Nix** (flakes enabled; Determinate Nix recommended) | builds the AFL-instrumented `crun` harness | `curl -fsSL https://install.determinate.systems/nix \| sh -s -- install` |

### System packages (Debian/Ubuntu)

The Rust crate's `build.rs` compiles C glue (with **clang** for SanCov, **gcc** for
the control plane) and links **FUSE** and **Python**:

```bash
sudo apt update
sudo apt install -y \
    build-essential clang pkg-config \
    libfuse3-dev fuse3 \
    python3 python3-dev \
    util-linux

# Optional — only for the separate libarchive demo campaign, NOT the combined fuzzer:
sudo apt install -y libarchive-dev
```

What each is for:

- **build-essential** — `gcc` + `make`; `build.rs` runs `make` in `control_plane/` (compiled with gcc).
- **clang** — `build.rs` compiles the FUSE/SanCov C glue with clang (GCC lacks `-fsanitize-coverage=trace-pc-guard`).
- **pkg-config** — locates `fuse3` at build time.
- **libfuse3-dev** + **fuse3** — the in-process FUSE rootfs (**required** — without it the combined fuzzer is compiled out) and the `fusermount3` helper.
- **python3** + **python3-dev** — `pyo3` embeds Python to execute the Nautilus grammar.
- **util-linux** — provides `unshare` (each instance runs in its own mount namespace). Usually preinstalled.

> **Note:** you do **not** need to apt-install crun's own build deps (criu, libcap,
> libseccomp, yajl, systemd, …) — the crun harness is built by **Nix**, which
> supplies them in its sandbox (step 2).

### Runtime requirements

- **Root / sudo** — the fuzzer needs it for `unshare -m`, the FUSE mount, and `crun`.
  The launcher invokes `sudo -E unshare -m` per instance; passwordless sudo avoids
  6 prompts.
- **`/dev/fuse`** present and the FUSE kernel module loaded (`modprobe fuse` if not).
- **Free CPU cores** — the default config pins 6 instances to cores 0–5.

---

## 2. Build the crun harness (the fuzz target)

The fuzzer execs an AFL-instrumented `crun` via the forkserver. Build it from the
SemanticSanitizer flake:

```bash
cd /home/arjun/mpi-sp/SemanticSanitizer
nix build .#artifact-eval.crun-harness

# the binary is now under the result symlink:
readlink -f result          # -> /nix/store/<hash>-crun-harness-1.23.1
ls -l result/bin/crun
```

Optional — the **ASAN** variant (slower, but catches memory-safety bugs; used for
crash triage, not the main campaign):

```bash
nix build .#artifact-eval.crun-harness-asan -o result-asan
ls -l result-asan/bin/crun
```

---

## 3. Build the fuzzer

```bash
cd /home/arjun/mpi-sp/mutator
cargo build --release --bin fuzz_combined_afl
# -> target/release/fuzz_combined_afl
```

`build.rs` automatically builds `control_plane/` (via `make`) and the FUSE glue.
Two warnings — `libarchive/libcrun: using SanCov-instrumented static build` — are
**expected and benign**. (The checked-in `mutator/static/true`, a static `exit(0)`
stub used as every binary inside the FUSE rootfs, and the in-repo grammar are
already in the tree — nothing to fetch.)

---

## 4. Point the launcher at your crun

Edit [`launch_campaigns.sh`](launch_campaigns.sh) and set **`CRUN=`** to the harness
you just built. The Nix store hash changes on every rebuild, so the simplest robust
form is:

```bash
CRUN="$(readlink -f /home/arjun/mpi-sp/SemanticSanitizer/result)/bin/crun"
```

The other vars are already correct for this checkout:
- `FUZZ_BIN` → `mutator/target/release/fuzz_combined_afl`
- `GRAMMAR`  → the in-repo edited grammar (`SemanticSanitizer/case-studies/oci/grammar.py`)
- `CORES` / `CAMPAIGN_DIRS` → 6 instances on cores 0–5 (reduce if you have fewer free cores)

---

## 5. Run

```bash
cd /home/arjun/mpi-sp
bash launch_campaigns.sh
```

This:
- launches 6 instances on cores 0–5, all sharing the corpus at `/tmp/c3_shared`,
- creates the real bind-mount source `/tmp/fuzz_bind_src` (so bind/idmapped mounts resolve),
- tees each instance's output to `/tmp/c3_<i>_fuzz.log`.

You'll be asked for sudo (each instance runs under `sudo -E unshare -m`). The
`WARNING: core N already used by … kworker/ksoftirqd …` messages are **expected** —
those are per-CPU kernel threads, harmless; the launcher continues.

---

## 6. Watch progress

```bash
python3 /home/arjun/mpi-sp/web_campaign/server.py
# then open http://localhost:8090
```

Per instance you'll see: **edges** (`X / 11072` = % of crun's instrumented edges hit),
**exec/s**, **corpus** size, and **crashes**.

---

## 7. Stop

Press **Ctrl-C** in the launcher terminal. It kills all instances, writes a final
combined plot (`parallel_<timestamp>.png`), and cleans up the `/tmp` campaign dirs,
the shared corpus, and the bind source.

---

## Optional / related

### Crash triage under ASAN
Once crashes appear, decide genuine-vs-spurious by replaying each under ASAN crun:
```bash
sudo bash /home/arjun/mpi-sp/asan_triage.sh
```
It re-renders every saved crash through the same FUSE pipeline against the ASAN
build and prints a summary (ASAN-BUG / CRASH-NO-ASAN / NO-CRASH). The script's
`GRAMMAR` must match the campaign's grammar (already set to the in-repo one).

### In-process crun fuzzer (`fuzz_crun`) — a different harness
Not needed for the combined fuzzer. It links `crun` in-process and needs a
SanCov-instrumented static build (already present at `vendor/crun/.libs/libcrun.a`).
To rebuild it:
```bash
cd vendor/crun && ./autogen.sh && \
  CC=clang CFLAGS="-fsanitize-coverage=trace-pc-guard,trace-cmp -O1 -g" \
  ./configure --disable-shared --enable-static && make -j"$(nproc)"
```

---

## Troubleshooting

| Symptom | Cause / fix |
|---|---|
| `text file busy` when running `cargo build` | The campaign is still running the binary (it's mmap'd). Stop it first, then rebuild. |
| Build says `libfuse3-dev not found — FUSE harness loop disabled` | `apt install libfuse3-dev` — without it the combined fuzzer is compiled out. |
| `crun` path errors / forkserver won't start after a rebuild | The Nix store hash changed; re-point `CRUN=` via `readlink -f result`. |
| FUSE mount times out | `/dev/fuse` missing or fuse module not loaded (`modprobe fuse`); ensure you're root. |
| `decode_crash` "unused import" warning | Pre-existing and harmless, unrelated to the fuzzer. |

> ⚠️ The fuzzer runs `crun` as **root** in a mount namespace by design. Review
> `launch_campaigns.sh` before running on a shared machine.

#!/usr/bin/env bash

# ── Fleet layout ─────────────────────────────────────────────────────────────
#   inst 0-5  : baseline (no sanitizer), share one corpus (read+write)
#   inst 6    : ASAN    — own corpus, READS the shared corpus, never exports
#   inst 7    : UBSan   — own corpus, READS the shared corpus, never exports
#   + SemSan  : one eBPF monitor watching every `crun` system-wide (all 8)
# The ASAN/UBSan instances vet the base fleet's corpus under a sanitizer without
# polluting it (--no-export), so a sanitizer find is a real bug, not coverage noise.

CRUN_BASE=/nix/store/nm1sr5r2gzckh90y68avwa6fzp8hq83i-crun-harness-1.23.1/bin/crun        # no sanitizer
CRUN_ASAN=/nix/store/4w5j3vmpd4rl71c0vxzkl5mwq4mqjnz7-crun-harness-asan-1.23.1/bin/crun    # ASAN, no false positives
CRUN_UBSAN=/nix/store/hfqz6i88ll99j6ymab1jcj70hz0pzm64-crun-harness-ubsan-1.23.1/bin/crun  # nix build .#artifact-eval.crun-harness-ubsan (result-crun-ubsan symlink kept as a GC root)

FUZZ_BIN=/home/arjun/mpi-sp/mutator/target/release/fuzz_combined_afl
GRAMMAR=/home/arjun/mpi-sp/SemanticSanitizer/case-studies/oci/grammar.py # Tier-1 fields + rebalance + golden + bind sources

SEMSAN_CLI=/home/arjun/mpi-sp/SemanticSanitizer/semsan-cli
SEMSAN_CONFIG=/home/arjun/mpi-sp/semsan_crun.yaml
SEMSAN_LOG=/tmp/semsan.log

# UBSan must ABORT on a violation, else it prints+continues and the forkserver
# never sees a crash. Tunable here without a rebuild (add suppressions=<file> if
# crun's common path turns out to emit benign UB that floods the instance).
UBSAN_OPTS="halt_on_error=1:abort_on_error=1:print_stacktrace=1"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Instance i -> campaign dir (CWD); index aligns with CORES.
CAMPAIGN_DIRS=(/tmp/c3_0 /tmp/c3_1 /tmp/c3_2 /tmp/c3_3 /tmp/c3_4 /tmp/c3_5 /tmp/c3_asan /tmp/c3_ubsan)
SYNC_DIR=/tmp/c3_shared      # base fleet's shared corpus (ASAN/UBSan read-only)
BIND_SRC=/tmp/fuzz_bind_src  # real host source so bind/idmapped mounts resolve
CORES=(0 1 2 3 4 5 6 7)
PIDS=()

echo "==> Checking CPU cores ${CORES[*]}..."
conflict=0
for core in "${CORES[@]}"; do
    mask=$(printf '%x' $((1 << core)))
    while IFS= read -r pid; do
        [[ "$pid" == "$$" ]] && continue
        comm=$(ps -p "$pid" -o comm= 2>/dev/null)
        user=$(ps -p "$pid" -o user= 2>/dev/null)
        echo "  WARNING: core $core already used by pid=$pid ($user: $comm)"
        conflict=1
    done < <(ls /proc 2>/dev/null | grep -E '^[0-9]+$' | while read -r p; do
        aff=$(taskset -p "$p" 2>/dev/null | awk '{print $NF}')
        [[ "$aff" == "$mask" ]] && echo "$p"
    done)
done
if [[ $conflict -eq 1 ]]; then
    echo "  (conflicts found — those cores are shared, not exclusively ours)"
else
    echo "  All clear — cores ${CORES[*]} are free."
fi

kill_our_fuzzers() {
    local dir pids pid
    for dir in "${CAMPAIGN_DIRS[@]}"; do
        pids=()
        for pid in $(ls /proc 2>/dev/null | grep -E '^[0-9]+$'); do
            cwd=$(readlink "/proc/$pid/cwd" 2>/dev/null)
            [[ "$cwd" == "$dir" ]] && pids+=("$pid")
        done
        if [[ ${#pids[@]} -gt 0 ]]; then
            echo "  Killing pids in $dir: ${pids[*]}"
            sudo kill -TERM "${pids[@]}" 2>/dev/null || true
        fi
    done
}

cleanup() {
    echo ""
    echo "==> Caught interrupt — stopping all fuzzers..."

    for pid in "${PIDS[@]}"; do
        kill -TERM "$pid" 2>/dev/null || true
    done

    # only kills processes whose cwd is one of our campaign dirs
    kill_our_fuzzers

    # stop the SemSan eBPF monitor (runs as root)
    sudo pkill -x semsan-cli 2>/dev/null || true

    echo "  Waiting for fuzzers to flush stats..."
    sleep 3
    wait 2>/dev/null || true

    # SIGKILL any crun strays. Hung container children CATCH SIGTERM (crun installs
    # signal handlers), so kill -TERM / plain pkill leave them sleeping forever —
    # that's what leaks thousands of orphaned crun across runs. Only SIGKILL reaps
    # them. Safe on a dedicated fuzzing host where every crun is ours.
    echo "  Reaping crun strays (SIGKILL)..."
    sudo pkill -9 -x crun 2>/dev/null || true

    # Unmount + remove any FUSE mountpoints the instances/strays left behind.
    mount 2>/dev/null | grep -oE '/tmp/campaign3-fuse-[0-9]+' \
        | sudo xargs -r -n1 fusermount3 -u 2>/dev/null || true
    sudo rm -rf /tmp/campaign3-fuse-* 2>/dev/null || true

    echo "==> Generating final plots..."
    OUT="$SCRIPT_DIR/parallel_$(date +%Y%m%d_%H%M%S).png"
    if python3 "$SCRIPT_DIR/web_campaign/plot_final.py" "${CAMPAIGN_DIRS[@]}" "$OUT"; then
        echo "==> Plot saved: $OUT"
    else
        echo "==> Plot skipped (no data yet)."
    fi

    echo "==> Cleaning up /tmp campaign directories..."
    for dir in "${CAMPAIGN_DIRS[@]}"; do
        sudo rm -rf "$dir"
        echo "    rm -rf $dir"
        rm -f "/tmp/$(basename "$dir")_fuzz.log"
    done
    sudo rm -rf "$SYNC_DIR" "$BIND_SRC"
    echo "    rm -rf $SYNC_DIR $BIND_SRC"
    rm -f "$SEMSAN_LOG"

    echo "==> Done."
    exit 0
}

trap cleanup INT TERM

# Prime sudo once up front so the backgrounded sudo's (instances + SemSan) don't
# each try to prompt on a tty they can't read.
sudo -v || { echo "sudo required"; exit 1; }

for dir in "${CAMPAIGN_DIRS[@]}"; do mkdir -p "$dir"; done
mkdir -p "$SYNC_DIR"

# Tier-3: a real host directory bind/idmapped mounts can resolve to, so the bind
# path (libcrun_container_do_bind_mount) actually runs instead of ENOENT-ing.
mkdir -p "$BIND_SRC/dir"
printf 'fuzz bind source file\n' > "$BIND_SRC/file.txt"
printf 'nested\n' > "$BIND_SRC/dir/nested.txt"
chmod -R a+rX "$BIND_SRC"

# ── SemSan monitor — one eBPF attach watches every `crun` (covers all 8) ──────
if [ -x "$SEMSAN_CLI" ]; then
    echo "==> Starting SemSan monitor (comm=crun) → $SEMSAN_LOG"
    ( sudo "$SEMSAN_CLI" attach --config "$SEMSAN_CONFIG" > "$SEMSAN_LOG" 2>&1 ) &
    PIDS+=($!)
    sleep 2
else
    echo "  WARNING: $SEMSAN_CLI not found — SemSan monitor NOT started"
fi

# ── Fuzzer instances ─────────────────────────────────────────────────────────
for i in "${!CAMPAIGN_DIRS[@]}"; do
    dir="${CAMPAIGN_DIRS[$i]}"
    pre=""          # optional env prefix
    extra=""        # extra fuzzer args
    case $i in
        6) bin="$CRUN_ASAN";  arm="ASAN";  extra="--no-export" ;;
        7) bin="$CRUN_UBSAN"; arm="UBSan"; extra="--no-export"; pre="env UBSAN_OPTIONS=$UBSAN_OPTS" ;;
        *) bin="$CRUN_BASE";  arm="base" ;;
    esac

    if [ ! -x "$bin" ]; then
        echo "  SKIP inst-$i [$arm]: crun binary not found ($bin) — build it, then relaunch"
        continue
    fi

    log="/tmp/$(basename "$dir")_fuzz.log"
    (cd "$dir" && RUST_BACKTRACE=full taskset -c "${CORES[$i]}" sudo -E $pre unshare -m \
        "$FUZZ_BIN" "$bin" "$GRAMMAR" \
        --sync-dir "$SYNC_DIR" --instance "$i" $extra \
        2>&1 | tee "$log") &
    PIDS+=($!)
    echo "C3-inst-$i [$arm] started on core ${CORES[$i]} (pid ${PIDS[-1]})  cwd=$dir"
done

echo ""
echo "Fleet launched — 6 base + ASAN + UBSan, single shared corpus; SemSan monitoring all crun."
echo "Dashboard : python3 $SCRIPT_DIR/web_campaign/server.py"
echo "SemSan log: tail -f $SEMSAN_LOG"
echo "Press Ctrl+C to stop, save plots, and clean up /tmp."
wait

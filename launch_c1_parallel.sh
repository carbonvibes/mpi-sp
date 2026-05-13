#!/usr/bin/env bash

FUZZER=/nix/store/2hpav3yiv5fffrs9g3mf0lx21y7dxk41-crun-fuzzer-0.0.1
CRUN=/nix/store/wgwpvlvpw94s1k7ir5rkw01v56454mpy-crun-harness-1.23.1/bin/crun
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

CAMPAIGN_DIRS=(/tmp/c1_0 /tmp/c1_1 /tmp/c1_2)
CORES=(0 1 2)
PIDS=()

# ── Pre-flight: check cores are not already pinned by another process ─────────
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

# ── kill_our_fuzzers: terminate processes by CWD — never touches other users ──
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

    kill_our_fuzzers

    echo "  Waiting for fuzzers to flush stats..."
    sleep 3
    wait 2>/dev/null || true

    # ── Plot final results ────────────────────────────────────────────────
    echo "==> Generating final plots..."
    OUT="$SCRIPT_DIR/c1_parallel_$(date +%Y%m%d_%H%M%S).png"
    if python3 "$SCRIPT_DIR/web_campaign/plot_final.py" "${CAMPAIGN_DIRS[@]}" "$OUT"; then
        echo "==> Plot saved: $OUT"
    else
        echo "==> Plot skipped (no data yet)."
    fi

    # ── Clean up /tmp ─────────────────────────────────────────────────────
    echo "==> Cleaning up /tmp campaign directories..."
    for dir in "${CAMPAIGN_DIRS[@]}"; do
        sudo rm -rf "$dir"
        echo "    rm -rf $dir"
    done
    rm -f /tmp/c1_{0,1,2}_fuzz.log

    echo "==> Done."
    exit 0
}

trap cleanup INT TERM

# ── Create working dirs ───────────────────────────────────────────────────────
for dir in "${CAMPAIGN_DIRS[@]}"; do mkdir -p "$dir"; done

# ── Campaign 1 — 3 instances on cores 0, 1, 2 ────────────────────────────────
for i in 0 1 2; do
    (cd /tmp/c1_$i && taskset -c $i sudo unshare -m \
        "$FUZZER/bin/forkserver_simple" -g "$FUZZER/share/grammar.py" "$CRUN" @@ \
        2>&1 | tee /tmp/c1_${i}_fuzz.log) &
    PIDS+=($!)
    echo "C1-inst-$i started on core $i (pid ${PIDS[-1]})"
done

echo ""
echo "All 3 fuzzers launched."
echo "Dashboard : python3 $SCRIPT_DIR/web_campaign/server.py"
echo "Press Ctrl+C to stop, save plots, and clean up /tmp."
wait

#!/usr/bin/env bash

FUZZER=/nix/store/p3bm9g848p95kr4avw01ags8gf1rp8nk-crun-fuzzer-0.0.1
#CRUN=/nix/store/arwkshyi5pj6f4j6a6nrvrf89irhgdp4-crun-harness-1.23.1/bin/crun -- without ASAN & False positives in crash detection.
#CRUN=/nix/store/rc2jnhbq6n62xjql87jm22xibad02n2n-crun-harness-asan-1.23.1/bin/crun -- with ASAN & False Positives in crash detection.
#CRUN=/nix/store/4w5j3vmpd4rl71c0vxzkl5mwq4mqjnz7-crun-harness-asan-1.23.1/bin/crun # with ASAN & No False Positives in crash detection.
CRUN=/nix/store/nm1sr5r2gzckh90y68avwa6fzp8hq83i-crun-harness-1.23.1/bin/crun # without ASAN & No False Positives in crash detection.
COMBINED_BIN=/home/arjun/mpi-sp/mutator/target/release/fuzz_combined_afl
GRAMMAR=/nix/store/p3bm9g848p95kr4avw01ags8gf1rp8nk-crun-fuzzer-0.0.1/share/grammar.py # new grammar
#GRAMMAR=/nix/store/2hpav3yiv5fffrs9g3mf0lx21y7dxk41-crun-fuzzer-0.0.1/share/grammar.py -- old grammar
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

CAMPAIGN_DIRS=(/tmp/c3_0 /tmp/c3_1 /tmp/c3_2 /tmp/c3_3 /tmp/c3_4 /tmp/c3_5)
CORES=(0 1 2 3 4 5)
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

    echo "  Waiting for fuzzers to flush stats..."
    sleep 3
    wait 2>/dev/null || true

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
    done
    rm -f /tmp/c3_{0,1,2,3,4,5}_fuzz.log

    echo "==> Done."
    exit 0
}

trap cleanup INT TERM

for dir in "${CAMPAIGN_DIRS[@]}"; do mkdir -p "$dir"; done

for i in 0 1 2 3 4 5; do
    (cd /tmp/c3_$i && RUST_BACKTRACE=full taskset -c $i sudo -E unshare -m \
        "$COMBINED_BIN" "$CRUN" "$GRAMMAR" \
        2>&1 | tee /tmp/c3_${i}_fuzz.log) &
    PIDS+=($!)
    echo "C3-inst-$i started on core $i (pid ${PIDS[-1]})"
done

echo ""
echo "All 6 fuzzers launched."
echo "Dashboard : python3 $SCRIPT_DIR/web_campaign/server.py"
echo "Press Ctrl+C to stop, save plots, and clean up /tmp."
wait

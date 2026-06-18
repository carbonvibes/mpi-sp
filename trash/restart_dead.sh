#!/usr/bin/env bash
[[ $EUID -ne 0 ]] && exec sudo "$0" "$@"

FUZZER=/nix/store/2hpav3yiv5fffrs9g3mf0lx21y7dxk41-crun-fuzzer-0.0.1
CRUN=/nix/store/bamjhvk6vc4w6f0gpiyyvdyxx825i7l0-crun-harness-1.23.1/bin/crun
COMBINED_BIN=/home/arjun/mpi-sp/mutator/target/release/fuzz_combined_afl
GRAMMAR=/nix/store/2hpav3yiv5fffrs9g3mf0lx21y7dxk41-crun-fuzzer-0.0.1/share/grammar.py

declare -A INSTANCE_CORE=(
    [c1_0]=0 [c1_1]=1 [c1_2]=2
    [c3_0]=3 [c3_1]=4 [c3_2]=5
)

is_running() {
    local dir=$1
    # Only count the actual fuzzer binary, not lingering bash/tee/sudo shells
    for pid in $(ls /proc 2>/dev/null | grep -E '^[0-9]+$'); do
        cwd=$(readlink "/proc/$pid/cwd" 2>/dev/null)
        [[ "$cwd" != "$dir" ]] && continue
        exe=$(readlink "/proc/$pid/exe" 2>/dev/null)
        case "$exe" in
            */forkserver_simple|*/fuzz_combined_afl) return 0 ;;
        esac
    done
    return 1
}

for inst in c1_0 c1_1 c1_2 c3_0 c3_1 c3_2; do
    dir="/tmp/$inst"
    core=${INSTANCE_CORE[$inst]}

    if is_running "$dir"; then
        echo "$inst: already running on core $core — skipping"
        continue
    fi

    mkdir -p "$dir"
    echo "$inst: dead — restarting on core $core"

    if [[ "$inst" == c1_* ]]; then
        (cd "$dir" && taskset -c "$core" sudo unshare -m \
            "$FUZZER/bin/forkserver_simple" -g "$FUZZER/share/grammar.py" "$CRUN" @@ \
            2>&1 | tee -a "/tmp/${inst}_fuzz.log") &
    else
        (cd "$dir" && taskset -c "$core" sudo unshare -m \
            "$COMBINED_BIN" "$CRUN" "$GRAMMAR" --resume \
            2>&1 | tee -a "/tmp/${inst}_fuzz.log") &
    fi

    echo "$inst: started (pid $!)"
done

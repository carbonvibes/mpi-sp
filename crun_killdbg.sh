#!/usr/bin/env bash
# Crash triage for the crun fuzzer. Replays recorded crashes through the SAME
# FUSE pipeline the fuzzer uses (replay_crash) — host-safe (FUSE rootfs disposable
# + unshare -m contains all mounts) — and buckets the outcome by signal.
#
#   sudo bash crun_killdbg.sh base              one replay each; signal histogram
#   sudo bash crun_killdbg.sh ubsan             UBSan: + distinct UB sites
#   sudo bash crun_killdbg.sh asan              ASAN:  + distinct ASAN errors
#   sudo bash crun_killdbg.sh base --calibrate  replay each REPEAT times; CONFIRM a
#                                               crash only if >=THRESHOLD runs fault
#                                               (defaults 5 / 2) — beats the flakiness
#   sudo bash crun_killdbg.sh base --trace      strace one replay each: who sends SIGKILL
#
#   PER=<n>        crashes sampled per instance        (default 12)
#   REPEAT=<n>     replays per crash                   (default 1; 5 under --calibrate)
#   THRESHOLD=<n>  min faulting runs to confirm        (default 1; 2 under --calibrate)
#
# Fault signals (= a real bug): SIGILL(4) SIGABRT(6) SIGBUS(7) SIGFPE(8) SIGSEGV(11).
# Resource / self-kill (not a bug): SIGKILL(9) SIGXCPU(24) SIGXFSZ(25).
# Note: --calibrate does REPEAT x the work, so use a modest PER or expect it to take a while.
set -u

TARGET="${1:-base}"; [ $# -gt 0 ] && shift
TRACE=0; CAL=0
for a in "$@"; do
  case "$a" in
    --trace)              TRACE=1 ;;
    --calibrate|-c|-th)   CAL=1 ;;
    *) echo "unknown flag: $a"; echo "usage: $0 [base|asan|ubsan] [--calibrate] [--trace]"; exit 1 ;;
  esac
done

GRAMMAR=/home/arjun/mpi-sp/SemanticSanitizer/case-studies/oci/grammar.py
REPLAY=/home/arjun/mpi-sp/mutator/target/release/replay_crash
PER="${PER:-12}"
if [ $CAL = 1 ]; then REPEAT="${REPEAT:-5}"; THRESHOLD="${THRESHOLD:-2}"; else REPEAT="${REPEAT:-1}"; THRESHOLD="${THRESHOLD:-1}"; fi
[ $TRACE = 1 ] && REPEAT=1   # tracing is a single-shot diagnostic
SAN_ENV=""

case "$TARGET" in
  base)  CRUN=/nix/store/mw0ahwgdmayvml2ch0hsi0j6kf5x49as-crun-harness-1.23.1/bin/crun
         DIRS=(/tmp/c3_0 /tmp/c3_1 /tmp/c3_2 /tmp/c3_3 /tmp/c3_4 /tmp/c3_5) ;;
  asan)  CRUN=/nix/store/vif1fpiq9ckx0kw5qrdgab6bl1xdj5yd-crun-harness-asan-1.23.1/bin/crun
         DIRS=(/tmp/c3_asan) ;;
  ubsan) CRUN=/nix/store/1p3qp63k86rbg3ifihiavp6c3q4ds9vm-crun-harness-ubsan-1.23.1/bin/crun
         DIRS=(/tmp/c3_ubsan)
         SAN_ENV="UBSAN_OPTIONS=halt_on_error=1:abort_on_error=1:print_stacktrace=1:report_error_type=1" ;;
  *) echo "usage: $0 [base|asan|ubsan] [--calibrate] [--trace]"; exit 1 ;;
esac

for f in "$CRUN" "$GRAMMAR" "$REPLAY"; do [ -e "$f" ] || { echo "ERROR: missing $f"; exit 1; }; done
[ $TRACE = 1 ] && ! command -v strace >/dev/null && { echo "strace missing — dropping --trace"; TRACE=0; }

mapfile -t CRASHES < <(
  for d in "${DIRS[@]}"; do
    ls "$d"/crashes/combined_* 2>/dev/null | grep -E 'combined_[0-9]+$' | head -n "$PER"
  done
)
[ "${#CRASHES[@]}" -eq 0 ] && { echo "No crashes for '$TARGET' under ${DIRS[*]}/crashes."; exit 0; }

cd /tmp 2>/dev/null || cd /    # step out before the wipe so we never rm our own CWD
OUT="/tmp/crun_triage_$TARGET"; rm -rf "$OUT"; mkdir -p "$OUT"
cd "$OUT" || { echo "ERROR: cannot cd $OUT"; exit 1; }   # harness's throwaway rootfs/ lands here
echo "=== target $TARGET : $CRUN"
echo "=== ${#CRASHES[@]} crash(es) x REPEAT=$REPEAT (THRESHOLD=$THRESHOLD)$([ $TRACE = 1 ] && echo ' +strace')"
echo "=== confirmed iff >=$THRESHOLD of $REPEAT replays fault (SIGILL/ABRT/BUS/FPE/SEGV)"
echo

declare -A SIGNAME=([4]=SIGILL [5]=SIGTRAP [6]=SIGABRT [7]=SIGBUS [8]=SIGFPE [11]=SIGSEGV [9]=SIGKILL [15]=SIGTERM [24]=SIGXCPU [25]=SIGXFSZ [31]=SIGSYS)
FAULTS=" 4 6 7 8 11 "
declare -A hist
confirmed=""; flaky=0; clean=0

replay_sig() {   # $1 crash  $2 logfile -> echoes signal number (0 = no crash)
  local crash="$1" log="$2"
  if [ $TRACE = 1 ]; then
    env $SAN_ENV timeout 45 unshare -m --propagation private \
      strace -f -tt -e trace=kill,tgkill,tkill,execve -o "${log%.log}.strace" \
      "$REPLAY" "$crash" "$GRAMMAR" "$CRUN" > "$log" 2>&1
  else
    env $SAN_ENV timeout 40 unshare -m --propagation private \
      "$REPLAY" "$crash" "$GRAMMAR" "$CRUN" > "$log" 2>&1
  fi
  local s; s=$(grep -m1 -oE 'killed by signal [0-9]+' "$log" | grep -oE '[0-9]+')
  echo "${s:-0}"
}

for crash in "${CRASHES[@]}"; do
  inst=$(echo "$crash" | sed -E 's#/tmp/([^/]+)/.*#\1#')
  tag="${inst}_$(basename "$crash")"
  declare -A csig=()
  faultruns=0
  for r in $(seq 1 "$REPEAT"); do
    s=$(replay_sig "$crash" "$OUT/${tag}_r${r}.log")
    csig[$s]=$(( ${csig[$s]:-0} + 1 ))
    hist[$s]=$(( ${hist[$s]:-0} + 1 ))
    [[ "$FAULTS" == *" $s "* ]] && faultruns=$((faultruns + 1))
  done
  if [ "$faultruns" -ge "$THRESHOLD" ]; then
    dom=11; dommax=0
    for s in 4 6 7 8 11; do c=${csig[$s]:-0}; [ "$c" -gt "$dommax" ] && { dommax=$c; dom=$s; }; done
    confirmed+=" ${tag}:${SIGNAME[$dom]}(${faultruns}/${REPEAT})"
  elif [ "$faultruns" -gt 0 ]; then
    flaky=$((flaky + 1))
  else
    clean=$((clean + 1))
  fi
  unset csig
done

# clean up FUSE mountpoints / temp configs the replays left behind
mount 2>/dev/null | grep -oE '/tmp/replay_fuse_[0-9]+' | xargs -r -n1 fusermount3 -u 2>/dev/null
rmdir /tmp/replay_fuse_* 2>/dev/null; rm -f /tmp/replay_config_*.json 2>/dev/null

nconf=$(echo $confirmed | wc -w)
reproduced=$((nconf + flaky))
total_runs=$(( ${#CRASHES[@]} * REPEAT ))
faultreplays=0; for s in 4 6 7 8 11; do faultreplays=$(( faultreplays + ${hist[$s]:-0} )); done

echo "===================== VERDICT ====================="
printf "  %-34s %d / %d\n" "REPRODUCED a fault (>=1 of $REPEAT):" "$reproduced" "${#CRASHES[@]}"
echo   "      ^ the real reproducer set"
printf "  %-34s %d\n"      "    high-confidence (>=$THRESHOLD of $REPEAT):" "$nconf"
printf "  %-34s %d\n"      "    rare/flaky (1..$((THRESHOLD-1)) of $REPEAT):" "$flaky"
printf "  %-34s %d / %d\n" "never reproduced:" "$clean" "${#CRASHES[@]}"
echo   "      ^ live-run / self-kill noise (clean on standalone replay)"
printf "  %-34s %d / %d\n" "per-replay fault rate:" "$faultreplays" "$total_runs"
echo
echo "  fault signals seen:"
for s in $(printf '%s\n' "${!hist[@]}" | sort -n); do
  [[ "$FAULTS" == *" $s "* ]] && printf "      %-8s %d replays\n" "${SIGNAME[$s]:-sig$s}" "${hist[$s]}"
done
echo
echo "  high-confidence reproducers (>=$THRESHOLD/$REPEAT):"
if [ -n "$confirmed" ]; then echo "$confirmed" | tr ' ' '\n' | grep -v '^$' | sort | sed 's/^/      /'
else echo "      (none cleared the $THRESHOLD/$REPEAT bar)"; fi
echo
echo "  NOTE: on a standalone replay, ANY fault = a real crash, so the rare/flaky ones"
echo "        are the SAME bug firing rarely (alloc-failure races). The true reproducer"
echo "        set is ~$reproduced, not just the $nconf high-confidence."
echo "==================================================="

if [ "$TARGET" = ubsan ]; then
  echo; echo "===== DISTINCT UB SITES ====="
  grep -hE 'runtime error:' "$OUT"/*.log 2>/dev/null \
    | sed -E 's#^/nix/store/[a-z0-9]+-[^/]*/##; s/(runtime error: [a-z0-9 ._-]+).*/\1/' \
    | sort | uniq -c | sort -rn
elif [ "$TARGET" = asan ]; then
  echo; echo "===== DISTINCT ASAN ERRORS ====="
  grep -hoE '(ERROR|SUMMARY): AddressSanitizer: [^ ]+' "$OUT"/*.log 2>/dev/null | sort | uniq -c | sort -rn
fi

if [ $TRACE = 1 ]; then
  echo; echo "  SIGKILL senders (si_pid):"
  grep -hE '\-\-\- SIGKILL ' "$OUT"/*.strace 2>/dev/null | grep -oE 'si_pid=[0-9]+' | sort | uniq -c | sort -rn | head
fi
echo; echo "logs: $OUT/"

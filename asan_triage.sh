#!/usr/bin/env bash
# Replay every recorded crash under the No-False-Positive ASAN crun build to
# decide which recorded "crashes" are real memory bugs vs spurious/environmental.
#
# Run as root (FUSE mount + unshare -m + crun all need it):
#     sudo bash /home/arjun/mpi-sp/asan_triage.sh
#
# Per-crash artifacts land in /tmp/asan_triage/reports/. A summary is printed at
# the end and also written to /tmp/asan_triage/summary.tsv.

set -u

ASAN=/nix/store/4w5j3vmpd4rl71c0vxzkl5mwq4mqjnz7-crun-harness-asan-1.23.1/bin/crun
REPLAY=/home/arjun/mpi-sp/mutator/target/release/replay_crash
# Must match the grammar the campaign used (the edited in-repo one), or the saved
# Nautilus tree re-renders against the wrong rule table → unfaithful replay.
GRAMMAR=/home/arjun/mpi-sp/SemanticSanitizer/case-studies/oci/grammar.py
CAMPAIGNS=(/tmp/c3_0 /tmp/c3_1 /tmp/c3_2 /tmp/c3_3 /tmp/c3_4 /tmp/c3_5)

OUT=/tmp/asan_triage
REPORTS="$OUT/reports"
SUMMARY="$OUT/summary.tsv"
rm -rf "$OUT"; mkdir -p "$REPORTS"
printf "crash\tverdict\tsignal_or_exit\tasan_summary\n" > "$SUMMARY"

# sanity
for f in "$ASAN" "$REPLAY" "$GRAMMAR"; do
  [ -e "$f" ] || { echo "MISSING: $f"; exit 1; }
done

n_total=0 n_asan=0 n_crash_noasan=0 n_nocrash=0 n_timeout=0 n_error=0

for d in "${CAMPAIGNS[@]}"; do
  inst=$(basename "$d")
  [ -d "$d/crashes" ] || continue
  for cf in $(ls "$d/crashes" 2>/dev/null | grep -E '^combined_[0-9]+$' | sort -V); do
    crash="$d/crashes/$cf"
    tag="${inst}_${cf}"
    n_total=$((n_total+1))

    rm -f /tmp/asan_reports/crun_asan.* 2>/dev/null
    log="$REPORTS/${tag}.log"
    timeout 30 unshare -m "$REPLAY" "$crash" "$GRAMMAR" "$ASAN" > "$log" 2>&1
    rc=$?

    # capture any ASAN report (log_path=/tmp/asan_reports/crun_asan.<pid>)
    asan_summary=""
    report_file=$(ls /tmp/asan_reports/crun_asan.* 2>/dev/null | head -1)
    if [ -n "$report_file" ]; then
      cp "$report_file" "$REPORTS/${tag}.asan"
      asan_summary=$(grep -m1 -E 'SUMMARY: AddressSanitizer:' "$REPORTS/${tag}.asan" \
                     | sed 's/^[[:space:]]*//')
      [ -z "$asan_summary" ] && asan_summary=$(grep -m1 -oE 'ERROR: AddressSanitizer: [A-Za-z0-9_-]+' "$REPORTS/${tag}.asan")
    fi

    # replay verdict line
    sig=$(grep -m1 -oE 'killed by signal [0-9]+' "$log" | grep -oE '[0-9]+')
    nocrash=$(grep -m1 -oE 'No crash — exit code -?[0-9]+' "$log")

    if [ -n "$asan_summary" ]; then
      verdict="ASAN-BUG"; soe="sig${sig:-?}"; n_asan=$((n_asan+1))
    elif [ -n "$sig" ]; then
      verdict="CRASH-NO-ASAN"; soe="sig${sig}"; n_crash_noasan=$((n_crash_noasan+1))
    elif [ -n "$nocrash" ]; then
      verdict="NO-CRASH"; soe="${nocrash#No crash — exit code }"; n_nocrash=$((n_nocrash+1))
    elif [ "$rc" -eq 124 ]; then
      verdict="TIMEOUT"; soe="timeout30s"; n_timeout=$((n_timeout+1))
    else
      verdict="REPLAY-ERROR"; soe="rc${rc}"; n_error=$((n_error+1))
    fi

    printf "%s\t%s\t%s\t%s\n" "$tag" "$verdict" "$soe" "${asan_summary:-}" >> "$SUMMARY"
    printf "  %-18s %-14s %-10s %s\n" "$tag" "$verdict" "$soe" "${asan_summary:-}"
  done
done

# leftover empty FUSE mountpoint dirs from any crashed/timed-out replays
rmdir /tmp/replay_fuse_* 2>/dev/null
rm -f /tmp/replay_config_*.json 2>/dev/null

echo ""
echo "================ ASAN TRIAGE SUMMARY ================"
echo "  total crashes replayed : $n_total"
echo "  ASAN-BUG (real bug)     : $n_asan"
echo "  CRASH-NO-ASAN (signal, no ASAN report) : $n_crash_noasan"
echo "  NO-CRASH (did not reproduce / spurious): $n_nocrash"
echo "  TIMEOUT                 : $n_timeout"
echo "  REPLAY-ERROR            : $n_error"
echo ""
echo "  Distinct ASAN bug signatures (SUMMARY line, by count):"
awk -F'\t' '$2=="ASAN-BUG"{print $4}' "$SUMMARY" | sort | uniq -c | sort -rn | sed 's/^/    /'
echo ""
echo "  Full table: $SUMMARY"
echo "  Per-crash ASAN reports + replay logs: $REPORTS/"
echo "====================================================="

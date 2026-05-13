#!/usr/bin/env bash
# Run as: sudo bash stop_campaigns.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CAMPAIGN_DIRS=(/tmp/c1_0 /tmp/c1_1 /tmp/c1_2 /tmp/c2_0 /tmp/c2_1 /tmp/c2_2)

if [[ "$EUID" -ne 0 ]]; then
    echo "ERROR: run this as root:  sudo bash stop_campaigns.sh"
    exit 1
fi

# ── Step 1: kill every process whose CWD is one of our campaign dirs ──────────
echo "==> Stopping fuzzers..."
killed=0
for dir in "${CAMPAIGN_DIRS[@]}"; do
    [[ -d "$dir" ]] || continue
    for pid in $(ls /proc 2>/dev/null | grep -E '^[0-9]+$'); do
        cwd=$(readlink "/proc/$pid/cwd" 2>/dev/null) || continue
        if [[ "$cwd" == "$dir" ]]; then
            comm=$(cat "/proc/$pid/comm" 2>/dev/null || echo "?")
            echo "  SIGTERM pid=$pid ($comm) in $dir"
            kill -TERM "$pid" 2>/dev/null || true
            killed=$((killed + 1))
        fi
    done
done

if [[ $killed -eq 0 ]]; then
    echo "  No fuzzer processes found in campaign dirs — already stopped?"
else
    echo "  Sent SIGTERM to $killed process(es). Waiting 4s for stats flush..."
    sleep 4

    # Force-kill anything still alive in those dirs
    for dir in "${CAMPAIGN_DIRS[@]}"; do
        [[ -d "$dir" ]] || continue
        for pid in $(ls /proc 2>/dev/null | grep -E '^[0-9]+$'); do
            cwd=$(readlink "/proc/$pid/cwd" 2>/dev/null) || continue
            if [[ "$cwd" == "$dir" ]]; then
                comm=$(cat "/proc/$pid/comm" 2>/dev/null || echo "?")
                echo "  SIGKILL pid=$pid ($comm) — still alive after SIGTERM"
                kill -KILL "$pid" 2>/dev/null || true
            fi
        done
    done
fi

# ── Step 2: plot final results ────────────────────────────────────────────────
echo ""
echo "==> Generating final plots..."
OUT="$SCRIPT_DIR/parallel_$(date +%Y%m%d_%H%M%S).png"

# Run plotter as the original user so the PNG is not owned by root
REAL_USER="${SUDO_USER:-arjun}"
if sudo -u "$REAL_USER" python3 "$SCRIPT_DIR/web_campaign/plot_final.py" \
        "${CAMPAIGN_DIRS[@]}" "$OUT"; then
    echo "==> Plot saved: $OUT"
else
    echo "==> Plot skipped (no data in any campaign dir)."
fi

# ── Step 3: back up then remove from /tmp ────────────────────────────────────
BACKUP="/tmp/campaign-backup/$(date +%Y%m%d_%H%M%S)"
mkdir -p "$BACKUP"
echo ""
echo "==> Moving campaign dirs to $BACKUP ..."
for dir in "${CAMPAIGN_DIRS[@]}"; do
    if [[ -d "$dir" ]]; then
        mv "$dir" "$BACKUP/"
        echo "    mv $dir -> $BACKUP/$(basename "$dir")"
    fi
done

# Move log files into the backup too
for log in /tmp/c1_0_fuzz.log /tmp/c1_1_fuzz.log /tmp/c1_2_fuzz.log \
           /tmp/c2_0_fuzz.log /tmp/c2_1_fuzz.log /tmp/c2_2_fuzz.log; do
    [[ -f "$log" ]] && mv "$log" "$BACKUP/" && echo "    mv $log -> $BACKUP/"
done

# Copy the plot into the backup as well so everything is in one place
[[ -f "$OUT" ]] && cp "$OUT" "$BACKUP/plot.png"

echo ""
echo "==> Done."
echo "    Backup : $BACKUP"
[[ -f "$OUT" ]] && echo "    Plot   : $OUT"

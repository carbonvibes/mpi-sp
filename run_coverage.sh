#!/usr/bin/env bash
# coverage pipeline. run after the campaign has a corpus (no need to stop it).
# usage: sudo bash run_coverage.sh
# output: /tmp/cov_report/index.html, served at http://localhost:8099

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

REPLAY_BIN="$SCRIPT_DIR/mutator/target/release/replay_coverage"
COV_CRUN="/nix/store/z2s0whk5bhid8w65h64dm2npxkflixgc-crun-harness-cov-1.23.1/bin/crun"
GRAMMAR="/nix/store/p3bm9g848p95kr4avw01ags8gf1rp8nk-crun-fuzzer-0.0.1/share/grammar.py"

LLVM_PROFDATA="/nix/store/jp45dqzv8mjpqsvhj99c93pc4vlmhy16-llvm-21.1.8/bin/llvm-profdata"
LLVM_COV="/nix/store/jp45dqzv8mjpqsvhj99c93pc4vlmhy16-llvm-21.1.8/bin/llvm-cov"

CAMPAIGN_DIRS=(/tmp/c3_0 /tmp/c3_1 /tmp/c3_2 /tmp/c3_3 /tmp/c3_4 /tmp/c3_5)

PROFILES_BASE="/tmp/cov_profiles"
MERGED_PROF="/tmp/cov_merged.profdata"
REPORT_DIR="/tmp/cov_report"

# only replay entries from first CUTOFF_HOURS; coverage saturates ~4h, the tail
# adds no edges. CUTOFF_HOURS=0 replays everything.
CUTOFF_HOURS="${CUTOFF_HOURS:-20}"

echo "==> Checking binaries..."
for f in "$REPLAY_BIN" "$COV_CRUN" "$GRAMMAR" "$LLVM_PROFDATA" "$LLVM_COV"; do
    if [[ ! -e "$f" ]]; then
        echo "ERROR: not found: $f"
        echo "       Build first:"
        echo "         cd $SCRIPT_DIR/mutator && cargo build --release --bin replay_coverage"
        echo "         cd $SCRIPT_DIR/SemanticSanitizer && nix build .#\"artifact-eval.crun-harness-cov\""
        exit 1
    fi
done
echo "    All binaries present."

echo ""
echo "==> Checking corpus..."
total_entries=0
for dir in "${CAMPAIGN_DIRS[@]}"; do
    corpus="$dir/corpus"
    if [[ ! -d "$corpus" ]]; then
        echo "    $corpus — not found, skipping"
        continue
    fi
    # find, not glob: glob blows past ARG_MAX on 160k+ entry corpora
    count=$(find "$corpus" -maxdepth 1 -name 'combined_*' ! -name '*.json' | wc -l)
    echo "    $corpus — $count entries"
    total_entries=$((total_entries + count))
done

if [[ $total_entries -eq 0 ]]; then
    echo "ERROR: No corpus entries found. Has the campaign run long enough?"
    exit 1
fi
echo "    Total: $total_entries entries across all instances"

echo ""
echo "==> Step 1: replaying corpus through coverage binary..."
echo "    (each instance replayed separately, profiles in $PROFILES_BASE/c3_N/)"
echo ""

rm -rf "$PROFILES_BASE"
mkdir -p "$PROFILES_BASE"

# echoes "--before-epoch <ts>" to limit replay to the first CUTOFF_HOURS.
# start = mtime of combined_0 (startup seed), else earliest entry mtime.
cutoff_args() {
    local dir="$1"
    [[ "$CUTOFF_HOURS" -eq 0 ]] && return 0
    local start
    start=$(stat -c %Y "$dir/combined_0" 2>/dev/null)
    if [[ -z "$start" ]]; then
        start=$(find "$dir" -maxdepth 1 -name 'combined_*' ! -name '*.json' \
                    -printf '%T@\n' 2>/dev/null | sort -n | head -1 | cut -d. -f1)
    fi
    [[ -z "$start" ]] && return 0
    echo "--before-epoch $(( start + CUTOFF_HOURS * 3600 ))"
}

for i in 0 1 2 3 4 5; do
    dir="/tmp/c3_$i/corpus"
    if [[ ! -d "$dir" ]]; then continue; fi
    count=$(find "$dir" -maxdepth 1 -name 'combined_*' ! -name '*.json' | wc -l)
    if [[ $count -eq 0 ]]; then continue; fi

    profile_dir="$PROFILES_BASE/c3_$i"
    mkdir -p "$profile_dir"

    echo "--- Instance c3_$i corpus ($count entries, first ${CUTOFF_HOURS}h) ---"
    unshare -m "$REPLAY_BIN" \
        "$dir" \
        "$GRAMMAR" \
        "$COV_CRUN" \
        --profile-dir "$profile_dir" \
        $(cutoff_args "$dir")
    echo ""
done

# crashes hit paths the corpus never reaches; include them in coverage
echo "==> Step 1b: replaying crashes through coverage binary..."
for i in 0 1 2 3 4 5; do
    dir="/tmp/c3_$i/crashes"
    if [[ ! -d "$dir" ]]; then continue; fi
    count=$(find "$dir" -maxdepth 1 -name 'combined_*' ! -name '*.json' | wc -l)
    if [[ $count -eq 0 ]]; then echo "    c3_$i crashes — none"; continue; fi

    profile_dir="$PROFILES_BASE/c3_${i}_crashes"
    mkdir -p "$profile_dir"

    echo "--- Instance c3_$i crashes ($count entries, first ${CUTOFF_HOURS}h) ---"
    unshare -m "$REPLAY_BIN" \
        "$dir" \
        "$GRAMMAR" \
        "$COV_CRUN" \
        --profile-dir "$profile_dir" \
        $(cutoff_args "$dir")
    echo ""
done

profraw_count=$(find "$PROFILES_BASE" -name "*.profraw" | wc -l)
echo "==> Step 2: collected $profraw_count .profraw files"

if [[ $profraw_count -eq 0 ]]; then
    echo "ERROR: No .profraw files generated. Something went wrong in replay."
    exit 1
fi

# crash inputs make crun exit via signal, so the LLVM atexit flush never runs
# and leaves corrupt .profraw. drop them before merging.
echo ""
echo "==> Step 3: merging profiles → $MERGED_PROF ..."

corrupt_count=0
while IFS= read -r f; do
    if ! "$LLVM_PROFDATA" show "$f" &>/dev/null; then
        rm -f "$f"
        corrupt_count=$((corrupt_count + 1))
    fi
done < <(find "$PROFILES_BASE" -name "*.profraw")
if [[ $corrupt_count -gt 0 ]]; then
    echo "    Removed $corrupt_count corrupt .profraw files (from crash inputs — expected)."
fi

valid_count=$(find "$PROFILES_BASE" -name "*.profraw" | wc -l)
if [[ $valid_count -eq 0 ]]; then
    echo "ERROR: No valid .profraw files remain after filtering."
    exit 1
fi
echo "    Merging $valid_count valid profiles..."

# --input-files, not argv: hundreds of thousands of profraws exceed ARG_MAX (E2BIG)
PROFLIST="$PROFILES_BASE/profraw.list"
find "$PROFILES_BASE" -name "*.profraw" > "$PROFLIST"

"$LLVM_PROFDATA" merge -sparse \
    --input-files="$PROFLIST" \
    -o "$MERGED_PROF"
echo "    Merge done."

# nix sandbox compiles at /build/source/ which isn't on the host; remap via
# -path-equivalence to the store source tree (the one with src/libcrun).
CRUN_SRC=$(for d in /nix/store/*crun* /nix/store/*source*; do
    [[ -d "$d/src/libcrun" ]] && echo "$d" && break
done 2>/dev/null)
if [[ -n "$CRUN_SRC" ]]; then
    PATH_EQ="-path-equivalence=/build/source,$CRUN_SRC"
    echo "    Source root: $CRUN_SRC"
else
    PATH_EQ=""
    echo "    WARNING: crun source not found in Nix store — source annotations will be missing"
fi

echo ""
echo "==> Step 4: coverage summary (sorted by missed lines):"
echo ""
"$LLVM_COV" report "$COV_CRUN" \
    -instr-profile="$MERGED_PROF" \
    -ignore-filename-regex='vendor|libocispec' \
    $PATH_EQ \
    2>/dev/null \
| awk '
    NR <= 2 { print; next }            # keep header
    NF >= 4 { print | "sort -k4 -rn" } # sort by missed-lines, highest first
'
echo ""

echo "==> Step 5: generating HTML report → $REPORT_DIR ..."
rm -rf "$REPORT_DIR"
mkdir -p "$REPORT_DIR"
"$LLVM_COV" show "$COV_CRUN" \
    -instr-profile="$MERGED_PROF" \
    -format=html \
    -output-dir="$REPORT_DIR" \
    -show-line-counts-or-regions \
    -show-branches=count \
    -ignore-filename-regex='vendor|libocispec' \
    $PATH_EQ

# step 6: plain-text analysis file for pasting into Claude
COV_ANALYSIS="$REPORT_DIR/cov_analysis.txt"
echo ""
echo "==> Step 6: generating structured analysis → $COV_ANALYSIS ..."

{
echo "============================================================"
echo "COVERAGE ANALYSIS REPORT"
echo "Generated : $(date)"
echo "Binary    : $COV_CRUN"
echo "Corpus    : $total_entries entries across all instances"
echo "Profiles  : $profraw_count .profraw files merged"
echo "============================================================"
echo ""

echo "------------------------------------------------------------"
echo "SECTION 1: OVERALL SUMMARY"
echo "------------------------------------------------------------"
"$LLVM_COV" report "$COV_CRUN" \
    -instr-profile="$MERGED_PROF" \
    -ignore-filename-regex='vendor|libocispec' \
    $PATH_EQ \
    2>/dev/null | tail -3
echo ""

# llvm-cov report cols (no -show-functions): $1=File $4=RegCover% $7=FuncExec%
# $8=Lines $9=MissedLines $10=LineCover%; branches add $11-$13
echo "------------------------------------------------------------"
echo "SECTION 2: PER-FILE COVERAGE (sorted by missed lines, highest first)"
echo "Columns: Filename | Regions | MissedRegions | RegCover% | Functions | MissedFuncs | FuncExec% | Lines | MissedLines | LineCover%"
echo "------------------------------------------------------------"
"$LLVM_COV" report "$COV_CRUN" \
    -instr-profile="$MERGED_PROF" \
    -ignore-filename-regex='vendor|libocispec' \
    2>/dev/null \
| awk '
    /^Filename/ { print; next }
    /^---/      { print; next }
    /^TOTAL/    { totline=$0; next }
    NF >= 9     { lines[NR] = $0; missed[NR] = $9+0 }
    END {
        n = asorti(missed, idx, "@val_num_desc")
        for (i = 1; i <= n; i++) print lines[idx[i]]
        print ""
        print totline
    }
'
echo ""

echo "------------------------------------------------------------"
echo "SECTION 3: ZERO-COVERAGE FUNCTIONS (never called during corpus replay)"
echo "Format: function_name  [file]"
echo "------------------------------------------------------------"
# -show-functions: one row per function, $10=LineCover%, 0.00%=never run
"$LLVM_COV" report "$COV_CRUN" \
    -instr-profile="$MERGED_PROF" \
    -show-functions \
    -ignore-filename-regex='vendor|libocispec' \
    2>/dev/null \
| awk '
    NF >= 10 && $10 == "0.00%" {
        print $1 "  [" $NF "]"
    }
' | sort -u
echo ""

echo "------------------------------------------------------------"
echo "SECTION 4: COVERAGE TIERS (by line coverage %)"
echo "------------------------------------------------------------"
echo ""

RAW_REPORT=$("$LLVM_COV" report "$COV_CRUN" \
    -instr-profile="$MERGED_PROF" \
    -ignore-filename-regex='vendor|libocispec' \
    2>/dev/null)

echo ">> UNREACHED FILES (0% line coverage):"
echo "$RAW_REPORT" | awk 'NF >= 10 && $10 == "0.00%" { print "  " $10 "\t" $1 }' | head -30
echo ""

echo ">> BARELY TOUCHED (1-10% line coverage):"
echo "$RAW_REPORT" | awk '
    NF >= 10 && $10 ~ /^[0-9]+\.[0-9]+%$/ {
        pct = $10; sub(/%$/, "", pct)
        if (pct+0 > 0 && pct+0 < 10) print "  " $10 "\t" $1
    }
' | sort -k1 -n | head -30
echo ""

echo ">> PARTIALLY COVERED (10-60% line coverage):"
echo "$RAW_REPORT" | awk '
    NF >= 10 && $10 ~ /^[0-9]+\.[0-9]+%$/ {
        pct = $10; sub(/%$/, "", pct)
        if (pct+0 >= 10 && pct+0 < 60) print "  " $10 "\t" $1
    }
' | sort -k1 -n | head -30
echo ""

echo ">> WELL COVERED (60%+ line coverage):"
echo "$RAW_REPORT" | awk '
    NF >= 10 && $10 ~ /^[0-9]+\.[0-9]+%$/ {
        pct = $10; sub(/%$/, "", pct)
        if (pct+0 >= 60) print "  " $10 "\t" $1
    }
' | sort -k1 -rn | head -20
echo ""

echo "------------------------------------------------------------"
echo "SECTION 5: UNCOVERED FUNCTIONS IN PARTIALLY-COVERED FILES"
echo "(files that have some coverage but specific functions never called)"
echo "------------------------------------------------------------"
"$LLVM_COV" report "$COV_CRUN" \
    -instr-profile="$MERGED_PROF" \
    -show-functions \
    -ignore-filename-regex='vendor|libocispec' \
    2>/dev/null \
| awk '
    NF >= 10 && $10 == "0.00%" {
        print $1 "\t0%\t" $NF
    }
' | sort -t$'\t' -k3 | head -80
echo ""

echo "============================================================"
echo "END OF REPORT"
echo "============================================================"
} > "$COV_ANALYSIS"

echo "    Analysis saved: $COV_ANALYSIS"

echo ""
echo "==> Done."
echo ""
echo "    Structured analysis : $COV_ANALYSIS   ← paste this into Claude"
echo "    HTML report         : $REPORT_DIR/index.html"
echo "    Merged prof         : $MERGED_PROF"
echo ""
echo "Serving HTML report at http://localhost:8099 (Ctrl-C to stop)"
python3 -m http.server 8099 --directory "$REPORT_DIR"

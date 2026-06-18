#!/usr/bin/env bash

set -euo pipefail

if [ ! -f "./semsan-cli" ]; then
  echo "Error: ./semsan-cli not found. Please build semsan-cli first as per the instructions in the README."
  exit 1
fi

if [ ! -f "nix/packages/by-name/artifact-eval/micro-benchmark/config-general.yaml" ]; then
  echo "Error: This script must be run from the root of the repository."
  exit 1
fi

BENCHMARKS=(
  "benchmark-general:nix/packages/by-name/artifact-eval/micro-benchmark/config-general.yaml:false"
  "benchmark-symlinkmount:nix/packages/by-name/artifact-eval/micro-benchmark/config-symlinkmount.yaml:true"
  "benchmark-dirownership:nix/packages/by-name/artifact-eval/micro-benchmark/config-dirownership.yaml:false"
  "benchmark-canary:nix/packages/by-name/artifact-eval/micro-benchmark/config-canary.yaml:false"
)

declare -A BASELINE_MED
declare -A SEMSAN_MED
declare -A OVERHEAD_PCT

extract_median_iterations() {
  local logfile="$1"

  awk '
        /^Iterations:/ {
            vals[count++] = $2
        }
        END {
            if (count == 0) {
                exit 1
            }
            # sort
            for (i = 0; i < count; i++)
                for (j = i + 1; j < count; j++)
                    if (vals[i] > vals[j]) {
                        t = vals[i]; vals[i] = vals[j]; vals[j] = t
                    }
            if (count % 2 == 1)
                printf "%.2f", vals[int(count/2)]
            else
                printf "%.2f", (vals[count/2 - 1] + vals[count/2]) / 2
        }
    ' "${logfile}"
}

compute_overhead_pct() {
  local baseline_med="$1"
  local semsan_med="$2"

  awk -v baseline="${baseline_med}" -v semsan="${semsan_med}" '
        BEGIN {
            if (baseline == 0) {
                print "n/a"
            } else {
                printf "%.2f", ((baseline - semsan) / baseline) * 100
            }
        }
    '
}

run_and_capture() {
  local logfile="$1"
  shift

  stdbuf -oL "$@" | tee "${logfile}"
}

run_benchmark() {
  local benchmark="$1"
  local config="$2"
  local needs_root="$3"
  local baseline_log
  local semsan_log
  local baseline_med
  local semsan_med
  local overhead_pct
  local semsan_pid

  local cmd=("${benchmark}")
  if [ "${needs_root}" = "true" ]; then
    cmd=(sudo "$(command -v "${benchmark}")")
  fi

  baseline_log=$(mktemp)
  semsan_log=$(mktemp)

  echo "Running ${benchmark} without SemSan..."
  run_and_capture "${baseline_log}" "${cmd[@]}"
  baseline_med=$(extract_median_iterations "${baseline_log}")

  echo "Running ${benchmark} with SemSan..."
  sudo ./semsan-cli attach --config "${config}" &
  semsan_pid=$!

  run_and_capture "${semsan_log}" "${cmd[@]}"
  semsan_med=$(extract_median_iterations "${semsan_log}")

  echo "Stopping SemSan..."
  sudo kill "${semsan_pid}"
  wait "${semsan_pid}" 2>/dev/null || true

  rm -f "${baseline_log}" "${semsan_log}"

  overhead_pct=$(compute_overhead_pct "${baseline_med}" "${semsan_med}")

  BASELINE_MED["${benchmark}"]="${baseline_med}"
  SEMSAN_MED["${benchmark}"]="${semsan_med}"
  OVERHEAD_PCT["${benchmark}"]="${overhead_pct}"
}

print_summary_table() {
  local benchmark

  echo
  echo "Summary"
  printf '%-24s %18s %18s %14s\n' "Benchmark" "Med w/o SemSan" "Med w/ SemSan" "Overhead %"
  printf '%-24s %18s %18s %14s\n' "---------" "---------------" "-------------" "----------"

  for entry in "${BENCHMARKS[@]}"; do
    benchmark=${entry%%:*}
    printf '%-24s %18s %18s %14s\n' \
      "${benchmark}" \
      "${BASELINE_MED["${benchmark}"]}" \
      "${SEMSAN_MED["${benchmark}"]}" \
      "${OVERHEAD_PCT["${benchmark}"]}"
  done
}

for entry in "${BENCHMARKS[@]}"; do
  benchmark=${entry%%:*}
  rest=${entry#*:}
  config=${rest%:*}
  needs_root=${rest##*:}
  run_benchmark "${benchmark}" "${config}" "${needs_root}"
done

print_summary_table

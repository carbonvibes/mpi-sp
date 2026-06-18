#!/usr/bin/env bash

set -euo pipefail

if [ ! -f "./semsan-cli" ]; then
  echo "Error: ./semsan-cli not found. Please build semsan-cli first as per the instructions in the README."
  exit 1
fi

if [ ! -f "nix/packages/by-name/artifact-eval/macro-benchmark/benchmark-config.yaml" ]; then
  echo "Error: This script must be run from the root of the repository."
  exit 1
fi

CONFIG="nix/packages/by-name/artifact-eval/macro-benchmark/benchmark-config.yaml"
NUM_RUNS=${NUM_RUNS:-9}

PGBENCH_TEST="pts/pgbench"
PGBENCH_PRESET=${PGBENCH_PRESET:-"pts/pgbench.scaling-factor=1000;pts/pgbench.clients=250;pts/pgbench.run-mode=Read Write"}

APACHE_TEST="pts/apache"
APACHE_PRESET=${APACHE_PRESET:-"pts/apache.concurrent=200"}

BENCHMARKS=("pgbench" "apache")

declare -A BENCHMARK_TEST
BENCHMARK_TEST["pgbench"]="${PGBENCH_TEST}"
BENCHMARK_TEST["apache"]="${APACHE_TEST}"

declare -A BENCHMARK_PRESET
BENCHMARK_PRESET["pgbench"]="${PGBENCH_PRESET}"
BENCHMARK_PRESET["apache"]="${APACHE_PRESET}"

declare -A BASELINE_MED
declare -A SEMSAN_MED
declare -A OVERHEAD_PCT

patch_pgbench_install() {
  local test_dir="$HOME/.phoronix-test-suite/installed-tests/pts/pgbench-1.17.0"
  local pg_dir="${test_dir}/pg_"

  echo "Patching pgbench test with Nix-provided PostgreSQL..."

  # Kill any stale postgres/pgbench left over from a previously interrupted run
  # before deleting their data directory (otherwise rm races with writes to pg_wal).
  pkill -9 -f "${pg_dir}/bin/postgres" 2>/dev/null || true
  pkill -9 -f "${test_dir}/pgbench" 2>/dev/null || true
  sleep 1

  # Remove failed source build artifacts
  rm -rf "${test_dir:?}/postgresql-"*
  rm -rf "${pg_dir:?}"

  mkdir -p "${pg_dir}/bin" "${pg_dir}/data"

  # Resolve the Nix PostgreSQL prefix
  local pg_nix_prefix
  pg_nix_prefix="$(dirname "$(dirname "$(readlink -f "$(command -v initdb)")")")"

  # Symlink bin, share, lib from the Nix postgresql closure
  ln -sf "${pg_nix_prefix}/bin/"* "${pg_dir}/bin/"
  ln -sfn "${pg_nix_prefix}/share" "${pg_dir}/share"
  ln -sfn "${pg_nix_prefix}/lib" "${pg_dir}/lib"

  # Initialize database cluster
  "${pg_dir}/bin/initdb" -D "${pg_dir}/data/db" --encoding=SQL_ASCII --locale=C

  # The phoronix-generated `pgbench` run script is corrupted: backticks inside
  # its install-time heredoc were evaluated instead of escaped, leaving broken
  # conditionals. Replace it with a working script.
  cat >"${test_dir}/pgbench" <<'PGEOF'
#!/bin/sh
# Absolute paths are required: PostgreSQL resolves its sharedir from argv[0], so a
# relative invocation like `pg_/bin/postgres` leaves the server looking for
# `pg_/share/postgresql/...` relative to the data directory at runtime.
PG_PREFIX="$HOME/pg_"
PGDATA="$PG_PREFIX/data/db"
PGPORT=7777
export PGDATA
export PGPORT

SOCKET_DIR="$PG_PREFIX/run"
mkdir -p "$SOCKET_DIR"
export PGHOST="$SOCKET_DIR"

SHARED_BUFFER_SIZE=$(echo "$SYS_MEMORY * 0.25 / 1" | bc)
SHARED_BUFFER_SIZE=$(( SHARED_BUFFER_SIZE < 8192 ? SHARED_BUFFER_SIZE : 8192 ))
echo "Buffer size is ${SHARED_BUFFER_SIZE}MB" > "$LOG_FILE"

"$PG_PREFIX/bin/pg_ctl" start -w -l "$PG_PREFIX/data/server.log" \
    -o "-c max_connections=6000 -c shared_buffers=${SHARED_BUFFER_SIZE}MB -c unix_socket_directories=$SOCKET_DIR"

"$PG_PREFIX/bin/createdb" -h "$SOCKET_DIR" pgbench
"$PG_PREFIX/bin/pgbench" -h "$SOCKET_DIR" -i $1 $2 -n pgbench
"$PG_PREFIX/bin/pgbench" -h "$SOCKET_DIR" --protocol=prepared -j "$NUM_CPU_CORES" "$@" -n -T 120 -r pgbench >>"$LOG_FILE" 2>&1
"$PG_PREFIX/bin/dropdb" -h "$SOCKET_DIR" pgbench

"$PG_PREFIX/bin/pg_ctl" stop -w
PGEOF
  chmod +x "${test_dir}/pgbench"
}

patch_apache_install() {
  local test_dir="$HOME/.phoronix-test-suite/installed-tests/pts/apache-3.0.0"
  local httpd_dir="${test_dir}/httpd_"

  echo "Patching Apache test with Nix-provided apacheHttpd..."

  # Kill any stale httpd left over from a previously interrupted run.
  pkill -9 -f "${httpd_dir}/bin" 2>/dev/null || true
  sleep 1

  # Remove failed source build artifacts
  rm -rf "${test_dir:?}/httpd-"* "${test_dir:?}/apr-"* "${test_dir:?}/apr-util-"* "${test_dir:?}/http-test-files"
  rm -rf "${httpd_dir:?}"

  mkdir -p "${httpd_dir}/bin" "${httpd_dir}/conf" "${httpd_dir}/htdocs" "${httpd_dir}/logs"

  # Resolve the Nix apacheHttpd prefix
  local httpd_nix_prefix
  httpd_nix_prefix="$(dirname "$(dirname "$(readlink -f "$(command -v apachectl)")")")"

  ln -sf "${httpd_nix_prefix}/bin/"* "${httpd_dir}/bin/"
  ln -sfn "${httpd_nix_prefix}/modules" "${httpd_dir}/modules"

  # Extract the test document tree into htdocs
  tar -xf "${test_dir}/http-test-files-1.tar.xz" -C "${test_dir}/"
  mv "${test_dir}/http-test-files/"* "${httpd_dir}/htdocs/"
  rm -rf "${test_dir}/http-test-files"

  # Minimal working httpd.conf: listen on 8088, serve htdocs, put logs/pid under httpd_
  cat >"${httpd_dir}/conf/httpd.conf" <<APCONF
ServerRoot "${httpd_nix_prefix}"
Listen 8088

LoadModule mpm_event_module modules/mod_mpm_event.so
LoadModule authn_core_module modules/mod_authn_core.so
LoadModule authz_core_module modules/mod_authz_core.so
LoadModule authz_host_module modules/mod_authz_host.so
LoadModule unixd_module modules/mod_unixd.so
LoadModule log_config_module modules/mod_log_config.so
LoadModule mime_module modules/mod_mime.so
LoadModule dir_module modules/mod_dir.so

User ${USER}
Group #-1

ServerName localhost
ServerAdmin admin@localhost

DocumentRoot "${httpd_dir}/htdocs"
<Directory "${httpd_dir}/htdocs">
    Options FollowSymLinks
    AllowOverride None
    Require all granted
</Directory>

ErrorLog "${httpd_dir}/logs/error_log"
PidFile "${httpd_dir}/logs/httpd.pid"
CustomLog "${httpd_dir}/logs/access_log" common

TypesConfig "${httpd_nix_prefix}/conf/mime.types"

<IfModule dir_module>
    DirectoryIndex test.html index.html
</IfModule>
APCONF
}

setup_phoronix() {
  echo "Configuring phoronix-test-suite batch mode..."
  # n = don't save results, n = don't run all test options (use PRESET_OPTIONS instead)
  printf 'n\nn\n' | phoronix-test-suite batch-setup

  echo "Installing phoronix tests (source builds will fail -- Nix provides the binaries)..."
  # Skip apt-based dependency checks -- deps are provided via Nix.
  # Upstream install scripts fail to compile from source (icu pkg-config, pcre-config missing);
  # we ignore the failure and patch the install tree with Nix binaries afterwards.
  yes "" | SKIP_EXTERNAL_DEPENDENCIES=1 phoronix-test-suite install "${PGBENCH_TEST}" "${APACHE_TEST}" || true

  patch_pgbench_install
  patch_apache_install
}

run_pts_test() {
  local logfile="$1"
  local test="$2"
  local preset="$3"

  PRESET_OPTIONS="${preset}" \
    stdbuf -oL phoronix-test-suite batch-run "${test}" 2>&1 | tee "${logfile}"
}

extract_result() {
  local logfile="$1"

  # Phoronix colorizes output with ANSI escape codes; strip them before awk
  # so numeric fields parse cleanly downstream.
  sed -E 's/\x1b\[[0-9;]*[a-zA-Z]//g' "${logfile}" |
    awk '/Average:/ { print $2 }' | tail -1
}

compute_median() {
  local values_file="$1"

  sort -n "${values_file}" | awk '{vals[NR]=$1} END {
    if (NR == 0) { print "n/a"; exit }
    if (NR % 2 == 1) printf "%.2f", vals[(NR+1)/2]
    else printf "%.2f", (vals[NR/2] + vals[NR/2+1]) / 2
  }'
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

run_benchmark() {
  local name="$1"
  local test="$2"
  local preset="$3"
  local baseline_values semsan_values logfile result semsan_pid

  baseline_values=$(mktemp)
  semsan_values=$(mktemp)

  echo "=== Benchmark: ${name} ==="

  echo "Running ${name} without SemSan (${NUM_RUNS} runs)..."
  for i in $(seq 1 "${NUM_RUNS}"); do
    echo "  Run ${i}/${NUM_RUNS}..."
    logfile=$(mktemp)
    run_pts_test "${logfile}" "${test}" "${preset}"
    result=$(extract_result "${logfile}")
    echo "${result}" >>"${baseline_values}"
    echo "  Result: ${result}"
    rm -f "${logfile}"
  done

  echo "Running ${name} with SemSan (${NUM_RUNS} runs)..."
  sudo ./semsan-cli attach --config "${CONFIG}" &
  semsan_pid=$!
  sleep 2

  for i in $(seq 1 "${NUM_RUNS}"); do
    echo "  Run ${i}/${NUM_RUNS}..."
    logfile=$(mktemp)
    run_pts_test "${logfile}" "${test}" "${preset}"
    result=$(extract_result "${logfile}")
    echo "${result}" >>"${semsan_values}"
    echo "  Result: ${result}"
    rm -f "${logfile}"
  done

  echo "Stopping SemSan..."
  sudo kill "${semsan_pid}" 2>/dev/null || true
  wait "${semsan_pid}" 2>/dev/null || true

  BASELINE_MED["${name}"]=$(compute_median "${baseline_values}")
  SEMSAN_MED["${name}"]=$(compute_median "${semsan_values}")
  OVERHEAD_PCT["${name}"]=$(compute_overhead_pct "${BASELINE_MED["${name}"]}" "${SEMSAN_MED["${name}"]}")

  rm -f "${baseline_values}" "${semsan_values}"
}

print_summary_table() {
  echo
  echo "Summary"
  printf '%-16s %18s %18s %14s\n' "Benchmark" "Med w/o SemSan" "Med w/ SemSan" "Overhead %"
  printf '%-16s %18s %18s %14s\n' "---------" "---------------" "-------------" "----------"

  for bench in "${BENCHMARKS[@]}"; do
    printf '%-16s %18s %18s %14s\n' \
      "${bench}" \
      "${BASELINE_MED["${bench}"]}" \
      "${SEMSAN_MED["${bench}"]}" \
      "${OVERHEAD_PCT["${bench}"]}"
  done
}

setup_phoronix

for bench in "${BENCHMARKS[@]}"; do
  run_benchmark "${bench}" "${BENCHMARK_TEST["${bench}"]}" "${BENCHMARK_PRESET["${bench}"]}"
done

print_summary_table

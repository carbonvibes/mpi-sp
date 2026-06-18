# Artifact Evaluation

This document describes how to reproduce the artifacts of SemSan for
the sake of evaluation.

## Machine Configuration

To replicate our artifacts, rely on a machine with the following
specifications:

- CPU Platform: AMD EPYC 9B45 (Turin) (x86_64)
- CPU Cores: 4 CPU, 8 threads
- Memory: 32 GB
- OS: `#1 SMP PREEMPT_DYNAMIC Debian 6.1.158-1 (2025-11-09)` (`uname -v`)

While the benchmark results *should* proportionally translate to other
machines, *exact* reproducibility of the results is only likely on the
same configuration.

In any case, an x86_64-Linux machine is required to reproduce the
results.

## Prerequisites

For the dependencies, refer to the [main README](../README.md). The
following sections of this document assume that a properly configured
Nix installation is present as described in the
[main README](../README.md).

For a host machine based on Ubuntu, follow these commands:

```bash
curl --proto '=https' --tlsv1.2 -sSf -L https://install.determinate.systems/nix | sh -s -- install
source /nix/var/nix/profiles/default/etc/profile.d/nix-daemon.sh
```

When the dependencies are present, enter the Nix development shell
with:

```bash
nix develop
```

Then, generate the BPF bindings:

```bash
just codegen # This might take up to 5 minutes
```

Once the BPF bindings are generated, build the SemSan CLI:

```bash
just build
```

## RQ1: Vulnerabilities Detected in the Wild (Sec 7.1)

Here, we run vulnerable versions of the software targets we found bugs
in with SemSan with appropriate configurations to discover said bugs.

### Arbitrary file truncation in Git (Ref 69)

To reproduce the arbitrary file truncation vulnerability in Git, run:

```bash
nix run .#artifact-eval.bugs.git-reproduce
```

This will open a terminal multiplexer with a vulnerable version of Git-
web as well as SemSan with the [corresponding config](/nix/packages/by-name/artifact-eval/bugs/git-reproduce/reproduce-config.yaml).

Request the following URL with cURL or a web browser to trigger the bug:

```bash
curl 'http://127.0.0.1:1234/?p=.git;a=blobdiff;f=*;hpb=--output=/tmp/pwned;hb=HEAD'
```

**Expected output:** In the SemSan output in the multiplexer, you should see:
`[gitweb.cgi:2994615] Canary triggered: detected disallowed substring "pwned" in arg 1 of syscall execve`

### Local Privilege Escalation in Docker (Ref 41)

To reproduce the LPE vulnerability in Docker, we first need to install the
old, vulnerable version of Docker:

```bash
sudo ./aux/install-old-docker.sh
```

Then, configure the system and Docker accordingly:

```bash
sudo ./aux/setup-vuln-docker.sh
```

To reproduce the attack, first, the contents of `/tmp/example` (root-owned)
can be verified:

```bash
sudo cat /tmp/example
```

Now, the attack can be launched:

```bash
nix run .#artifact-eval.bugs.docker-reproduce
```

This will open a multiplexer similar as for the reproduction case above.

**Expected output:** In the SemSan output in the multiplexer, you should see:
`[dockerd:986503] dirownership open_without_o_nofollow example`

### Arbitrary File Truncation in Soft-Serve (Ref 54)

To reproduce the arbitrary file truncation vulnerability in Soft-Serve, we first
need to install the old, vulnerable version of Soft-Serve:

```bash
sudo ./aux/install-old-soft-serve.sh
```

Then, run the reproduction:

```bash
nix run .#artifact-eval.bugs.soft-serve-reproduce
```

This will open a terminal multiplexer with Soft-Serve v0.9.1, an attack script
that creates an `icecream` repository and triggers the bug with
`repo commit icecream -- --output=/tmp/pwned`, and SemSan with the
[corresponding config](/nix/packages/by-name/artifact-eval/bugs/soft-serve-reproduce/reproduce-config.yaml).

**Expected output:** In the SemSan output in the multiplexer, you should see:
`[git:988393] Canary triggered: detected disallowed substring "pwned" in arg 1 of syscall openat`

### Authorization Bypass in ViewVC (Ref 55)

To reproduce the authorization bypass in ViewVC, run:

```bash
nix run .#artifact-eval.bugs.viewvc-reproduce
```

This will open a terminal multiplexer with a vulnerable version of ViewVC
as well as SemSan with the [corresponding config](/nix/packages/by-name/artifact-eval/bugs/viewvc-reproduce/reproduce-config.yaml).

Request the following URL with cURL or a web browser to trigger the bug:

```bash
curl --max-time 10 --http0.9 'http://127.0.0.1:49152/viewvc/unprivileged/..%2fprivileged/secret.txt'
```

Recent cURL versions reject the old response format emitted by this ViewVC
standalone server unless `--http0.9` is passed. The request may time out after
SemSan terminates the vulnerable helper process; this is fine as long as the
SemSan finding below appears.

**Expected output:** In the SemSan output in the multiplexer, you should see:
`[rcs:3037580] Canary triggered: detected disallowed substring "secret.txt" in arg 1 of syscall openat`

## RQ2: Detection Accuracy (Sec 7.2)

This experiment stresses the capability of SemSan to detect existing bugs. For
the sake of the artifact evaluation, we provide experiments for positive errors.
As our false positive analysis required months, we only describe how to install
the tool at system level.

### True Positive Detection

To simplify the detection of true positive, we prepared a suite of program
samples that trigger a certain sanitizer, one that's triggering sanitization and
one that doesn't. The tests are contained in the [test](/test/) directory.

To exercise the test suite, run the [test entrypoint](/test/main_test.go)
with:

```bash
# Assuming you're still in the Nix shell with `nix develop`
just test
```

**Expected output:** Once finished, you should be presented with
passing tests for all built-in sanitizer primitives:

```text
Running Integration Tests
=== RUN   TestSyscallFilter/trigger
Attaching syscallfilter...
    client.go:45: all attached
    client.go:51: running
--- PASS: TestSyscallFilter (0.02s)
    --- PASS: TestSyscallFilter/benign (0.02s)
    --- PASS: TestSyscallFilter/trigger (0.00s)
# ... omitted for brevity
PASS
```

### System Level Installation

To detect potential false-positives, we employed SemSan in real-world
machines (development workstations, servers) over the course of multiple
months.

After SemSan is built as outlined in the [main README](../README.md),
it can be installed with the default, ["one-size-fits-all" config](../defaultConfig.yaml)
through a systemd service. First, copy the binary and configuration to
system-wide locations:

```bash
sudo install -Dm755 ./semsan-cli /usr/local/bin/semsan-cli
sudo install -Dm644 ./defaultConfig.yaml /etc/semsan/config.yaml
```

Then create `/etc/systemd/system/semsan.service` with the following
contents:

```ini
[Unit]
Description=SemanticSanitizer
After=network.target

[Service]
Type=simple
User=root
WorkingDirectory=/etc/semsan
ExecStart=/usr/local/bin/semsan-cli attach --config /etc/semsan/config.yaml
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

This can then be enabled with:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now semsan.service
```

This causes SemSan to be started with the system until you choose
disable it with an equivalent `systemctl disable` command.

To confirm that SemSan is active, inspect the service state and logs:

```bash
sudo systemctl status semsan.service
sudo journalctl -u semsan.service -f
```

**Expected output:** the journal should eventually contain `All sanitizers are
running`, after which SemSan remains attached in the background until the
service is stopped.

## RQ3: Performance Overhead (Sec 7.3)

### Micro-Benchmark

From the repository root, run the micro-benchmark with:

```bash
nix run .#artifact-eval.micro-benchmark
```

This may take around 20 minutes.

**Expected Output:** Once finished, you should be presented with a
table similar to table 5 in the paper:

```text
Benchmark                    Med w/o SemSan      Med w/ SemSan     Overhead %
---------                   ---------------      -------------     ----------
benchmark-general                3229168.00         3032532.00           6.09
benchmark-symlinkmount            499868.00          494137.00           1.15
benchmark-dirownership          27593849.00        25398242.00           7.96
benchmark-canary                 3228323.00         2956623.00           8.42
```

### Macro-Benchmark

From the repository root, run the macro-benchmark with:

```bash
nix run .#artifact-eval.macro-benchmark
```

This may take around 1 hour.

**Expected Output:** Once finished, you should be presented with a table similar
to this:

```text
Benchmark            Med w/o SemSan      Med w/ SemSan     Overhead %
pgbench                        2.14               2.23          -4.21
apache                     44292.93           44907.80          -1.39
```

## RQ4: Fuzzing Campaign (Sec 7.4)

This experiment runs a grammar-guided fuzzing campaign against **crun** (an
OCI container runtime) using a LibAFL/Nautilus fuzzer. SemSan is attached in
parallel to surface semantic violations that wouldnot manifest as crashes
otherwise.

The [fuzzer](case-studies/oci/fuzzer/) executes crun via the AFL++ forkserver protocol.
Nautilus uses the grammar [grammar](case-studies/oci/grammar.py) to generate
structurally valid OCI `config.json` inputs. The crun source is patched with a
fuzzing harness ([case-studies/oci/0001-crun-add-harness.patch](case-studies/oci/0001-crun-add-harness.patch))
that wraps `libcrun_container_{create,run,kill}` in an `__AFL_LOOP`.

### Run

The campaign must run as root because crun creates Linux namespaces and
cgroups. It can be run with:

```bash
nix run .#artifact-eval.crun-campaign
```

This opens a multiplexer that shows the output of both SemSan, showing
any potential sanitizations made, as well as those of the LibAFL-based
fuzzer, showing current coverage, execs/sec, etc.
Furthermore, it has a `tail` following the statistics which are regularly
written by the fuzzer, showing more detailed edge metrics, for example.

The multiplexer can be navigated with both mouse and keyboard.

**Expected Output:** The fuzzer should show increasing coverage (`shared_mem`) like so:

```text
[UserStats #0] run time: 4s, clients: 1, corpus: 76, objectives: 0, executions: 6985, exec/sec: 1.633k, shared_mem: 1361/11200 (12%)
[Testcase #0] run time: 4s, clients: 1, corpus: 77, objectives: 0, executions: 6985, exec/sec: 1.633k, shared_mem: 1361/11200 (12%)
[UserStats #0] run time: 4s, clients: 1, corpus: 77, objectives: 0, executions: 7001, exec/sec: 1.634k, shared_mem: 1366/11200 (12%)
[Testcase #0] run time: 4s, clients: 1, corpus: 78, objectives: 0, executions: 7001, exec/sec: 1.634k, shared_mem: 1366/11200 (12%)
[UserStats #0] run time: 5s, clients: 1, corpus: 78, objectives: 0, executions: 8715, exec/sec: 1.612k, shared_mem: 1366/11200 (12%)
[Testcase #0] run time: 5s, clients: 1, corpus: 79, objectives: 0, executions: 8715, exec/sec: 1.612k, shared_mem: 1366/11200 (12%)
[UserStats #0] run time: 6s, clients: 1, corpus: 79, objectives: 0, executions: 10008, exec/sec: 1.599k, shared_mem: 1367/11200 (12%)
[Testcase #0] run time: 6s, clients: 1, corpus: 80, objectives: 0, executions: 10008, exec/sec: 1.599k, shared_mem: 1367/11200 (12%)
[UserStats #0] run time: 10s, clients: 1, corpus: 80, objectives: 0, executions: 16288, exec/sec: 1.590k, shared_mem: 1368/11200 (12%)
[Testcase #0] run time: 10s, clients: 1, corpus: 81, objectives: 0, executions: 16288, exec/sec: 1.590k, shared_mem: 1368/11200 (12%)
[UserStats #0] run time: 10s, clients: 1, corpus: 81, objectives: 0, executions: 16305, exec/sec: 1.590k, shared_mem: 1368/11200 (12%)
```

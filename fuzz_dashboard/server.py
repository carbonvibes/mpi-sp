#!/usr/bin/env python3
"""
Fuzz Dashboard — live web UI for fuzz_libafl / fuzz_runc campaigns.
Usage:
    python3 server.py [log_file] [campaign] [port]
    python3 server.py /tmp/runc_fuzz.log runc
    python3 server.py /tmp/foobar_fuzz.log foobar
    python3 server.py /tmp/runc_fuzz.log runc 8081
"""

import http.server, json, re, os, sys, glob, time, threading, socket
from pathlib import Path

LOG_FILE  = sys.argv[1] if len(sys.argv) > 1 else "/tmp/runc_fuzz.log"
CAMPAIGN  = sys.argv[2] if len(sys.argv) > 2 else "runc"
PORT      = int(sys.argv[3]) if len(sys.argv) > 3 else 8080

CAMPAIGN_DIR  = Path(f"/tmp/{CAMPAIGN}")
CORPUS_DIR    = CAMPAIGN_DIR / "corpus"
SOLUTIONS_DIR = CAMPAIGN_DIR / "crashes"

# Campaign comparison directories and labels
COMPARE_DIRS = {
    1: Path("/tmp/campaign1"),
    2: Path("/tmp/campaign2"),
    3: Path("/tmp/campaign3"),
}
COMPARE_NAMES = {
    1: "Config-only (Grammar)",
    2: "Rootfs-only",
    3: "Combined",
}

# ── Log parser ──────────────────────────────────────────────────────────────

HEARTBEAT_RE = re.compile(
    r'\[(?:Client Heartbeat|UserStats|Testcase) #\d+\]'
    r' run time: ([\w\-m]+s?),'
    r' clients: \d+,'
    r' corpus: (\d+),'
    r' objectives: (\d+),'
    r' executions: ([\d,]+),'
    r' exec/sec: ([\d.k]+),'
    r' (?:edges|shared_mem): (\d+)/(\d+)'
)

def parse_exec_sec(s):
    if s.endswith('k'):
        return float(s[:-1]) * 1000
    return float(s)

def parse_executions(s):
    return int(s.replace(',', ''))

def parse_log(path):
    history   = []   # list of {t, execs, corpus, objectives, edges, edge_total, exec_sec}
    log_lines = []
    campaign_name = CAMPAIGN
    fuse_mount    = None

    try:
        with open(path, 'r', errors='replace') as f:
            for line in f:
                line = line.rstrip()
                log_lines.append(line)

                # fuzz_libafl prints  campaign=runc
                # fuzz_runc   prints  target=runc
                m = re.search(r'(?:campaign|target)=(\w+)', line)
                if m:
                    campaign_name = m.group(1)

                m = re.search(r'FUSE mounted at (.+)', line)
                if m:
                    fuse_mount = m.group(1).strip()

                m = HEARTBEAT_RE.search(line)
                if m:
                    history.append({
                        "runtime":    m.group(1),
                        "corpus":     int(m.group(2)),
                        "objectives": int(m.group(3)),
                        "execs":      parse_executions(m.group(4)),
                        "exec_sec":   parse_exec_sec(m.group(5)),
                        "edges":      int(m.group(6)),
                        "edge_total": int(m.group(7)),
                    })
    except FileNotFoundError:
        pass

    latest = history[-1] if history else {}
    return {
        "campaign":     campaign_name,
        "fuse_mount":   fuse_mount,
        "latest":       latest,
        "history":      history[-120:],   # last 120 heartbeats for chart
        "log_tail":     log_lines[-60:],  # last 60 lines
    }

def scan_corpus(directory, max_entries=80, decode_json=False):
    entries = []
    try:
        files = [p for p in Path(directory).iterdir()
                 if p.is_file() and not p.name.startswith('.')]
        # most-recently-modified first so the dashboard shows the freshest entries
        files.sort(key=lambda p: p.stat().st_mtime, reverse=True)
        for p in files[:max_entries]:
            stat = p.stat()
            delta = None
            if decode_json and p.suffix == '.json':
                try:
                    delta = json.loads(p.read_text(errors='replace'))
                except Exception:
                    pass
            entries.append({
                "name":  p.name,
                "bytes": stat.st_size,
                "mtime": stat.st_mtime,
                "delta": delta,
            })
    except FileNotFoundError:
        pass
    return entries

def get_state():
    log       = parse_log(LOG_FILE)
    corpus    = scan_corpus(CORPUS_DIR,    decode_json=True)
    solutions = scan_corpus(SOLUTIONS_DIR, decode_json=True)

    # Fallback: if log parsing found no heartbeat data, read from fuzzer_stats + plot_data.
    # This handles cases where the log file is missing, the heartbeat format changed,
    # or the fuzzer was restarted and the log was truncated.
    if not log["latest"]:
        stats = parse_fuzzer_stats(CAMPAIGN_DIR / "fuzzer_stats")
        if stats.get("execs_done"):
            execs_done   = int(stats.get("execs_done",   0) or 0)
            run_time_s   = int(stats.get("run_time",     1) or 1)
            edges_found  = int(stats.get("edges_found",  0) or 0)
            total_edges  = int(stats.get("total_edges",  11200) or 11200)
            corpus_count = int(stats.get("corpus_count", 0) or 0)
            crashes      = int(stats.get("saved_crashes",0) or 0)
            h = run_time_s // 3600
            m = (run_time_s % 3600) // 60
            s = run_time_s % 60
            runtime_str  = f"{h}h-{m:02d}m-{s:02d}s" if h else f"{m}m-{s:02d}s"
            log["latest"] = {
                "runtime":    runtime_str,
                "corpus":     corpus_count,
                "objectives": crashes,
                "execs":      execs_done,
                "exec_sec":   execs_done / max(run_time_s, 1),
                "edges":      edges_found,
                "edge_total": total_edges,
                "_source":    "fuzzer_stats",
            }

    # Populate chart history from plot_data if log heartbeats are missing.
    if not log["history"]:
        series = parse_plot_data(CAMPAIGN_DIR / "plot_data")
        if series:
            def _runtime_str(t):
                t = int(t)
                h = t // 3600; m = (t % 3600) // 60; s = t % 60
                return f"{h}h-{m:02d}m" if h else f"{m}m-{s:02d}s"
            log["history"] = [
                {
                    "runtime":    _runtime_str(pt["t"]),
                    "corpus":     pt["corpus"],
                    "objectives": pt["crashes"],
                    "execs":      int(pt["exec_sec"] * pt["t"]),
                    "exec_sec":   pt["exec_sec"],
                    "edges":      pt["edges"],
                    "edge_total": 11200,
                    "_source":    "plot_data",
                }
                for pt in series[-120:]
            ]

    return {
        **log,
        "corpus_files":    corpus,
        "solutions_files": solutions,
        "log_file":        LOG_FILE,
        "corpus_dir":      str(CORPUS_DIR),
        "solutions_dir":   str(SOLUTIONS_DIR),
        "ts":              time.time(),
    }

# ── Campaign comparison ──────────────────────────────────────────────────────

def parse_fuzzer_stats(path):
    stats = {}
    try:
        with open(path) as f:
            for line in f:
                if ':' in line:
                    key, _, val = line.partition(':')
                    stats[key.strip()] = val.strip()
    except FileNotFoundError:
        pass
    return stats

def parse_plot_data(path, total_edges=11200):
    """Parse LibAFL AflStatsStage plot_data file.

    LibAFL format (NOT standard AFL++ format):
      relative_time, cycles_done, cur_item, corpus_count, pending_total,
      pending_favs, total_edges, saved_crashes, saved_hangs, max_depth,
      execs_done(*), execs_done, edges_found
    (*) LibAFL writes execs_done into both col 10 and col 11 — NOT a rate.
    True exec/sec = execs_done / relative_time.
    """
    series = []
    try:
        with open(path) as f:
            for line in f:
                line = line.strip()
                if not line or line.startswith('#'):
                    continue
                parts = [p.strip() for p in line.split(',')]
                if len(parts) < 13:
                    continue
                try:
                    t          = float(parts[0])
                    corpus     = int(parts[3])
                    crashes    = int(parts[7])
                    execs_done = float(parts[11])
                    edges      = int(parts[12])
                    exec_sec   = execs_done / t if t > 0 else 0.0
                    series.append({"t": t, "edges": edges, "corpus": corpus,
                                   "exec_sec": exec_sec, "crashes": crashes})
                except (ValueError, IndexError):
                    pass
    except FileNotFoundError:
        pass
    # If the fuzzer was restarted, relative_time resets to near-zero mid-file.
    # Find the last reset point and keep only the latest run segment.
    if len(series) > 1:
        last_reset = 0
        for i in range(1, len(series)):
            if series[i]["t"] < series[i - 1]["t"]:
                last_reset = i
        if last_reset:
            series = series[last_reset:]

    return series

def get_compare():
    campaigns = []
    for cid, cdir in COMPARE_DIRS.items():
        stats       = parse_fuzzer_stats(cdir / "fuzzer_stats")
        total_edges = int(stats.get("total_edges", 11200) or 11200)
        series      = parse_plot_data(cdir / "plot_data", total_edges)
        campaigns.append({
            "id":          cid,
            "name":        COMPARE_NAMES[cid],
            "running":     (cdir / "fuzzer_stats").exists(),
            "edges_found": int(stats.get("edges_found", 0) or 0),
            "total_edges": total_edges,
            "run_time":    int(stats.get("run_time", 0) or 0),
            "execs_done":  int(stats.get("execs_done", 0) or 0),
            # LibAFL writes execs_done into execs_per_sec — compute real rate manually
            "exec_sec":    (int(stats.get("execs_done", 0) or 0) /
                            max(int(stats.get("run_time", 1) or 1), 1)),
            "corpus_count":int(stats.get("corpus_count", 0) or 0),
            "crashes":     int(stats.get("saved_crashes", 0) or 0),
            "series":      series,
        })
    return {"campaigns": campaigns}

# ── HTTP handler ─────────────────────────────────────────────────────────────

HTML = open(Path(__file__).parent / "index.html").read()

class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_): pass   # suppress access log noise

    def do_GET(self):
        if self.path == "/api/state":
            data = json.dumps(get_state()).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Access-Control-Allow-Origin", "*")
            self.end_headers()
            self.wfile.write(data)
        elif self.path == "/api/compare":
            data = json.dumps(get_compare()).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Access-Control-Allow-Origin", "*")
            self.end_headers()
            self.wfile.write(data)
        else:
            body = HTML.encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.end_headers()
            self.wfile.write(body)

if __name__ == "__main__":
    print(f"Fuzz Dashboard — http://localhost:{PORT}")
    print(f"  campaign : {CAMPAIGN}")
    print(f"  log file : {LOG_FILE}")
    print(f"  corpus   : {CORPUS_DIR}")
    print(f"  solutions: {SOLUTIONS_DIR}")
    # SO_REUSEADDR: lets us restart immediately without 'Address already in use'.
    # We must set the socket option before bind(), hence overriding server_bind.
    import socket as _socket
    class ReusableHTTPServer(http.server.HTTPServer):
        def server_bind(self):
            self.socket.setsockopt(_socket.SOL_SOCKET, _socket.SO_REUSEADDR, 1)
            super().server_bind()
    ReusableHTTPServer(("", PORT), Handler).serve_forever()


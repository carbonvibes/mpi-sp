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

MUTATOR_ROOT = Path(__file__).parent.parent / "mutator"
CORPUS_DIR   = MUTATOR_ROOT / f"corpus_{CAMPAIGN}"
SOLUTIONS_DIR= MUTATOR_ROOT / f"solutions_{CAMPAIGN}"

# ── Log parser ──────────────────────────────────────────────────────────────

HEARTBEAT_RE = re.compile(
    r'\[(?:Client Heartbeat|UserStats|Testcase) #\d+\]'
    r' run time: ([\w\-m]+s?),'
    r' clients: \d+,'
    r' corpus: (\d+),'
    r' objectives: (\d+),'
    r' executions: ([\d,]+),'
    r' exec/sec: ([\d.k]+),'
    r' edges: (\d+)/(\d+)'
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
    return {
        **log,
        "corpus_files":    corpus,
        "solutions_files": solutions,
        "log_file":        LOG_FILE,
        "corpus_dir":      str(CORPUS_DIR),
        "solutions_dir":   str(SOLUTIONS_DIR),
        "ts":              time.time(),
    }

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


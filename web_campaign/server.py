#!/usr/bin/env python3
"""
Parallel Campaign Dashboard — serves live data for 6 concurrent fuzzer instances.
Usage:
    python3 server.py [port]
    python3 server.py 8090
"""

import http.server, json, os, sys, time
from pathlib import Path

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8090

CAMPAIGNS = [
    {"id": "c1_0", "label": "C1-inst-0", "dir": Path("/tmp/c1_0"), "color": "#58a6ff"},
    {"id": "c1_1", "label": "C1-inst-1", "dir": Path("/tmp/c1_1"), "color": "#79c0ff"},
    {"id": "c1_2", "label": "C1-inst-2", "dir": Path("/tmp/c1_2"), "color": "#a5d6ff"},
    {"id": "c3_0", "label": "C3-inst-0", "dir": Path("/tmp/c3_0"), "color": "#ffa657"},
    {"id": "c3_1", "label": "C3-inst-1", "dir": Path("/tmp/c3_1"), "color": "#ffb74d"},
    {"id": "c3_2", "label": "C3-inst-2", "dir": Path("/tmp/c3_2"), "color": "#ffd180"},
]

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

def parse_plot_data(path):
    """Parse LibAFL AflStatsStage plot_data.
    Columns: relative_time, cycles_done, cur_item, corpus_count, pending_total,
             pending_favs, total_edges, saved_crashes, saved_hangs, max_depth,
             execs_done(*), execs_done, edges_found
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

    # If fuzzer restarted, relative_time resets — keep only latest run segment
    if len(series) > 1:
        last_reset = 0
        for i in range(1, len(series)):
            if series[i]["t"] < series[i - 1]["t"]:
                last_reset = i
        if last_reset:
            series = series[last_reset:]

    # Downsample evenly across the full time range (not just the tail)
    n = 400
    if len(series) <= n:
        return series
    step = (len(series) - 1) / (n - 1)
    indices = sorted(set(round(i * step) for i in range(n)))
    return [series[i] for i in indices]

def get_campaign_data(c):
    stats  = parse_fuzzer_stats(c["dir"] / "fuzzer_stats")
    series = parse_plot_data(c["dir"] / "plot_data")

    execs_done = int(stats.get("execs_done", 0) or 0)
    run_time   = int(stats.get("run_time",   1) or 1)
    latest = {
        "execs":   execs_done,
        "exec_sec": execs_done / max(run_time, 1),
        "edges":   int(stats.get("edges_found",   0) or 0),
        "corpus":  int(stats.get("corpus_count",  0) or 0),
        "crashes": int(stats.get("saved_crashes", 0) or 0),
        "run_time": run_time,
    }
    stats_path = c["dir"] / "fuzzer_stats"
    try:
        alive = (time.time() - stats_path.stat().st_mtime) < 30
    except FileNotFoundError:
        alive = False

    return {
        "id":      c["id"],
        "label":   c["label"],
        "color":   c["color"],
        "running": alive,
        "latest":  latest,
        "series":  series,
    }

def get_data():
    return {"campaigns": [get_campaign_data(c) for c in CAMPAIGNS], "ts": time.time()}

HTML = open(Path(__file__).parent / "index.html").read()

class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_): pass

    def do_GET(self):
        if self.path == "/api/data":
            body = json.dumps(get_data()).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Access-Control-Allow-Origin", "*")
            self.end_headers()
            self.wfile.write(body)
        else:
            body = HTML.encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.end_headers()
            self.wfile.write(body)

if __name__ == "__main__":
    import socket as _socket
    class ReusableHTTPServer(http.server.HTTPServer):
        def server_bind(self):
            self.socket.setsockopt(_socket.SOL_SOCKET, _socket.SO_REUSEADDR, 1)
            super().server_bind()

    print(f"Parallel Campaign Dashboard — http://localhost:{PORT}")
    for c in CAMPAIGNS:
        print(f"  {c['label']:12s} -> {c['dir']}")
    ReusableHTTPServer(("", PORT), Handler).serve_forever()

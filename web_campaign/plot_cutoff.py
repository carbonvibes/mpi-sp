#!/usr/bin/env python3
"""
Plot a parallel fuzzing campaign up to a time cutoff (default 20 h).
Usage:
    python3 plot_cutoff.py <dir1> [dir2 ...] <output.png> [--hours H]

Reads each dir's plot_data (written by AflStatsStage); does not modify it,
so it is safe to run against a live campaign.
"""
import sys
import os
import numpy as np
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt

FALLBACK_COLORS = ["#d62728", "#e85555", "#f0a0a0", "#1f77b4", "#4a9fd4", "#85c1e9"]

CUTOFF_HOURS = 20.0
args = sys.argv[1:]
if "--hours" in args:
    i = args.index("--hours")
    CUTOFF_HOURS = float(args[i + 1])
    del args[i:i + 2]

if len(args) < 2:
    print(f"Usage: {sys.argv[0]} <campaign_dir> [campaign_dir ...] <output.png> [--hours H]")
    sys.exit(1)

dirs   = args[:-1]
outpng = args[-1]


def load_plot_data(path, cutoff_h):
    times, corpus, execs, edges = [], [], [], []
    try:
        with open(path) as f:
            for line in f:
                line = line.strip()
                if not line or line.startswith('#'):
                    continue
                cols = line.split(',')
                if len(cols) < 13:
                    continue
                try:
                    times.append(float(cols[0]))
                    corpus.append(int(cols[3]))
                    execs.append(float(cols[11]))
                    edges.append(int(cols[12]))
                except ValueError:
                    continue
    except FileNotFoundError:
        return None
    if not times:
        return None

    t = np.array(times)
    corpus = np.array(corpus); execs = np.array(execs); edges = np.array(edges)

    # keep only the last run, drop pre-restart segments
    resets = np.where(np.diff(t) < 0)[0]
    if len(resets):
        s = resets[-1] + 1
        t, corpus, execs, edges = t[s:], corpus[s:], execs[s:], edges[s:]

    mask = t <= cutoff_h * 3600
    t, corpus, execs, edges = t[mask], corpus[mask], execs[mask], edges[mask]
    if len(t) == 0:
        return None

    exec_sec = np.where(t > 0, execs / t, 0.0)
    return {"hours": t / 3600, "corpus": corpus, "exec_sec": exec_sec, "edges": edges}


fig, axes = plt.subplots(3, 1, figsize=(12, 10))
fig.suptitle(f'Parallel Fuzzing — crun (C3, 6 instances) — first {CUTOFF_HOURS:g} h',
             fontsize=14, fontweight='bold')

plotted = 0
final_edges = []
for idx, d in enumerate(dirs):
    label = os.path.basename(d.rstrip('/'))
    data  = load_plot_data(os.path.join(d, 'plot_data'), CUTOFF_HOURS)
    if data is None:
        print(f"  [skip] no data <= {CUTOFF_HOURS}h in {d}")
        continue
    color = FALLBACK_COLORS[idx % len(FALLBACK_COLORS)]
    axes[0].plot(data["hours"], data["edges"],    color=color, label=label, linewidth=1.5)
    axes[1].plot(data["hours"], data["corpus"],   color=color, label=label, linewidth=1.5)
    axes[2].plot(data["hours"], data["exec_sec"], color=color, label=label, linewidth=1.5, alpha=0.85)
    plotted += 1
    final_edges.append(int(data["edges"][-1]))
    print(f"  [{label}] {len(data['hours'])} pts up to {data['hours'][-1]:.2f}h  "
          f"edges@cutoff={int(data['edges'][-1])}  max_corpus={int(data['corpus'].max())}")

if plotted == 0:
    print("No data found within cutoff — nothing to plot.")
    sys.exit(1)

if final_edges:
    print(f"  edges@{CUTOFF_HOURS:g}h across instances: "
          f"min={min(final_edges)} max={max(final_edges)} mean={np.mean(final_edges):.0f}")

axes[0].set_ylabel('Edges Found'); axes[0].set_title('Coverage Over Time'); axes[0].grid(True, alpha=0.3)
axes[1].set_ylabel('Corpus Size'); axes[1].set_title('Corpus Growth Over Time'); axes[1].grid(True, alpha=0.3)
axes[2].set_ylabel('Exec / sec');  axes[2].set_title('Execution Speed Over Time'); axes[2].grid(True, alpha=0.3)
for ax in axes:
    ax.set_xlabel('Time (hours)')
    ax.set_xlim(0, CUTOFF_HOURS)
    ax.legend(loc='upper left', bbox_to_anchor=(1.01, 1), borderaxespad=0, frameon=True, fontsize=9)

plt.tight_layout()
plt.savefig(outpng, dpi=150, bbox_inches='tight', facecolor='white')
print(f"Saved: {outpng}")

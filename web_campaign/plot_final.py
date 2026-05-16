#!/usr/bin/env python3
"""
Final plot generator for 6-instance parallel fuzzing campaigns.
Usage:
    python3 plot_final.py <dir1> [dir2 ...] <output.png>
The last argument is the output PNG path; all others are campaign dirs.
Each dir must contain a plot_data file written by AflStatsStage.
"""
import sys
import os
import numpy as np
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt

COLORS = {
    "c1_0": "#1f77b4",
    "c1_1": "#4a9fd4",
    "c1_2": "#85c1e9",
    "c3_0": "#d62728",
    "c3_1": "#e85555",
    "c3_2": "#f0a0a0",
}
FALLBACK_COLORS = ["#1f77b4", "#4a9fd4", "#85c1e9", "#d62728", "#e85555", "#f0a0a0"]

def load_plot_data(path):
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
        pass

    if not times:
        return None

    t = np.array(times)
    resets = np.where(np.diff(t) < 0)[0]
    if len(resets):
        start  = resets[-1] + 1
        t      = t[start:]
        corpus = corpus[start:]
        execs  = execs[start:]
        edges  = edges[start:]

    exec_sec = np.where(t > 0, np.array(execs) / t, 0.0)
    return {
        "hours":    t / 3600,
        "corpus":   np.array(corpus),
        "exec_sec": exec_sec,
        "edges":    np.array(edges),
    }

if len(sys.argv) < 3:
    print(f"Usage: {sys.argv[0]} <campaign_dir> [campaign_dir ...] <output.png>")
    sys.exit(1)

dirs   = sys.argv[1:-1]
outpng = sys.argv[-1]

fig, axes = plt.subplots(3, 1, figsize=(12, 10))
fig.suptitle('Parallel Fuzzing — Final Results', fontsize=14, fontweight='bold')

plotted = 0
for idx, d in enumerate(dirs):
    label = os.path.basename(d.rstrip('/'))
    path  = os.path.join(d, 'plot_data')
    data  = load_plot_data(path)
    if data is None:
        print(f"  [skip] no data in {path}")
        continue

    color = COLORS.get(label, FALLBACK_COLORS[idx % len(FALLBACK_COLORS)])
    axes[0].plot(data["hours"], data["edges"],    color=color, label=label, linewidth=1.5)
    axes[1].plot(data["hours"], data["corpus"],   color=color, label=label, linewidth=1.5)
    axes[2].plot(data["hours"], data["exec_sec"], color=color, label=label, linewidth=1.5, alpha=0.85)
    plotted += 1
    print(f"  [{label}] {len(data['hours'])} pts  "
          f"max edges={int(data['edges'].max())}  "
          f"max corpus={int(data['corpus'].max())}")

if plotted == 0:
    print("No data found in any campaign dir — nothing to plot.")
    sys.exit(1)

axes[0].set_ylabel('Edges Found')
axes[0].set_title('Coverage Over Time')
axes[0].grid(True, alpha=0.3)

axes[1].set_ylabel('Corpus Size')
axes[1].set_title('Corpus Growth Over Time')
axes[1].grid(True, alpha=0.3)

axes[2].set_ylabel('Exec / sec')
axes[2].set_title('Execution Speed Over Time')
axes[2].grid(True, alpha=0.3)

for ax in axes:
    ax.set_xlabel('Time (hours)')
    ax.legend(loc='upper left', bbox_to_anchor=(1.01, 1),
              borderaxespad=0, frameon=True, fontsize=9)

plt.tight_layout()
plt.savefig(outpng, dpi=150, bbox_inches='tight', facecolor='white')
print(f"Saved: {outpng}")

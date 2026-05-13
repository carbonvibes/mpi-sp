#!/usr/bin/env python3
import sys
import numpy as np
import matplotlib.pyplot as plt
import matplotlib.ticker as ticker

def load_plot_data(path):
    times, corpus, edges, execs = [], [], [], []
    with open(path) as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith('#'):
                continue
            cols = line.split(',')
            if len(cols) < 13:
                continue
            try:
                times.append(int(cols[0]))
                corpus.append(int(cols[3]))
                execs.append(int(cols[11]))
                edges.append(int(cols[12]))
            except ValueError:
                continue
    return np.array(times), np.array(corpus), np.array(execs), np.array(edges)

def compute_exec_per_sec(times, execs):
    rates = np.zeros(len(times))
    for i in range(1, len(times)):
        dt = times[i] - times[i-1]
        de = execs[i] - execs[i-1]
        rates[i] = de / dt if dt > 0 else 0
    rates[0] = execs[0] / times[0] if times[0] > 0 else 0
    return rates

if len(sys.argv) < 2:
    print(f"Usage: {sys.argv[0]} <plot_data_file> [label] [plot_data2 label2 ...]")
    sys.exit(1)

# Parse arguments: file1 label1 file2 label2 ...
campaigns = []
i = 1
while i < len(sys.argv):
    path = sys.argv[i]
    label = sys.argv[i+1] if i+1 < len(sys.argv) and not sys.argv[i+1].endswith('.plot_data') else f"Campaign {len(campaigns)+1}"
    campaigns.append((path, label))
    i += 2

fig, axes = plt.subplots(3, 1, figsize=(12, 10))
fig.suptitle('Fuzzing Campaign Comparison', fontsize=14, fontweight='bold')

colors = ['#1f77b4', '#ff7f0e', '#2ca02c', '#d62728']

for idx, (path, label) in enumerate(campaigns):
    times, corpus, execs, edges = load_plot_data(path)
    if len(times) == 0:
        print(f"No data in {path}")
        continue
    hours = times / 3600
    rates = compute_exec_per_sec(times, execs)
    c = colors[idx % len(colors)]

    axes[0].plot(hours, edges, color=c, label=label, linewidth=1.5)
    axes[1].plot(hours, corpus, color=c, label=label, linewidth=1.5)
    axes[2].plot(hours, rates, color=c, label=label, linewidth=1.5, alpha=0.8)

axes[0].set_ylabel('Edges Found')
axes[0].set_title('Coverage Over Time')
axes[0].legend()
axes[0].grid(True, alpha=0.3)

axes[1].set_ylabel('Corpus Size')
axes[1].set_title('Corpus Growth Over Time')
axes[1].legend()
axes[1].grid(True, alpha=0.3)

axes[2].set_ylabel('Exec/sec')
axes[2].set_title('Execution Speed Over Time')
axes[2].legend()
axes[2].grid(True, alpha=0.3)

for ax in axes:
    ax.set_xlabel('Time (hours)')

plt.tight_layout()
out = 'campaign_plots.png'
plt.savefig(out, dpi=150, bbox_inches='tight')
print(f"Saved: {out}")

import matplotlib.pyplot as plt
import numpy as np

plt.rcParams.update({'font.size': 14})

unsanitized_data = [120.882, 120.542, 127.804, 122.149, 116.504, 127.317, 118.413, 121.103, 117.654]
sanitized_data = [92.644, 120.156, 113.687, 120.494, 120.293, 124.405, 109.422, 122.096, 120.349]

median_unsanitized = np.median(unsanitized_data)
median_sanitized = np.median(sanitized_data)
difference_percentage = ((median_unsanitized - median_sanitized) / median_unsanitized) * 100

print(f"Median Unsanitized: {median_unsanitized}")
print(f"Median Sanitized: {median_sanitized}")
print(f"Difference Percentage: {difference_percentage:.2f}%")

plt.figure(figsize=(8, 6))
box_data = [unsanitized_data, sanitized_data]
labels = ["Unsanitized", "Sanitized"]

plt.boxplot(box_data, labels=labels)

plt.ylabel("Average Latency (ms)")

plt.grid(True, alpha=0.3)

plt.tight_layout()
plt.savefig('boxplot_pgbench_benchmark.pdf', dpi=300, bbox_inches='tight')
plt.show()

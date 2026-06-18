import matplotlib.pyplot as plt
import numpy as np

plt.rcParams.update({'font.size': 14})

unsanitized_data = [115406.06, 112764.52, 112643.68]
sanitized_data = [112653.61, 113311.1, 112616.27]

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

plt.ylabel("Requests per Second (RPS)")

plt.grid(True, alpha=0.3)

plt.tight_layout()
plt.savefig('boxplot_apache_benchmark.pdf', dpi=300, bbox_inches='tight')
plt.show()

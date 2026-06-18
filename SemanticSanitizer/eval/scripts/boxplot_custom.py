import matplotlib.pyplot as plt
import numpy as np

plt.rcParams.update({'font.size': 14})

unsanitized_data = [9786214, 9969080, 9904737]
sanitized_data = [9820067, 9762954, 9859666]
unsanitized_data = [x / 60 for x in unsanitized_data]
sanitized_data = [x / 60 for x in sanitized_data]

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

plt.ylabel("Open-Close-Unlink Cycles per Second")

plt.grid(True, alpha=0.3)

plt.tight_layout()
plt.savefig('boxplot_syscall_benchmark.pdf', dpi=300, bbox_inches='tight')
plt.show()

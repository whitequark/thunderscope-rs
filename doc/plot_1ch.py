import sys
import numpy as np
import matplotlib.pyplot as plt

plt.figure()
plt.plot(np.fromfile(sys.argv[1], dtype=np.int8))
plt.xlabel('Sample Index')
plt.ylabel('ADC Code')
plt.grid(True)
plt.show()

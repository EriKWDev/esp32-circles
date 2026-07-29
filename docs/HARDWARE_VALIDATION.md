# Hardware validation

The repository currently contains PMU device documentation but no product-level
schematic. The restructure therefore preserves existing pin assignments and
peripheral initialization sequences.

CI compilation can establish:

- dependency resolution;
- header and API compatibility;
- target and partition configuration;
- link success;
- creation of a complete flash bundle.

CI compilation cannot establish:

- that GPIO, I2C, SPI, I2S, MIPI/DBI, or interrupt pins match the PCB;
- display orientation, color order, timing, or brightness;
- touch transform and gesture behavior;
- microphone/speaker routing and analog performance;
- charging, battery measurement, or sleep-current behavior;
- RF, storage, thermal, or power integrity.

Before a release, validate the affected surface on hardware and record the board
revision, example or firmware, artifact commit, observed result, and any
measurement equipment used. If a product schematic is added later, audit the
shared BSP and example pin tables against it before claiming hardware parity.

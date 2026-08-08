# Hardware validation

[简体中文](HARDWARE_VALIDATION_ZH.md)

The repository does not contain a product-level schematic, PCB source, or board
revision record. An [official schematic](https://files.waveshare.com/wiki/ESP32-C6-Touch-AMOLED-2.16/ESP32-C6-Touch-AMOLED-2.16-Schematic.pdf)
is available externally, but this repository audit did not re-derive or change
hardware mappings from it. Existing pin assignments and peripheral
initialization sequences remain preserved.

## What CI can establish

CI compilation can establish:

- dependency resolution;
- header and API compatibility for the tested matrix;
- ESP32-C6 target and partition configuration;
- link success;
- creation of a complete example flash bundle.

## What CI cannot establish

CI compilation cannot establish:

- that GPIO, I2C, SPI, I2S, MIPI/DBI, or interrupt pins match a specific PCB
  revision;
- display orientation, color order, timing, or brightness;
- touch transform and gesture behavior;
- microphone/speaker routing and analog performance;
- charging, battery measurement, or sleep-current behavior;
- RF, storage, thermal, or power integrity;
- the suitability of a preserved factory or XiaoZhi recovery binary.

The example configuration proves 16 MB flash selection. It does not configure
or validate PSRAM, and examples that expose a choice explicitly use the
without-PSRAM path.

## Release evidence

Before a release, validate every affected surface on hardware and record the
board revision, example or firmware, artifact commit, observed result, and any
measurement equipment used. If pins or peripheral timing change, cite the exact
schematic revision and relevant page or net. Mark untested surfaces explicitly;
do not turn a source-level comparison into a hardware-parity claim.

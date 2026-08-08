# Examples

[简体中文](README_ZH.md)

The repository provides parallel learning paths for ESP-IDF and Arduino:

- [`esp-idf/`](esp-idf/README.md) contains nine independent projects validated
  against the exact maintained ESP-IDF versions in CI.
- [`arduino/examples/`](arduino/README.md) contains nine independent sketches
  validated with the repository's pinned Arduino-ESP32 core.

The numeric prefixes order examples from focused peripheral checks to display,
audio, and UI integration. They do not express build dependencies. Bundled
library examples are upstream material and are not discovered as product
examples.

Every product example targets ESP32-C6 and assumes the repository's documented
16 MB flash layout. Do not infer PSRAM or physical peripheral validation from a
successful compile.

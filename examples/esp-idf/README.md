# ESP-IDF examples

Each child directory is a self-contained ESP-IDF project targeting ESP32-C6.
Projects use managed components for suitable upstream drivers and the shared
repository component in `components/xpowers` for common PMU code.

Build one project from its own directory:

```sh
idf.py set-target esp32c6
idf.py build
idf.py flash monitor
```

CI discovers projects by requiring both `CMakeLists.txt` and a `main/`
directory. A new example that follows that shape is automatically added to the
matrix.

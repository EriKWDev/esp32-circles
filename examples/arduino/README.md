# Arduino examples

First-party sketches are in `examples/`; pinned library snapshots are in
`libraries/`.

The CI build script supplies XPowersLib to all sketches. It supplies LVGL 8 only
to `08_LVGL_V8_Test` and LVGL 9 only to `07_Audio_Test` and
`09_LVGL_V9_Test`, preventing incompatible major versions from being visible to
the compiler at the same time.

For Arduino IDE setup, see the repository root README and the product wiki.

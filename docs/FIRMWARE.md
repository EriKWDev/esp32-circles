# Firmware artifacts

Source snapshots and recovery images are intentionally separate:

- `firmware/xiaozhi/` is a preserved source snapshot.
- `firmware/factory_firmware/` contains prebuilt recovery images.
- neither firmware path is modified, built, packaged, or validated by repository CI.

Use recovery images only with the flash offsets and process documented on the
product wiki. CI artifacts cover the example projects, not these firmware paths.

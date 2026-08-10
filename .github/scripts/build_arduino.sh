#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 <sketch-dir> <build-dir> <output-dir>" >&2
  exit 2
fi

sketch_dir="$1"
build_dir="$2"
output_dir="$3"
sketch_name="$(basename "$sketch_dir")"
libraries_dir="$(mktemp -d)"
trap 'rm -rf "$libraries_dir"' EXIT

cp -R examples/arduino/libraries/XPowersLib "$libraries_dir/XPowersLib"
cp -R examples/arduino/libraries/C6_AMOLED_BSP "$libraries_dir/C6_AMOLED_BSP"

case "$sketch_name" in
  08_LVGL_V8_Test)
    cp -R examples/arduino/libraries/lvgl8/lvgl "$libraries_dir/lvgl"
    cp examples/arduino/libraries/lvgl8/lv_conf.h "$libraries_dir/lv_conf.h"
    ;;
  01_AXP2101_Test|02_I2C_QMI8658|03_I2C_PCF85063|04_SD_Card|05_WIFI_STA|06_WIFI_AP|07_Audio_Test|09_LVGL_V9_Test)
    cp -R examples/arduino/libraries/lvgl9/lvgl "$libraries_dir/lvgl"
    cp examples/arduino/libraries/lvgl9/lv_conf.h "$libraries_dir/lv_conf.h"
    ;;
esac

fqbn="esp32:esp32:esp32c6:CDCOnBoot=cdc,PartitionScheme=app3M_fat9M_16MB,CPUFreq=160,FlashMode=qio,FlashFreq=80,FlashSize=16M,UploadSpeed=921600,DebugLevel=none,EraseFlash=none,JTAGAdapter=builtin,ZigbeeMode=default"

arduino-cli compile \
  --fqbn "$fqbn" \
  --libraries "$libraries_dir" \
  --build-path "$build_dir" \
  --output-dir "$output_dir" \
  "$sketch_dir"

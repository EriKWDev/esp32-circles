#include <stdio.h>

#include "esp_log.h"

#include "bsp/esp-bsp.h"
#include "status_ui.h"

namespace {

const char *card_type(const sdmmc_card_t *card)
{
    if (card == nullptr) {
        return "unavailable";
    }
    if (card->is_sdio && card->is_mem) {
        return "SD combo";
    }
    if (card->is_sdio) {
        return "SDIO";
    }
    return card->is_mem ? "memory card" : "unknown";
}

void publish_sdcard_status(esp_err_t mount_result)
{
    status_ui_snapshot_t ui = {};
    snprintf(ui.title, sizeof(ui.title), "SD Card");
    snprintf(ui.lines[3], sizeof(ui.lines[3]), "Mount path: %s", BSP_SD_MOUNT_POINT);

    if (mount_result != ESP_OK || bsp_sdcard == nullptr) {
        ui.level = STATUS_UI_LEVEL_ERROR;
        snprintf(ui.lines[0], sizeof(ui.lines[0]), "SD init/mount: failed");
        snprintf(ui.lines[1], sizeof(ui.lines[1]), "Card: unavailable");
        snprintf(ui.lines[2], sizeof(ui.lines[2]), "Error: %s", esp_err_to_name(mount_result));
    } else {
        const uint64_t capacity_bytes =
            bsp_sdcard->csd.capacity * (uint64_t)bsp_sdcard->csd.sector_size;
        ui.level = STATUS_UI_LEVEL_OK;
        snprintf(ui.lines[0], sizeof(ui.lines[0]), "SD init/mount: ready");
        snprintf(ui.lines[1], sizeof(ui.lines[1]), "Card type: %s", card_type(bsp_sdcard));
        snprintf(ui.lines[2], sizeof(ui.lines[2]), "Capacity: %llu MiB",
                 (unsigned long long)(capacity_bytes / (1024ULL * 1024ULL)));
    }
    (void)status_ui_publish(&ui);
}

} // namespace

extern "C" void app_main(void)
{
    ESP_ERROR_CHECK(bsp_pmu_init());
    const esp_err_t ui_result = status_ui_init();
    if (ui_result != ESP_OK) {
        ESP_LOGE("SD", "Status UI unavailable: %s", esp_err_to_name(ui_result));
    }
    const esp_err_t mount_result = bsp_sdcard_mount();
    if (mount_result != ESP_OK) {
        ESP_LOGE("SD", "SD card mount failed: %s", esp_err_to_name(mount_result));
    }
    publish_sdcard_status(mount_result);
}

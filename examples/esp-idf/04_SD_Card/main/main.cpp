
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <freertos/FreeRTOS.h>
#include <esp_log.h>

#include "i2c_bsp.h"
#include "sdcard_bsp.h"
#include "power_bsp.h"
#include "status_ui.h"
#include "user_config.h"

I2cMasterBus I2cMasterBus_(BSP_I2C_SCL,BSP_I2C_SDA,BSP_I2C_NUM);
CustomSDPort *CustomSDPort_ = NULL;
static const char *const kSdMountPath = "/sdcard";

static esp_err_t status_ui_panel_power_reset(void *) {
    Axp2101_SetAldo3(1);
    vTaskDelay(pdMS_TO_TICKS(100));
    Axp2101_SetAldo3(0);
    vTaskDelay(pdMS_TO_TICKS(100));
    Axp2101_SetAldo3(1);
    vTaskDelay(pdMS_TO_TICKS(100));
    return ESP_OK;
}

static void publish_sdcard_status() {
    status_ui_snapshot_t ui = {};
    sdcard_snapshot_t sd = {};
    const esp_err_t snapshot_err = CustomSDPort_->SDPort_GetSnapshot(&sd);

    snprintf(ui.title, sizeof(ui.title), "SD Card");
    snprintf(ui.lines[3], sizeof(ui.lines[3]), "Mount path: %s", kSdMountPath);
    if (snapshot_err != ESP_OK || !sd.mounted) {
        ui.level = STATUS_UI_LEVEL_ERROR;
        snprintf(ui.lines[0], sizeof(ui.lines[0]), "SD init/mount: failed");
        snprintf(ui.lines[1], sizeof(ui.lines[1]), "Card: %s", sd.card_present ? "present" : "unavailable");
        snprintf(ui.lines[2], sizeof(ui.lines[2]), "Card type: %s", sd.card_type);
    } else {
        ui.level = STATUS_UI_LEVEL_OK;
        snprintf(ui.lines[0], sizeof(ui.lines[0]), "SD init/mount: ready");
        snprintf(ui.lines[1], sizeof(ui.lines[1]), "Card type: %s", sd.card_type);
        snprintf(ui.lines[2], sizeof(ui.lines[2]), "Capacity: %llu MiB",
                 (unsigned long long)(sd.capacity_bytes / (1024ULL * 1024ULL)));
    }
    status_ui_publish(&ui);
}

extern "C" void app_main(void) {
    Custom_PmicPortInit(&I2cMasterBus_, 0x34);

    status_ui_config_t ui_config = {};
    ui_config.panel_power_reset = status_ui_panel_power_reset;
    ui_config.panel_power_reset_context = NULL;
    const esp_err_t ui_err = status_ui_init(&ui_config);
    if (ui_err != ESP_OK) {
        ESP_LOGE("main", "Status UI unavailable: %s", esp_err_to_name(ui_err));
    }

    CustomSDPort_ = new CustomSDPort(kSdMountPath);
    publish_sdcard_status();

}

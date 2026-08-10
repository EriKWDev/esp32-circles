
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <freertos/FreeRTOS.h>
#include <esp_log.h>

#include "i2c_bsp.h"
#include "power_bsp.h"
#include "status_ui.h"
#include "user_config.h"

I2cMasterBus I2cMasterBus_(BSP_I2C_SCL,BSP_I2C_SDA,BSP_I2C_NUM);
static bool s_status_ui_ready = false;

static esp_err_t status_ui_panel_power_reset(void *) {
    Axp2101_SetAldo3(1);
    vTaskDelay(pdMS_TO_TICKS(100));
    Axp2101_SetAldo3(0);
    vTaskDelay(pdMS_TO_TICKS(100));
    Axp2101_SetAldo3(1);
    vTaskDelay(pdMS_TO_TICKS(100));
    return ESP_OK;
}

static void AXP2101_StatusUiTask(void *) {
    while (true) {
        axp2101_snapshot_t power = {};
        status_ui_snapshot_t ui = {};
        snprintf(ui.title, sizeof(ui.title), "AXP2101 Power");
        if (Axp2101_GetSnapshot(&power) == ESP_OK) {
            ui.level = power.charging ? STATUS_UI_LEVEL_OK : STATUS_UI_LEVEL_INFO;
            snprintf(ui.lines[0], sizeof(ui.lines[0]), "PMU init: ready");
            snprintf(ui.lines[1], sizeof(ui.lines[1]), "Display: ready");
            snprintf(ui.lines[2], sizeof(ui.lines[2]), "Charging: %s", power.charging ? "yes" : "no");
            snprintf(ui.lines[3], sizeof(ui.lines[3]), "Charger: %s", Axp2101_ChargerStatusText(power.charger_status));
            snprintf(ui.lines[4], sizeof(ui.lines[4]), "Battery: %u mV", (unsigned)power.battery_mv);
        } else {
            ui.level = STATUS_UI_LEVEL_ERROR;
            snprintf(ui.lines[0], sizeof(ui.lines[0]), "PMU status read failed");
        }
        status_ui_publish(&ui);
        vTaskDelay(pdMS_TO_TICKS(2000));
    }
}

extern "C" void app_main(void) {
    esp_err_t pmic_err = Custom_PmicPortInit(&I2cMasterBus_, 0x34);
    if (pmic_err == ESP_OK) {
        xTaskCreatePinnedToCore(Axp2101_isChargingTask, "Axp2101_isChargingTask", 3 * 1024, NULL, 3, NULL, 0);
        status_ui_config_t ui_config = {};
        ui_config.panel_power_reset = status_ui_panel_power_reset;
        ui_config.panel_power_reset_context = NULL;
        esp_err_t ui_err = status_ui_init(&ui_config);
        if (ui_err == ESP_OK) {
            s_status_ui_ready = true;
        } else {
            ESP_LOGE("main", "Status UI unavailable: %s", esp_err_to_name(ui_err));
        }
    } else {
        ESP_LOGE("main", "PMU initialization failed: %s", esp_err_to_name(pmic_err));
    }

    if (s_status_ui_ready) {
        xTaskCreatePinnedToCore(AXP2101_StatusUiTask, "AXP2101_StatusUiTask", 3 * 1024, NULL, 3, NULL, 0);
    }
}

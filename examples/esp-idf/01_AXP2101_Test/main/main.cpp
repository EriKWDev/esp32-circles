#include <stdio.h>

#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "esp_log.h"

#include "bsp/esp-bsp.h"
#include "status_ui.h"

namespace {

static const char *const kTag = "AXP2101";

void axp2101_status_task(void *)
{
    while (true) {
        bsp_pmu_snapshot_t power = {};
        status_ui_snapshot_t ui = {};
        snprintf(ui.title, sizeof(ui.title), "AXP2101 Power");

        const esp_err_t err = bsp_pmu_get_snapshot(&power);
        if (err == ESP_OK) {
            ui.level = power.charging ? STATUS_UI_LEVEL_OK : STATUS_UI_LEVEL_INFO;
            snprintf(ui.lines[0], sizeof(ui.lines[0]), "PMU init: ready");
            snprintf(ui.lines[1], sizeof(ui.lines[1]), "Display: ready");
            snprintf(ui.lines[2], sizeof(ui.lines[2]), "Charging: %s", power.charging ? "yes" : "no");
            snprintf(ui.lines[3], sizeof(ui.lines[3]), "Charger: %s",
                     bsp_pmu_charger_status_to_string(power.charger_status));
            snprintf(ui.lines[4], sizeof(ui.lines[4]), "Battery: %u mV", (unsigned)power.battery_mv);
            ESP_LOGI(kTag, "charging=%s status=%s battery=%u mV",
                     power.charging ? "yes" : "no",
                     bsp_pmu_charger_status_to_string(power.charger_status),
                     (unsigned)power.battery_mv);
        } else {
            ui.level = STATUS_UI_LEVEL_ERROR;
            snprintf(ui.lines[0], sizeof(ui.lines[0]), "PMU status read failed");
            ESP_LOGE(kTag, "PMU status read failed: %s", esp_err_to_name(err));
        }
        (void)status_ui_publish(&ui);
        vTaskDelay(pdMS_TO_TICKS(2000));
    }
}

} // namespace

extern "C" void app_main(void)
{
    ESP_ERROR_CHECK(bsp_pmu_init());
    ESP_ERROR_CHECK(status_ui_init());
    xTaskCreatePinnedToCore(axp2101_status_task, "axp2101_status", 3 * 1024,
                            nullptr, 3, nullptr, 0);
}

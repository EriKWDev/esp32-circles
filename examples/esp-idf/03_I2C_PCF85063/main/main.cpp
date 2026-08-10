#include <stdio.h>

#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "esp_log.h"

#include "bsp/esp-bsp.h"
#include "pcf85063a.h"
#include "status_ui.h"

namespace {

pcf85063a_dev_t s_rtc;
char s_log_buffer[40] = {};

void publish_rtc_status(status_ui_level_t level, const pcf85063a_datetime_t *datetime)
{
    status_ui_snapshot_t ui = {};
    ui.level = level;
    snprintf(ui.title, sizeof(ui.title), "PCF85063 RTC");
    if (datetime == nullptr) {
        snprintf(ui.lines[0], sizeof(ui.lines[0]), "RTC error/read failed");
    } else {
        snprintf(ui.lines[0], sizeof(ui.lines[0]), "RTC init: ready");
        snprintf(ui.lines[1], sizeof(ui.lines[1]), "Date: %04d-%02d-%02d",
                 datetime->year, datetime->month, datetime->day);
        snprintf(ui.lines[2], sizeof(ui.lines[2]), "Time: %02d:%02d:%02d",
                 datetime->hour, datetime->min, datetime->sec);
        snprintf(ui.lines[3], sizeof(ui.lines[3]), "Source: PCF85063");
    }
    (void)status_ui_publish(&ui);
}

void pcf85063_task(void *)
{
    while (true) {
        pcf85063a_datetime_t datetime = {};
        const esp_err_t ret = pcf85063a_get_time_date(&s_rtc, &datetime);
        if (ret == ESP_OK) {
            snprintf(s_log_buffer, sizeof(s_log_buffer), "%04d/%02d/%02d %02d:%02d:%02d",
                     datetime.year, datetime.month, datetime.day,
                     datetime.hour, datetime.min, datetime.sec);
            ESP_LOGW("RTC", "%s", s_log_buffer);
            publish_rtc_status(STATUS_UI_LEVEL_OK, &datetime);
        } else {
            ESP_LOGW("RTC", "Failed to read PCF85063: %d", ret);
            publish_rtc_status(STATUS_UI_LEVEL_ERROR, nullptr);
        }
        vTaskDelay(pdMS_TO_TICKS(1000));
    }
}

} // namespace

extern "C" void app_main(void)
{
    ESP_ERROR_CHECK(bsp_pmu_init());
    ESP_ERROR_CHECK(status_ui_init());

    i2c_master_bus_handle_t i2c = bsp_i2c_get_handle();
    if (i2c == nullptr) {
        ESP_LOGE("pcf85063", "Managed BSP I2C bus is unavailable");
        publish_rtc_status(STATUS_UI_LEVEL_ERROR, nullptr);
        return;
    }

    esp_err_t ret = pcf85063a_init(&s_rtc, i2c, BSP_RTC_I2C_ADDRESS);
    if (ret != ESP_OK) {
        ESP_LOGE("pcf85063", "Failed to initialize PCF85063 (error: %d)", ret);
        publish_rtc_status(STATUS_UI_LEVEL_ERROR, nullptr);
        return;
    }

    pcf85063a_datetime_t datetime = {};
    datetime.year = 2026;
    datetime.month = 1;
    datetime.day = 1;
    datetime.hour = 8;
    ESP_ERROR_CHECK(pcf85063a_set_time_date(&s_rtc, datetime));
    xTaskCreatePinnedToCore(pcf85063_task, "pcf85063", 3 * 1024, nullptr, 3, nullptr, 0);
}

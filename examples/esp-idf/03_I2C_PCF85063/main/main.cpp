
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <freertos/FreeRTOS.h>
#include <esp_log.h>

#include "i2c_bsp.h"
#include "power_bsp.h"
#include "status_ui.h"
#include "user_config.h"
#include "pcf85063a.h"

I2cMasterBus I2cMasterBus_(BSP_I2C_SCL,BSP_I2C_SDA,BSP_I2C_NUM);
static pcf85063a_dev_t pcf85063;
static char LvglDataBuff[40] = {""};

static esp_err_t status_ui_panel_power_reset(void *) {
    Axp2101_SetAldo3(1);
    vTaskDelay(pdMS_TO_TICKS(100));
    Axp2101_SetAldo3(0);
    vTaskDelay(pdMS_TO_TICKS(100));
    Axp2101_SetAldo3(1);
    vTaskDelay(pdMS_TO_TICKS(100));
    return ESP_OK;
}

static void publish_rtc_status(status_ui_level_t level, const pcf85063a_datetime_t *datetime) {
    status_ui_snapshot_t ui = {};
    ui.level = level;
    snprintf(ui.title, sizeof(ui.title), "PCF85063 RTC");
    if (datetime == NULL) {
        snprintf(ui.lines[0], sizeof(ui.lines[0]), "RTC error/read failed");
    } else {
        snprintf(ui.lines[0], sizeof(ui.lines[0]), "RTC init: ready");
        snprintf(ui.lines[1], sizeof(ui.lines[1]), "Date: %04d-%02d-%02d", datetime->year, datetime->month, datetime->day);
        snprintf(ui.lines[2], sizeof(ui.lines[2]), "Time: %02d:%02d:%02d", datetime->hour, datetime->min, datetime->sec);
        snprintf(ui.lines[3], sizeof(ui.lines[3]), "Source: PCF85063");
    }
    status_ui_publish(&ui);
}

void PCF85063_Task(void *arg) {
	while(1) {
        pcf85063a_datetime_t datatime = {};
        esp_err_t ret = pcf85063a_get_time_date(&pcf85063, &datatime);
        if (ret == ESP_OK) {
            pcf85063a_datetime_to_str(LvglDataBuff, sizeof(LvglDataBuff), datatime);
            snprintf(LvglDataBuff, sizeof(LvglDataBuff), "%04d/%02d/%02d %02d:%02d:%02d",\
            datatime.year, datatime.month,datatime.day, datatime.hour, datatime.min, datatime.sec);
            ESP_LOGW("RTC","%s", LvglDataBuff);
            publish_rtc_status(STATUS_UI_LEVEL_OK, &datatime);
        } else {
            ESP_LOGW("RTC", "Failed to read PCF85063: %d", ret);
            publish_rtc_status(STATUS_UI_LEVEL_ERROR, NULL);
        }
        vTaskDelay(pdMS_TO_TICKS(1000));
    }
}

extern "C" void app_main(void) {
    Custom_PmicPortInit(&I2cMasterBus_,0x34);
    status_ui_config_t ui_config = {};
    ui_config.panel_power_reset = status_ui_panel_power_reset;
    ui_config.panel_power_reset_context = NULL;
    esp_err_t ui_err = status_ui_init(&ui_config);
    if (ui_err != ESP_OK) {
        ESP_LOGE("main", "Status UI unavailable: %s", esp_err_to_name(ui_err));
    }
    esp_err_t ret = pcf85063a_init(&pcf85063, I2cMasterBus_.Get_I2cBusHandle(), PCF85063A_ADDRESS);
    if (ret != ESP_OK) {
        ESP_LOGE("pcf85063", "Failed to initialize PCF85063 (error: %d)", ret);
        publish_rtc_status(STATUS_UI_LEVEL_ERROR, NULL);
    } else {
        pcf85063a_datetime_t datatime = {};
        datatime.year = 2026;
        datatime.month = 1;
        datatime.day = 1;
        datatime.hour = 8;
        datatime.min = 0;
        datatime.sec = 0;
        pcf85063a_set_time_date(&pcf85063, datatime);
		xTaskCreatePinnedToCore(PCF85063_Task, "PCF85063_Task", 3 * 1024, NULL, 3, NULL,0);
    }
}


#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <freertos/FreeRTOS.h>
#include <esp_log.h>

#include "i2c_bsp.h"
#include "power_bsp.h"
#include "status_ui.h"
#include "user_config.h"
#include "qmi8658.h"

I2cMasterBus I2cMasterBus_(BSP_I2C_SCL,BSP_I2C_SDA,BSP_I2C_NUM);
static qmi8658_dev_t qmi8658;
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

static void publish_qmi_status(status_ui_level_t level, const qmi8658_data_t *data) {
    status_ui_snapshot_t ui = {};
    ui.level = level;
    snprintf(ui.title, sizeof(ui.title), "QMI8658 Motion");
    if (data == NULL) {
        snprintf(ui.lines[0], sizeof(ui.lines[0]), "Sensor error/read failed");
    } else {
        snprintf(ui.lines[0], sizeof(ui.lines[0]), "Acc X: %.2f m/s2", data->accelX);
        snprintf(ui.lines[1], sizeof(ui.lines[1]), "Acc Y: %.2f m/s2", data->accelY);
        snprintf(ui.lines[2], sizeof(ui.lines[2]), "Acc Z: %.2f m/s2", data->accelZ);
        snprintf(ui.lines[3], sizeof(ui.lines[3]), "Gyro X: %.2f rad/s", data->gyroX);
        snprintf(ui.lines[4], sizeof(ui.lines[4]), "Gyro Y/Z: %.2f / %.2f rad/s", data->gyroY, data->gyroZ);
    }
    status_ui_publish(&ui);
}

void QMI8658_Task(void *arg) {
	while(1) {
        bool ready;
        int ret = qmi8658_is_data_ready(&qmi8658, &ready);
        if(ret == ESP_OK && ready) {
            qmi8658_data_t qmidata = {};
            ret = qmi8658_read_sensor_data(&qmi8658, &qmidata);
            if(ret == ESP_OK) {
                snprintf(LvglDataBuff, sizeof(LvglDataBuff), "Acc(m/s2):%.2f,%.2f,%.2f",\
                qmidata.accelX, qmidata.accelY, qmidata.accelZ);
                ESP_LOGW("acc","%s", LvglDataBuff);
                snprintf(LvglDataBuff, sizeof(LvglDataBuff), "Gyro(rad/s):%.2f,%.2f,%.2f",\
                qmidata.gyroX, qmidata.gyroY, qmidata.gyroZ);
                ESP_LOGW("gyro","%s", LvglDataBuff);
                publish_qmi_status(STATUS_UI_LEVEL_OK, &qmidata);
            } else {
                ESP_LOGW("qmi8658", "Sensor read failed: %d", ret);
                publish_qmi_status(STATUS_UI_LEVEL_ERROR, NULL);
            }
        } else if (ret != ESP_OK) {
            ESP_LOGW("qmi8658", "Data-ready check failed: %d", ret);
            publish_qmi_status(STATUS_UI_LEVEL_ERROR, NULL);
        } else {
            status_ui_snapshot_t ui = {};
            ui.level = STATUS_UI_LEVEL_WARNING;
            snprintf(ui.title, sizeof(ui.title), "QMI8658 Motion");
            snprintf(ui.lines[0], sizeof(ui.lines[0]), "Sensor: data not ready");
            status_ui_publish(&ui);
        }
        vTaskDelay(pdMS_TO_TICKS(500));
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
    esp_err_t ret = qmi8658_init(&qmi8658, I2cMasterBus_.Get_I2cBusHandle(), QMI8658_ADDRESS_HIGH);
    if (ret != ESP_OK) {
        ESP_LOGE("qmi8658", "Failed to initialize QMI8658 (error: %d)", ret);
        publish_qmi_status(STATUS_UI_LEVEL_ERROR, NULL);
    } else {
        qmi8658_set_accel_range(&qmi8658, QMI8658_ACCEL_RANGE_8G);
        qmi8658_set_accel_odr(&qmi8658, QMI8658_ACCEL_ODR_1000HZ);
        qmi8658_set_gyro_range(&qmi8658, QMI8658_GYRO_RANGE_512DPS);
        qmi8658_set_gyro_odr(&qmi8658, QMI8658_GYRO_ODR_1000HZ);
        qmi8658_set_accel_unit_mps2(&qmi8658, true);
        qmi8658_set_gyro_unit_rads(&qmi8658, true);
        qmi8658_set_display_precision(&qmi8658, 4);
		xTaskCreatePinnedToCore(QMI8658_Task, "QMI8658_Task", 3 * 1024, NULL, 3, NULL,0);
    }
}

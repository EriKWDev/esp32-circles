#include <stdio.h>

#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "esp_log.h"

#include "bsp/esp-bsp.h"
#include "qmi8658.h"
#include "status_ui.h"

namespace {

qmi8658_dev_t s_qmi8658;
char s_log_buffer[96] = {};

void publish_qmi_status(status_ui_level_t level, const qmi8658_data_t *data)
{
    status_ui_snapshot_t ui = {};
    ui.level = level;
    snprintf(ui.title, sizeof(ui.title), "QMI8658 Motion");
    if (data == nullptr) {
        snprintf(ui.lines[0], sizeof(ui.lines[0]), "Sensor error/read failed");
    } else {
        snprintf(ui.lines[0], sizeof(ui.lines[0]), "Acc X: %.2f m/s2", data->accelX);
        snprintf(ui.lines[1], sizeof(ui.lines[1]), "Acc Y: %.2f m/s2", data->accelY);
        snprintf(ui.lines[2], sizeof(ui.lines[2]), "Acc Z: %.2f m/s2", data->accelZ);
        snprintf(ui.lines[3], sizeof(ui.lines[3]), "Gyro X: %.2f rad/s", data->gyroX);
        snprintf(ui.lines[4], sizeof(ui.lines[4]), "Gyro Y/Z: %.2f / %.2f rad/s", data->gyroY, data->gyroZ);
    }
    (void)status_ui_publish(&ui);
}

void qmi8658_task(void *)
{
    while (true) {
        bool ready = false;
        int ret = qmi8658_is_data_ready(&s_qmi8658, &ready);
        if (ret == ESP_OK && ready) {
            qmi8658_data_t data = {};
            ret = qmi8658_read_sensor_data(&s_qmi8658, &data);
            if (ret == ESP_OK) {
                snprintf(s_log_buffer, sizeof(s_log_buffer), "Acc(m/s2):%.2f,%.2f,%.2f",
                         data.accelX, data.accelY, data.accelZ);
                ESP_LOGW("acc", "%s", s_log_buffer);
                snprintf(s_log_buffer, sizeof(s_log_buffer), "Gyro(rad/s):%.2f,%.2f,%.2f",
                         data.gyroX, data.gyroY, data.gyroZ);
                ESP_LOGW("gyro", "%s", s_log_buffer);
                publish_qmi_status(STATUS_UI_LEVEL_OK, &data);
            } else {
                ESP_LOGW("qmi8658", "Sensor read failed: %d", ret);
                publish_qmi_status(STATUS_UI_LEVEL_ERROR, nullptr);
            }
        } else if (ret != ESP_OK) {
            ESP_LOGW("qmi8658", "Data-ready check failed: %d", ret);
            publish_qmi_status(STATUS_UI_LEVEL_ERROR, nullptr);
        } else {
            status_ui_snapshot_t ui = {};
            ui.level = STATUS_UI_LEVEL_WARNING;
            snprintf(ui.title, sizeof(ui.title), "QMI8658 Motion");
            snprintf(ui.lines[0], sizeof(ui.lines[0]), "Sensor: data not ready");
            (void)status_ui_publish(&ui);
        }
        vTaskDelay(pdMS_TO_TICKS(500));
    }
}

} // namespace

extern "C" void app_main(void)
{
    ESP_ERROR_CHECK(bsp_pmu_init());
    ESP_ERROR_CHECK(status_ui_init());

    i2c_master_bus_handle_t i2c = bsp_i2c_get_handle();
    if (i2c == nullptr) {
        ESP_LOGE("qmi8658", "Managed BSP I2C bus is unavailable");
        publish_qmi_status(STATUS_UI_LEVEL_ERROR, nullptr);
        return;
    }

    esp_err_t ret = qmi8658_init(&s_qmi8658, i2c, BSP_IMU_I2C_ADDRESS);
    if (ret != ESP_OK) {
        ESP_LOGE("qmi8658", "Failed to initialize QMI8658 (error: %d)", ret);
        publish_qmi_status(STATUS_UI_LEVEL_ERROR, nullptr);
        return;
    }
    ESP_ERROR_CHECK(qmi8658_set_accel_range(&s_qmi8658, QMI8658_ACCEL_RANGE_8G));
    ESP_ERROR_CHECK(qmi8658_set_accel_odr(&s_qmi8658, QMI8658_ACCEL_ODR_1000HZ));
    ESP_ERROR_CHECK(qmi8658_set_gyro_range(&s_qmi8658, QMI8658_GYRO_RANGE_512DPS));
    ESP_ERROR_CHECK(qmi8658_set_gyro_odr(&s_qmi8658, QMI8658_GYRO_ODR_1000HZ));
    qmi8658_set_accel_unit_mps2(&s_qmi8658, true);
    qmi8658_set_gyro_unit_rads(&s_qmi8658, true);
    qmi8658_set_display_precision(&s_qmi8658, 4);
    xTaskCreatePinnedToCore(qmi8658_task, "qmi8658", 3 * 1024, nullptr, 3, nullptr, 0);
}

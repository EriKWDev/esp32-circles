#include <C6AmoledBsp.h>
#include "qmi8658.h"

static c6_amoled::Board board;
static c6_amoled::StatusPage status_page;
static qmi8658_dev_t qmi8658;
static bool qmi_ready = false;

static void publish_status(const qmi8658_data_t *data) {
  c6_amoled::StatusPageData status = {};
  snprintf(status.title, sizeof(status.title), "QMI8658 IMU");
  snprintf(status.lines[0], sizeof(status.lines[0]), "PMIC: %s", board.pmicReady() ? "ready" : "failed");
  if (data) {
    snprintf(status.lines[1], sizeof(status.lines[1]), "Acc X: %.2f m/s2", data->accelX);
    snprintf(status.lines[2], sizeof(status.lines[2]), "Acc Y: %.2f  Z: %.2f", data->accelY, data->accelZ);
    snprintf(status.lines[3], sizeof(status.lines[3]), "Gyro X: %.2f rad/s", data->gyroX);
    snprintf(status.lines[4], sizeof(status.lines[4]), "Gyro Y: %.2f Z: %.2f", data->gyroY, data->gyroZ);
  } else {
    snprintf(status.lines[1], sizeof(status.lines[1]), "QMI8658 init/read failed");
  }
  status_page.publish(status);
}

void setup() {
  Serial.begin(115200);
  delay(2000);
  board.begin();
  if (!status_page.begin(board)) Serial.println("Status UI unavailable; IMU test continues.");
  qmi_ready = qmi8658_init(&qmi8658, board.i2c().Get_I2cBusHandle(), QMI8658_ADDRESS_HIGH) == ESP_OK;
  if (!qmi_ready) {
    Serial.println("Failed to initialize QMI8658");
    publish_status(NULL);
    return;
  }
  qmi8658_set_accel_range(&qmi8658, QMI8658_ACCEL_RANGE_8G);
  qmi8658_set_accel_odr(&qmi8658, QMI8658_ACCEL_ODR_1000HZ);
  qmi8658_set_gyro_range(&qmi8658, QMI8658_GYRO_RANGE_512DPS);
  qmi8658_set_gyro_odr(&qmi8658, QMI8658_GYRO_ODR_1000HZ);
  qmi8658_set_accel_unit_mps2(&qmi8658, true);
  qmi8658_set_gyro_unit_rads(&qmi8658, true);
}

void loop() {
  qmi8658_data_t data = {};
  bool ready = false;
  if (qmi_ready && qmi8658_is_data_ready(&qmi8658, &ready) == ESP_OK && ready && qmi8658_read_sensor_data(&qmi8658, &data) == ESP_OK) {
    Serial.printf("Acc: %.2f, %.2f, %.2f; Gyro: %.2f, %.2f, %.2f\n", data.accelX, data.accelY, data.accelZ, data.gyroX, data.gyroY, data.gyroZ);
    publish_status(&data);
  } else {
    publish_status(NULL);
  }
  delay(500);
}

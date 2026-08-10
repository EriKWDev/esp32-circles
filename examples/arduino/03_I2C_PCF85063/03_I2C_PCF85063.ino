#include <C6AmoledBsp.h>
#include "pcf85063a.h"

static c6_amoled::Board board;
static c6_amoled::StatusPage status_page;
static pcf85063a_dev_t pcf85063;
static bool rtc_ready = false;

static void publish_status(const pcf85063a_datetime_t *time) {
  c6_amoled::StatusPageData status = {};
  snprintf(status.title, sizeof(status.title), "PCF85063A RTC");
  snprintf(status.lines[0], sizeof(status.lines[0]), "PMIC: %s", board.pmicReady() ? "ready" : "failed");
  if (time) {
    snprintf(status.lines[1], sizeof(status.lines[1]), "Date: %04u/%02u/%02u", time->year, time->month, time->day);
    snprintf(status.lines[2], sizeof(status.lines[2]), "Time: %02u:%02u:%02u", time->hour, time->min, time->sec);
  } else {
    snprintf(status.lines[1], sizeof(status.lines[1]), "RTC init/read failed");
  }
  status_page.publish(status);
}

void setup() {
  Serial.begin(115200);
  delay(2000);
  board.begin();
  if (!status_page.begin(board)) Serial.println("Status UI unavailable; RTC test continues.");
  rtc_ready = pcf85063a_init(&pcf85063, board.i2c().Get_I2cBusHandle(), PCF85063A_ADDRESS) == ESP_OK;
  if (!rtc_ready) {
    Serial.println("Failed to initialize PCF85063A");
    return;
  }
  pcf85063a_datetime_t initial_time = {
      .year = 2026, .month = 1, .day = 1, .hour = 8, .min = 0, .sec = 0};
  esp_err_t set_err = pcf85063a_set_time_date(&pcf85063, initial_time);
  if (set_err == ESP_OK) {
    Serial.println("RTC set to 2026/01/01 08:00:00");
  } else {
    Serial.printf("Failed to set RTC time: %s\n", esp_err_to_name(set_err));
  }
}

void loop() {
  pcf85063a_datetime_t time = {};
  if (rtc_ready && pcf85063a_get_time_date(&pcf85063, &time) == ESP_OK) {
    Serial.printf("RTC: %04u/%02u/%02u %02u:%02u:%02u\n", time.year, time.month, time.day, time.hour, time.min, time.sec);
    publish_status(&time);
  } else {
    publish_status(NULL);
  }
  delay(1000);
}

#include <Arduino.h>
#include <C6AmoledBsp.h>
#include "lvgl.h"
#include "demos/lv_demos.h"

static c6_amoled::Board board;
static bool display_ready = false;

#define Brightness_Test_EN  1

void setup() {
  Serial.begin(115200);
  delay(2000);
  Serial.println("start");
  board.begin();
  display_ready = board.beginDisplay(true);
  if (display_ready && c6_amoled::lvglLock()) {
    Serial.println("start ui");
    lv_demo_widgets();
    c6_amoled::lvglUnlock();
  } else if (!display_ready) {
    Serial.println("Display unavailable; LVGL demo not started.");
  }
}

uint8_t back = 100;

void loop() {
#if (Brightness_Test_EN == 1) 
  if (display_ready && c6_amoled::lvglLock()) {
    c6_amoled::setBacklight(back);
    c6_amoled::lvglUnlock();
  }
  vTaskDelay(pdMS_TO_TICKS(2000));
  if (back > 0) {
    back = back - 20;
  } else {
    back = 100;
  }
#endif
}

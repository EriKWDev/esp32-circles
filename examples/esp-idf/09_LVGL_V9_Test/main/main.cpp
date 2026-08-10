#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "esp_log.h"

#include "bsp/esp-bsp.h"
#include "lv_demos.h"
#include "lvgl.h"

namespace {

void brightness_task(void *)
{
    uint8_t brightness = 100;
    while (true) {
        if (bsp_display_lock(500)) {
            const esp_err_t err = bsp_display_brightness_set(brightness);
            bsp_display_unlock();
            if (err != ESP_OK) {
                ESP_LOGW("brightness", "Brightness update failed: %s", esp_err_to_name(err));
            }
        }
        vTaskDelay(pdMS_TO_TICKS(2000));
        brightness = brightness > 0 ? brightness - 20 : 100;
    }
}

} // namespace

extern "C" void app_main(void)
{
    if (bsp_display_start() == nullptr) {
        ESP_LOGE("main", "Managed BSP display initialization failed");
        return;
    }
    if (!bsp_display_lock(0)) {
        ESP_LOGE("main", "Managed BSP display lock failed");
        return;
    }
    lv_demo_widgets();
    bsp_display_unlock();
    xTaskCreatePinnedToCore(brightness_task, "brightness", 3 * 1024,
                            nullptr, 3, nullptr, 0);
}

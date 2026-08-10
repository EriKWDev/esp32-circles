#include "freertos/FreeRTOS.h"
#include "freertos/queue.h"
#include "freertos/task.h"
#include "esp_err.h"
#include "esp_log.h"
#include "lvgl.h"

#include "bsp/esp-bsp.h"
#include "status_ui.h"

namespace {

static const char *const kTag = "status_ui";

struct StatusUiState {
    QueueHandle_t queue;
    lv_obj_t *title;
    lv_obj_t *badge;
    lv_obj_t *lines[STATUS_UI_LINE_COUNT];
    bool initialized;
};

StatusUiState s_ui = {};

const char *level_text(status_ui_level_t level)
{
    switch (level) {
    case STATUS_UI_LEVEL_OK:
        return "READY";
    case STATUS_UI_LEVEL_WARNING:
        return "WAITING";
    case STATUS_UI_LEVEL_ERROR:
        return "ERROR";
    case STATUS_UI_LEVEL_INFO:
    default:
        return "INFO";
    }
}

lv_color_t level_color(status_ui_level_t level)
{
    switch (level) {
    case STATUS_UI_LEVEL_OK:
        return lv_color_hex(0x1B8F4B);
    case STATUS_UI_LEVEL_WARNING:
        return lv_color_hex(0xB7791F);
    case STATUS_UI_LEVEL_ERROR:
        return lv_color_hex(0xC53030);
    case STATUS_UI_LEVEL_INFO:
    default:
        return lv_color_hex(0x2563EB);
    }
}

void create_page()
{
    lv_obj_t *screen = lv_screen_active();
    lv_obj_set_style_bg_color(screen, lv_color_hex(0x101827), 0);
    lv_obj_set_style_bg_opa(screen, LV_OPA_COVER, 0);

    s_ui.title = lv_label_create(screen);
    lv_obj_set_pos(s_ui.title, 28, 28);
    lv_obj_set_width(s_ui.title, 300);
    lv_obj_set_style_text_color(s_ui.title, lv_color_white(), 0);

    s_ui.badge = lv_label_create(screen);
    lv_obj_set_pos(s_ui.badge, 342, 24);
    lv_obj_set_size(s_ui.badge, 110, 34);
    lv_obj_set_style_radius(s_ui.badge, 17, 0);
    lv_obj_set_style_pad_top(s_ui.badge, 8, 0);
    lv_obj_set_style_text_align(s_ui.badge, LV_TEXT_ALIGN_CENTER, 0);
    lv_obj_set_style_text_color(s_ui.badge, lv_color_white(), 0);
    lv_obj_set_style_bg_opa(s_ui.badge, LV_OPA_COVER, 0);

    lv_obj_t *card = lv_obj_create(screen);
    lv_obj_set_pos(card, 20, 88);
    lv_obj_set_size(card, 440, 300);
    lv_obj_set_style_radius(card, 16, 0);
    lv_obj_set_style_bg_color(card, lv_color_hex(0x1F2937), 0);
    lv_obj_set_style_border_width(card, 0, 0);
    lv_obj_clear_flag(card, LV_OBJ_FLAG_SCROLLABLE);

    for (size_t i = 0; i < STATUS_UI_LINE_COUNT; ++i) {
        s_ui.lines[i] = lv_label_create(card);
        lv_obj_set_pos(s_ui.lines[i], 22, 24 + static_cast<int>(i) * 50);
        lv_obj_set_width(s_ui.lines[i], 396);
        lv_obj_set_style_text_color(s_ui.lines[i], lv_color_hex(0xD1D5DB), 0);
        lv_label_set_long_mode(s_ui.lines[i], LV_LABEL_LONG_CLIP);
    }

    lv_obj_t *footer = lv_label_create(screen);
    lv_obj_set_pos(footer, 28, 425);
    lv_obj_set_width(footer, 424);
    lv_obj_set_style_text_align(footer, LV_TEXT_ALIGN_CENTER, 0);
    lv_obj_set_style_text_color(footer, lv_color_hex(0x9CA3AF), 0);
    lv_label_set_text(footer, "Serial log remains available");
}

void apply_snapshot(const status_ui_snapshot_t &snapshot)
{
    lv_label_set_text(s_ui.title, snapshot.title);
    lv_label_set_text(s_ui.badge, level_text(snapshot.level));
    lv_obj_set_style_bg_color(s_ui.badge, level_color(snapshot.level), 0);
    for (size_t i = 0; i < STATUS_UI_LINE_COUNT; ++i) {
        lv_label_set_text(s_ui.lines[i], snapshot.lines[i]);
    }
}

void status_ui_task(void *)
{
    if (!bsp_display_lock(0)) {
        ESP_LOGE(kTag, "Failed to acquire BSP display lock for page creation");
        vTaskDelete(nullptr);
        return;
    }
    create_page();
    bsp_display_unlock();

    status_ui_snapshot_t snapshot;
    while (true) {
        if (xQueueReceive(s_ui.queue, &snapshot, portMAX_DELAY) != pdTRUE) {
            continue;
        }
        if (!bsp_display_lock(250)) {
            ESP_LOGW(kTag, "BSP display lock unavailable");
            continue;
        }
        apply_snapshot(snapshot);
        bsp_display_unlock();
    }
}

} // namespace

extern "C" esp_err_t status_ui_init(void)
{
    if (s_ui.initialized) {
        return ESP_ERR_INVALID_STATE;
    }

    s_ui.queue = xQueueCreate(1, sizeof(status_ui_snapshot_t));
    if (s_ui.queue == nullptr) {
        ESP_LOGE(kTag, "Snapshot queue creation failed");
        return ESP_ERR_NO_MEM;
    }

    if (bsp_display_start() == nullptr) {
        ESP_LOGE(kTag, "Managed BSP display initialization failed");
        vQueueDelete(s_ui.queue);
        s_ui.queue = nullptr;
        return ESP_FAIL;
    }

    if (xTaskCreate(status_ui_task, "status_ui", 4096, nullptr, 4, nullptr) != pdPASS) {
        ESP_LOGE(kTag, "UI task creation failed");
        vQueueDelete(s_ui.queue);
        s_ui.queue = nullptr;
        return ESP_ERR_NO_MEM;
    }
    s_ui.initialized = true;
    ESP_LOGI(kTag, "Status UI initialized through the managed BSP");
    return ESP_OK;
}

extern "C" esp_err_t status_ui_publish(const status_ui_snapshot_t *snapshot)
{
    if (snapshot == nullptr) {
        return ESP_ERR_INVALID_ARG;
    }
    if (!s_ui.initialized || s_ui.queue == nullptr) {
        return ESP_ERR_INVALID_STATE;
    }
    return xQueueOverwrite(s_ui.queue, snapshot) == pdPASS ? ESP_OK : ESP_FAIL;
}

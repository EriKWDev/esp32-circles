#include <string.h>

#include "freertos/FreeRTOS.h"
#include "freertos/queue.h"
#include "freertos/task.h"
#include "driver/spi_master.h"
#include "esp_err.h"
#include "esp_lcd_panel_io.h"
#include "esp_lcd_panel_ops.h"
#include "esp_lcd_panel_vendor.h"
#include "esp_lcd_sh8601.h"
#include "esp_log.h"
#include "esp_lv_adapter.h"
#include "lvgl.h"

#include "status_ui.h"

namespace {

constexpr int kDisplayWidth = 480;
constexpr int kDisplayHeight = 480;
constexpr gpio_num_t kLcdSclk = GPIO_NUM_0;
constexpr gpio_num_t kLcdData0 = GPIO_NUM_1;
constexpr gpio_num_t kLcdData1 = GPIO_NUM_2;
constexpr gpio_num_t kLcdData2 = GPIO_NUM_3;
constexpr gpio_num_t kLcdData3 = GPIO_NUM_4;
constexpr gpio_num_t kLcdCs = GPIO_NUM_15;

static const char *const kTag = "status_ui";

static uint8_t kParam00[] = {0x00};
static uint8_t kParam20[] = {0x20};
static uint8_t kParam10[] = {0x10};
static uint8_t kParamA0[] = {0xA0};
static uint8_t kParam80[] = {0x80};
static uint8_t kParam55[] = {0x55};
static uint8_t kParam30[] = {0x30};
static uint8_t kParamFF[] = {0xFF};
static uint8_t kColumnRange[] = {0x00, 0x00, 0x01, 0xDF};
static uint8_t kRowRange[] = {0x00, 0x00, 0x01, 0xDF};

static const sh8601_lcd_init_cmd_t kLcdInitCommands[] = {
    {0x11, kParam00, 0, 600},
    {0xFE, kParam20, 1, 0},
    {0x19, kParam10, 1, 0},
    {0x1C, kParamA0, 1, 0},
    {0xFE, kParam00, 1, 0},
    {0xC4, kParam80, 1, 0},
    {0x3A, kParam55, 1, 0},
    {0x35, kParam00, 1, 0},
    {0x36, kParam30, 1, 0},
    {0x53, kParam20, 1, 0},
    {0x51, kParamFF, 1, 0},
    {0x63, kParamFF, 1, 0},
    {0x2A, kColumnRange, 4, 0},
    {0x2B, kRowRange, 4, 0},
    {0x29, kParam00, 0, 100},
};

struct StatusUiState {
    QueueHandle_t queue;
    lv_obj_t *title;
    lv_obj_t *badge;
    lv_obj_t *lines[STATUS_UI_LINE_COUNT];
    bool initialized;
};

StatusUiState s_ui = {};

void rounder_event_cb(lv_event_t *event)
{
    lv_area_t *area = static_cast<lv_area_t *>(lv_event_get_param(event));
    area->x1 = (area->x1 >> 1) << 1;
    area->y1 = (area->y1 >> 1) << 1;
    area->x2 = ((area->x2 >> 1) << 1) + 1;
    area->y2 = ((area->y2 >> 1) << 1) + 1;
}

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
    if (esp_lv_adapter_lock(-1) != ESP_OK) {
        ESP_LOGE(kTag, "Failed to acquire LVGL adapter lock for page creation");
        vTaskDelete(nullptr);
        return;
    }
    create_page();
    esp_lv_adapter_unlock();

    status_ui_snapshot_t snapshot;
    while (true) {
        if (xQueueReceive(s_ui.queue, &snapshot, portMAX_DELAY) != pdTRUE) {
            continue;
        }
        esp_err_t err = esp_lv_adapter_lock(250);
        if (err != ESP_OK) {
            ESP_LOGW(kTag, "LVGL adapter lock unavailable: %s", esp_err_to_name(err));
            continue;
        }
        apply_snapshot(snapshot);
        esp_lv_adapter_unlock();
    }
}

esp_err_t initialize_panel(const status_ui_config_t *config)
{
    spi_bus_config_t bus_config = {};
    bus_config.sclk_io_num = kLcdSclk;
    bus_config.data0_io_num = kLcdData0;
    bus_config.data1_io_num = kLcdData1;
    bus_config.data2_io_num = kLcdData2;
    bus_config.data3_io_num = kLcdData3;
    bus_config.max_transfer_sz = kDisplayWidth * 50 * 2;

    esp_err_t err = spi_bus_initialize(SPI2_HOST, &bus_config, SPI_DMA_CH_AUTO);
    if (err != ESP_OK) {
        ESP_LOGE(kTag, "SPI bus initialization failed: %s", esp_err_to_name(err));
        return err;
    }

    esp_lcd_panel_io_spi_config_t io_config = {};
    io_config.cs_gpio_num = kLcdCs;
    io_config.dc_gpio_num = GPIO_NUM_NC;
    io_config.spi_mode = 0;
    io_config.pclk_hz = 40 * 1000 * 1000;
    io_config.trans_queue_depth = 1;
    io_config.lcd_cmd_bits = 32;
    io_config.lcd_param_bits = 8;
    io_config.flags.quad_mode = true;

    esp_lcd_panel_io_handle_t io_handle = nullptr;
    err = esp_lcd_new_panel_io_spi((esp_lcd_spi_bus_handle_t)SPI2_HOST, &io_config, &io_handle);
    if (err != ESP_OK) {
        ESP_LOGE(kTag, "Panel IO initialization failed: %s", esp_err_to_name(err));
        return err;
    }

    sh8601_vendor_config_t vendor_config = {};
    vendor_config.init_cmds = kLcdInitCommands;
    vendor_config.init_cmds_size = sizeof(kLcdInitCommands) / sizeof(kLcdInitCommands[0]);
    vendor_config.flags.use_qspi_interface = 1;

    esp_lcd_panel_dev_config_t panel_config = {};
    panel_config.reset_gpio_num = GPIO_NUM_NC;
    panel_config.rgb_ele_order = LCD_RGB_ELEMENT_ORDER_RGB;
    panel_config.bits_per_pixel = 16;
    panel_config.vendor_config = &vendor_config;

    esp_lcd_panel_handle_t panel_handle = nullptr;
    err = esp_lcd_new_panel_sh8601(io_handle, &panel_config, &panel_handle);
    if (err != ESP_OK) {
        ESP_LOGE(kTag, "SH8601 panel creation failed: %s", esp_err_to_name(err));
        return err;
    }

    err = config->panel_power_reset(config->panel_power_reset_context);
    if (err != ESP_OK) {
        ESP_LOGE(kTag, "Panel power/reset callback failed: %s", esp_err_to_name(err));
        return err;
    }

    err = esp_lcd_panel_init(panel_handle);
    if (err != ESP_OK) {
        ESP_LOGE(kTag, "SH8601 panel initialization failed: %s", esp_err_to_name(err));
        return err;
    }

    esp_lv_adapter_config_t adapter_config = ESP_LV_ADAPTER_DEFAULT_CONFIG();
    adapter_config.task_stack_size = 20 * 1024;
    adapter_config.task_priority = 10;
    adapter_config.task_core_id = 0;
    adapter_config.stack_in_psram = false;
    err = esp_lv_adapter_init(&adapter_config);
    if (err != ESP_OK) {
        ESP_LOGE(kTag, "LVGL adapter initialization failed: %s", esp_err_to_name(err));
        return err;
    }

    esp_lv_adapter_display_config_t display_config =
        ESP_LV_ADAPTER_DISPLAY_SPI_WITHOUT_PSRAM_DEFAULT_CONFIG(
            panel_handle, io_handle, kDisplayWidth, kDisplayHeight, ESP_LV_ADAPTER_ROTATE_0);
    display_config.profile.buffer_height = 100;
    lv_display_t *display = esp_lv_adapter_register_display(&display_config);
    if (display == nullptr) {
        ESP_LOGE(kTag, "LVGL display registration failed");
        return ESP_FAIL;
    }
    lv_display_add_event_cb(display, rounder_event_cb, LV_EVENT_INVALIDATE_AREA, nullptr);

    err = esp_lv_adapter_start();
    if (err != ESP_OK) {
        ESP_LOGE(kTag, "LVGL adapter start failed: %s", esp_err_to_name(err));
    }
    return err;
}

} // namespace

extern "C" esp_err_t status_ui_init(const status_ui_config_t *config)
{
    if (config == nullptr || config->panel_power_reset == nullptr) {
        return ESP_ERR_INVALID_ARG;
    }
    if (s_ui.initialized) {
        return ESP_ERR_INVALID_STATE;
    }

    s_ui.queue = xQueueCreate(1, sizeof(status_ui_snapshot_t));
    if (s_ui.queue == nullptr) {
        ESP_LOGE(kTag, "Snapshot queue creation failed");
        return ESP_ERR_NO_MEM;
    }

    esp_err_t err = initialize_panel(config);
    if (err != ESP_OK) {
        vQueueDelete(s_ui.queue);
        s_ui.queue = nullptr;
        return err;
    }

    if (xTaskCreate(status_ui_task, "status_ui", 4096, nullptr, 4, nullptr) != pdPASS) {
        ESP_LOGE(kTag, "UI task creation failed");
        return ESP_ERR_NO_MEM;
    }
    s_ui.initialized = true;
    ESP_LOGI(kTag, "Status UI initialized");
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
    if (xQueueOverwrite(s_ui.queue, snapshot) != pdPASS) {
        return ESP_FAIL;
    }
    return ESP_OK;
}

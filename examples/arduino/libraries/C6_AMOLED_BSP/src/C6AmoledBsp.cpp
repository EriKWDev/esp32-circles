#include "C6AmoledBsp.h"

#include <Arduino.h>
#include <driver/gpio.h>
#include <esp_heap_caps.h>
#include <esp_lcd_panel_io.h>
#include <esp_lcd_panel_ops.h>
#include <esp_log.h>
#include <esp_timer.h>
#include <freertos/FreeRTOS.h>
#include <freertos/queue.h>
#include <freertos/semphr.h>
#include <freertos/task.h>
#include <lvgl.h>

#include "esp_lcd_sh8601.h"

namespace {

constexpr uint32_t kI2cDataTimeoutMs = 5000;
constexpr uint32_t kI2cDoneTimeoutMs = 1000;
constexpr uint32_t kLvglTickMs = 2;
constexpr uint32_t kLvglTaskMinDelayMs = 5;
constexpr uint32_t kLvglTaskMaxDelayMs = 500;
constexpr size_t kDisplayBufferBytes = C6_AMOLED_LCD_H_RES * 50 * 2;

c6_amoled::Board *s_active_board = nullptr;
esp_lcd_panel_handle_t s_panel = NULL;
esp_lcd_panel_io_handle_t s_panel_io = NULL;
SemaphoreHandle_t s_lvgl_mutex = NULL;
bool s_display_started = false;
bool s_touch_started = false;

class Cst9217Touch {
 public:
  bool begin(I2cMasterBus &i2c) {
    if (!i2c.ready()) return false;
    i2c_device_config_t config = {};
    config.dev_addr_length = I2C_ADDR_BIT_LEN_7;
    config.scl_speed_hz = 400000;
    config.device_address = 0x5A;
    esp_err_t err = i2c_master_bus_add_device(i2c.Get_I2cBusHandle(), &config, &device_);
    if (err != ESP_OK) {
      ESP_LOGE("C6AmoledBsp", "CST9217 I2C init failed: %s", esp_err_to_name(err));
      return false;
    }

    gpio_config_t gpio = {};
    gpio.mode = GPIO_MODE_OUTPUT;
    gpio.pin_bit_mask = 1ULL << C6_AMOLED_TOUCH_RST;
    gpio.pull_up_en = GPIO_PULLUP_ENABLE;
    err = gpio_config(&gpio);
    if (err != ESP_OK) {
      ESP_LOGE("C6AmoledBsp", "CST9217 reset GPIO failed: %s", esp_err_to_name(err));
      return false;
    }
    gpio_set_level(C6_AMOLED_TOUCH_RST, 1);
    vTaskDelay(pdMS_TO_TICKS(200));
    gpio_set_level(C6_AMOLED_TOUCH_RST, 0);
    vTaskDelay(pdMS_TO_TICKS(200));
    gpio_set_level(C6_AMOLED_TOUCH_RST, 1);
    vTaskDelay(pdMS_TO_TICKS(200));
    i2c_ = &i2c;
    return true;
  }

  bool read(uint16_t *x, uint16_t *y) {
    if (!i2c_ || !device_) return false;
    uint8_t command[] = {0xD0, 0x00};
    uint8_t data[10] = {};
    esp_err_t err = static_cast<esp_err_t>(i2c_->i2c_master_write_read_dev(device_, command, sizeof(command), data, sizeof(data)));
    if (err != ESP_OK || data[6] != 0xAB || (data[5] & 0x7F) == 0 || (data[0] & 0x0F) != 0x06) return false;
    const uint16_t raw_y = (static_cast<uint16_t>(data[1]) << 4) | (data[3] >> 4);
    const uint16_t raw_x = (static_cast<uint16_t>(data[2]) << 4) | (data[3] & 0x0F);
    const uint16_t rotated_x = C6_AMOLED_LCD_H_RES - raw_x;
    *x = rotated_x >= C6_AMOLED_LCD_H_RES ? C6_AMOLED_LCD_H_RES - 1 : rotated_x;
    *y = raw_y >= C6_AMOLED_LCD_V_RES ? C6_AMOLED_LCD_V_RES - 1 : raw_y;
    return true;
  }

 private:
  I2cMasterBus *i2c_ = nullptr;
  i2c_master_dev_handle_t device_ = NULL;
};

Cst9217Touch s_touch;

#if LVGL_VERSION_MAJOR >= 9
lv_display_t *s_display = NULL;
lv_indev_t *s_touch_input = NULL;
#else
lv_disp_draw_buf_t s_draw_buf;
lv_disp_drv_t s_display_driver;
lv_disp_t *s_display = NULL;
lv_indev_drv_t s_touch_driver;
lv_indev_t *s_touch_input = NULL;
#endif

static const sh8601_lcd_init_cmd_t kSh8601InitCommands[] = {
    {0x11, (uint8_t[]){0x00}, 0, 600}, {0xFE, (uint8_t[]){0x20}, 1, 0},
    {0x19, (uint8_t[]){0x10}, 1, 0},   {0x1C, (uint8_t[]){0xA0}, 1, 0},
    {0xFE, (uint8_t[]){0x00}, 1, 0},   {0xC4, (uint8_t[]){0x80}, 1, 0},
    {0x3A, (uint8_t[]){0x55}, 1, 0},   {0x35, (uint8_t[]){0x00}, 1, 0},
    {0x36, (uint8_t[]){0x30}, 1, 0},   {0x53, (uint8_t[]){0x20}, 1, 0},
    {0x51, (uint8_t[]){0xFF}, 1, 0},   {0x63, (uint8_t[]){0xFF}, 1, 0},
    {0x2A, (uint8_t[]){0x00, 0x00, 0x01, 0xDF}, 4, 0},
    {0x2B, (uint8_t[]){0x00, 0x00, 0x01, 0xDF}, 4, 0},
    {0x29, (uint8_t[]){0x00}, 0, 100},
};

bool takeLvgl(TickType_t timeout = portMAX_DELAY) {
  return s_lvgl_mutex && xSemaphoreTake(s_lvgl_mutex, timeout) == pdTRUE;
}

void giveLvgl() {
  if (s_lvgl_mutex) xSemaphoreGive(s_lvgl_mutex);
}

#if LVGL_VERSION_MAJOR >= 9
void displayFlush(lv_display_t *, const lv_area_t *area, uint8_t *color_p) {
  lv_draw_sw_rgb565_swap(color_p, lv_area_get_width(area) * lv_area_get_height(area));
  esp_lcd_panel_draw_bitmap(s_panel, area->x1, area->y1, area->x2 + 1, area->y2 + 1, color_p);
  lv_display_flush_ready(s_display);
}

void displayRounder(lv_event_t *event) {
  lv_area_t *area = static_cast<lv_area_t *>(lv_event_get_param(event));
  area->x1 &= ~1;
  area->y1 &= ~1;
  area->x2 = (area->x2 & ~1) + 1;
  area->y2 = (area->y2 & ~1) + 1;
}
#else
void displayFlush(lv_disp_drv_t *display_driver, const lv_area_t *area, lv_color_t *color_p) {
  esp_lcd_panel_draw_bitmap(s_panel, area->x1, area->y1, area->x2 + 1, area->y2 + 1, color_p);
  lv_disp_flush_ready(display_driver);
}

void displayRounder(lv_disp_drv_t *, lv_area_t *area) {
  area->x1 &= ~1;
  area->y1 &= ~1;
  area->x2 = (area->x2 & ~1) + 1;
  area->y2 = (area->y2 & ~1) + 1;
}
#endif

#if LVGL_VERSION_MAJOR >= 9
void touchRead(lv_indev_t *, lv_indev_data_t *data) {
#else
void touchRead(lv_indev_drv_t *, lv_indev_data_t *data) {
#endif
  uint16_t x = 0;
  uint16_t y = 0;
  if (s_touch.read(&x, &y)) {
    data->point.x = x;
    data->point.y = y;
    data->state = LV_INDEV_STATE_PRESSED;
  } else {
    data->state = LV_INDEV_STATE_RELEASED;
  }
}

bool startTouch() {
  if (s_touch_started) return true;
  if (!s_active_board || !s_touch.begin(s_active_board->i2c())) return false;
#if LVGL_VERSION_MAJOR >= 9
  s_touch_input = lv_indev_create();
  if (!s_touch_input) return false;
  lv_indev_set_type(s_touch_input, LV_INDEV_TYPE_POINTER);
  lv_indev_set_read_cb(s_touch_input, touchRead);
#else
  lv_indev_drv_init(&s_touch_driver);
  s_touch_driver.type = LV_INDEV_TYPE_POINTER;
  s_touch_driver.read_cb = touchRead;
  s_touch_input = lv_indev_drv_register(&s_touch_driver);
  if (!s_touch_input) return false;
#endif
  s_touch_started = true;
  return true;
}

void lvglTick(void *) { lv_tick_inc(kLvglTickMs); }

void lvglTask(void *) {
  for (;;) {
    uint32_t delay_ms = kLvglTaskMaxDelayMs;
    if (takeLvgl()) {
      delay_ms = lv_timer_handler();
      giveLvgl();
    }
    delay_ms = delay_ms < kLvglTaskMinDelayMs ? kLvglTaskMinDelayMs : delay_ms;
    delay_ms = delay_ms > kLvglTaskMaxDelayMs ? kLvglTaskMaxDelayMs : delay_ms;
    vTaskDelay(pdMS_TO_TICKS(delay_ms));
  }
}

bool resetPanelPower() {
  if (!s_active_board || !s_active_board->pmicReady()) return false;
  XPowersPMU &pmic = s_active_board->pmic();
  pmic.enableALDO3();
  vTaskDelay(pdMS_TO_TICKS(100));
  pmic.disableALDO3();
  vTaskDelay(pdMS_TO_TICKS(100));
  pmic.enableALDO3();
  vTaskDelay(pdMS_TO_TICKS(100));
  return true;
}

bool startDisplay(bool enable_touch) {
  if (s_display_started) {
    if (enable_touch && !s_touch_started) {
      const bool locked = takeLvgl();
      const bool touch_started = locked && startTouch();
      if (locked) giveLvgl();
      if (!touch_started) ESP_LOGE("C6AmoledBsp", "CST9217 touch unavailable");
    }
    return true;
  }
  s_lvgl_mutex = xSemaphoreCreateMutex();
  if (!s_lvgl_mutex) return false;

  spi_bus_config_t bus_config = {};
  bus_config.sclk_io_num = C6_AMOLED_LCD_PCLK;
  bus_config.data0_io_num = C6_AMOLED_LCD_DATA0;
  bus_config.data1_io_num = C6_AMOLED_LCD_DATA1;
  bus_config.data2_io_num = C6_AMOLED_LCD_DATA2;
  bus_config.data3_io_num = C6_AMOLED_LCD_DATA3;
  bus_config.max_transfer_sz = C6_AMOLED_LCD_H_RES * C6_AMOLED_LCD_V_RES * 2;
  esp_err_t err = spi_bus_initialize(C6_AMOLED_LCD_SPI_HOST, &bus_config, SPI_DMA_CH_AUTO);
  if (err != ESP_OK && err != ESP_ERR_INVALID_STATE) {
    ESP_LOGE("C6AmoledBsp", "LCD SPI bus init failed: %s", esp_err_to_name(err));
    return false;
  }

  esp_lcd_panel_io_spi_config_t io_config = {};
  io_config.cs_gpio_num = C6_AMOLED_LCD_CS;
  io_config.dc_gpio_num = GPIO_NUM_NC;
  io_config.spi_mode = 0;
  io_config.pclk_hz = 40 * 1000 * 1000;
  io_config.trans_queue_depth = 2;
  io_config.lcd_cmd_bits = 32;
  io_config.lcd_param_bits = 8;
  io_config.flags.quad_mode = true;
  err = esp_lcd_new_panel_io_spi((esp_lcd_spi_bus_handle_t)C6_AMOLED_LCD_SPI_HOST, &io_config, &s_panel_io);
  if (err != ESP_OK) return false;

  sh8601_vendor_config_t vendor_config = {};
  vendor_config.init_cmds = kSh8601InitCommands;
  vendor_config.init_cmds_size = sizeof(kSh8601InitCommands) / sizeof(kSh8601InitCommands[0]);
  vendor_config.flags.use_qspi_interface = 1;
  esp_lcd_panel_dev_config_t panel_config = {};
  panel_config.reset_gpio_num = GPIO_NUM_NC;
  panel_config.rgb_ele_order = LCD_RGB_ELEMENT_ORDER_RGB;
  panel_config.bits_per_pixel = 16;
  panel_config.vendor_config = &vendor_config;
  err = esp_lcd_new_panel_sh8601(s_panel_io, &panel_config, &s_panel);
  if (err != ESP_OK || !resetPanelPower()) return false;
  if (esp_lcd_panel_init(s_panel) != ESP_OK) return false;

  lv_init();
  uint8_t *buffer1 = static_cast<uint8_t *>(heap_caps_malloc(kDisplayBufferBytes, MALLOC_CAP_DMA));
  uint8_t *buffer2 = static_cast<uint8_t *>(heap_caps_malloc(kDisplayBufferBytes, MALLOC_CAP_DMA));
  if (!buffer1 || !buffer2) return false;
#if LVGL_VERSION_MAJOR >= 9
  s_display = lv_display_create(C6_AMOLED_LCD_H_RES, C6_AMOLED_LCD_V_RES);
  if (!s_display) return false;
  lv_display_set_flush_cb(s_display, displayFlush);
  lv_display_set_buffers(s_display, buffer1, buffer2, kDisplayBufferBytes, LV_DISPLAY_RENDER_MODE_PARTIAL);
  lv_display_add_event_cb(s_display, displayRounder, LV_EVENT_INVALIDATE_AREA, NULL);
#else
  lv_disp_draw_buf_init(&s_draw_buf, reinterpret_cast<lv_color_t *>(buffer1), reinterpret_cast<lv_color_t *>(buffer2), C6_AMOLED_LCD_H_RES * 50);
  lv_disp_drv_init(&s_display_driver);
  s_display_driver.hor_res = C6_AMOLED_LCD_H_RES;
  s_display_driver.ver_res = C6_AMOLED_LCD_V_RES;
  s_display_driver.flush_cb = displayFlush;
  s_display_driver.rounder_cb = displayRounder;
  s_display_driver.draw_buf = &s_draw_buf;
  s_display = lv_disp_drv_register(&s_display_driver);
  if (!s_display) return false;
#endif

  if (enable_touch && !startTouch()) ESP_LOGE("C6AmoledBsp", "CST9217 touch unavailable");

  esp_timer_create_args_t timer_args = {};
  timer_args.callback = lvglTick;
  timer_args.name = "c6_lvgl_tick";
  esp_timer_handle_t timer = NULL;
  if (esp_timer_create(&timer_args, &timer) != ESP_OK ||
      esp_timer_start_periodic(timer, kLvglTickMs * 1000) != ESP_OK ||
      xTaskCreate(lvglTask, "C6Lvgl", 8 * 1024, NULL, 2, NULL) != pdPASS) {
    return false;
  }
  s_display_started = true;
  return true;
}

}  // namespace

namespace c6_amoled {

bool lvglLock(uint32_t timeout_ms) {
  const TickType_t timeout = timeout_ms == 0 ? portMAX_DELAY : pdMS_TO_TICKS(timeout_ms);
  return takeLvgl(timeout);
}

void lvglUnlock() { giveLvgl(); }

bool setBacklight(uint8_t percent) {
  if (!s_panel_io) return false;
  if (percent > 100) percent = 100;
  const uint8_t value = static_cast<uint8_t>((percent * 255U) / 100U);
  const uint32_t command = 0x02005100;
  esp_err_t err = esp_lcd_panel_io_tx_param(s_panel_io, command, &value, 1);
  if (err != ESP_OK) ESP_LOGE("C6AmoledBsp", "backlight update failed: %s", esp_err_to_name(err));
  return err == ESP_OK;
}

}  // namespace c6_amoled

I2cMasterBus::I2cMasterBus(int scl_pin, int sda_pin, int i2c_port) {
  i2c_master_bus_config_t config = {};
  config.clk_source = I2C_CLK_SRC_DEFAULT;
  config.i2c_port = static_cast<i2c_port_t>(i2c_port);
  config.scl_io_num = static_cast<gpio_num_t>(scl_pin);
  config.sda_io_num = static_cast<gpio_num_t>(sda_pin);
  config.glitch_ignore_cnt = 7;
  config.flags.enable_internal_pullup = true;
  status_ = i2c_new_master_bus(&config, &user_i2c_handle_);
  if (status_ != ESP_OK) ESP_LOGE("C6AmoledBsp", "I2C init failed: %s", esp_err_to_name(status_));
}

I2cMasterBus::~I2cMasterBus() {}
bool I2cMasterBus::ready() const { return status_ == ESP_OK && user_i2c_handle_; }
esp_err_t I2cMasterBus::status() const { return status_; }
i2c_master_bus_handle_t I2cMasterBus::Get_I2cBusHandle() { return user_i2c_handle_; }

int I2cMasterBus::i2c_write_buff(i2c_master_dev_handle_t handle, int reg, uint8_t *buf, uint8_t len) {
  if (!ready()) return status_;
  esp_err_t err = i2c_master_bus_wait_all_done(user_i2c_handle_, pdMS_TO_TICKS(kI2cDoneTimeoutMs));
  if (err != ESP_OK) return err;
  if (reg == -1) return i2c_master_transmit(handle, buf, len, pdMS_TO_TICKS(kI2cDataTimeoutMs));
  uint8_t payload[256];
  if (len >= sizeof(payload)) return ESP_ERR_INVALID_SIZE;
  payload[0] = static_cast<uint8_t>(reg);
  memcpy(payload + 1, buf, len);
  return i2c_master_transmit(handle, payload, len + 1, pdMS_TO_TICKS(kI2cDataTimeoutMs));
}

int I2cMasterBus::i2c_master_write_read_dev(i2c_master_dev_handle_t handle, uint8_t *write_buf, uint8_t write_len, uint8_t *read_buf, uint8_t read_len) {
  if (!ready()) return status_;
  esp_err_t err = i2c_master_bus_wait_all_done(user_i2c_handle_, pdMS_TO_TICKS(kI2cDoneTimeoutMs));
  if (err != ESP_OK) return err;
  return i2c_master_transmit_receive(handle, write_buf, write_len, read_buf, read_len, pdMS_TO_TICKS(kI2cDataTimeoutMs));
}

int I2cMasterBus::i2c_read_buff(i2c_master_dev_handle_t handle, int reg, uint8_t *buf, uint8_t len) {
  if (!ready()) return status_;
  esp_err_t err = i2c_master_bus_wait_all_done(user_i2c_handle_, pdMS_TO_TICKS(kI2cDoneTimeoutMs));
  if (err != ESP_OK) return err;
  if (reg == -1) return i2c_master_receive(handle, buf, len, pdMS_TO_TICKS(kI2cDataTimeoutMs));
  uint8_t address = static_cast<uint8_t>(reg);
  return i2c_master_transmit_receive(handle, &address, 1, buf, len, pdMS_TO_TICKS(kI2cDataTimeoutMs));
}

namespace c6_amoled {

Board::Board() : i2c_(C6_AMOLED_I2C_SCL, C6_AMOLED_I2C_SDA, C6_AMOLED_I2C_PORT) {}

bool Board::begin() { return beginPmic(); }
I2cMasterBus &Board::i2c() { return i2c_; }
XPowersPMU &Board::pmic() { return pmic_; }
bool Board::pmicReady() const { return pmic_ready_; }

int Board::pmicRead(uint8_t, uint8_t reg_addr, uint8_t *data, uint8_t len) {
  return (s_active_board && s_active_board->pmic_device_ &&
          s_active_board->i2c_.i2c_read_buff(s_active_board->pmic_device_, reg_addr, data, len) == ESP_OK) ? 0 : -1;
}

int Board::pmicWrite(uint8_t, uint8_t reg_addr, uint8_t *data, uint8_t len) {
  return (s_active_board && s_active_board->pmic_device_ &&
          s_active_board->i2c_.i2c_write_buff(s_active_board->pmic_device_, reg_addr, data, len) == ESP_OK) ? 0 : -1;
}

void Board::configurePmic() {
  pmic_.setVbusCurrentLimit(XPOWERS_AXP2101_VBUS_CUR_LIM_2000MA);
  if (pmic_.getDC1Voltage() != 3300) pmic_.setDC1Voltage(3300);
  if (pmic_.getALDO1Voltage() != 3300) pmic_.setALDO1Voltage(3300);
  if (pmic_.getALDO2Voltage() != 3300) pmic_.setALDO2Voltage(3300);
  if (pmic_.getALDO3Voltage() != 3300) pmic_.setALDO3Voltage(3300);
  if (pmic_.getALDO4Voltage() != 3300) pmic_.setALDO4Voltage(3300);
  pmic_.setPrechargeCurr(XPOWERS_AXP2101_PRECHARGE_50MA);
  pmic_.setChargerConstantCurr(XPOWERS_AXP2101_CHG_CUR_500MA);
  pmic_.setChargerTerminationCurr(XPOWERS_AXP2101_CHG_ITERM_50MA);
}

bool Board::beginPmic() {
  if (pmic_ready_) return true;
  if (!i2c_.ready()) return false;
  s_active_board = this;
  if (!pmic_device_) {
    i2c_device_config_t config = {};
    config.dev_addr_length = I2C_ADDR_BIT_LEN_7;
    config.scl_speed_hz = 100000;
    config.device_address = C6_AMOLED_PMIC_ADDRESS;
    esp_err_t err = i2c_master_bus_add_device(i2c_.Get_I2cBusHandle(), &config, &pmic_device_);
    if (err != ESP_OK) {
      ESP_LOGE("C6AmoledBsp", "PMIC device init failed: %s", esp_err_to_name(err));
      return false;
    }
  }
  pmic_ready_ = pmic_.begin(C6_AMOLED_PMIC_ADDRESS, pmicRead, pmicWrite);
  if (pmic_ready_) configurePmic();
  Serial.printf("AXP2101 init: %s\n", pmic_ready_ ? "ready" : "failed");
  return pmic_ready_;
}

PmicSnapshot Board::pmicSnapshot() {
  PmicSnapshot snapshot = {};
  snapshot.ready = pmic_ready_;
  if (pmic_ready_) {
    snapshot.charging = pmic_.isCharging();
    snapshot.charger_status = pmic_.getChargerStatus();
    snapshot.battery_mv = pmic_.getBattVoltage();
  }
  return snapshot;
}

bool Board::beginDisplay(bool enable_touch) {
  if (display_ready_) {
    if (enable_touch && !s_touch_started) {
      const bool locked = lvglLock();
      const bool touch_started = locked && startTouch();
      if (locked) lvglUnlock();
      if (!touch_started) {
        ESP_LOGE("C6AmoledBsp", "CST9217 touch unavailable");
      }
    }
    touch_ready_ = s_touch_started;
    return true;
  }
  if (!pmic_ready_) return false;
  display_ready_ = startDisplay(enable_touch);
  touch_ready_ = s_touch_started;
  if (!display_ready_) ESP_LOGE("C6AmoledBsp", "status display unavailable");
  return display_ready_;
}

bool StatusPage::begin(Board &board) {
  if (ready_) return true;
  if (!board.beginDisplay(false)) return false;
  QueueHandle_t queue = xQueueCreate(1, sizeof(StatusPageData));
  if (!queue) return false;
  queue_ = queue;
  if (xTaskCreate(task, "C6Status", 4 * 1024, this, 2, NULL) != pdPASS) return false;
  ready_ = true;
  return true;
}

bool StatusPage::ready() const { return ready_; }

bool StatusPage::publish(const StatusPageData &data) {
  return queue_ && xQueueOverwrite(static_cast<QueueHandle_t>(queue_), &data) == pdPASS;
}

void StatusPage::task(void *context) {
  StatusPage *page = static_cast<StatusPage *>(context);
  StatusPageData data = {};
  for (;;) {
    if (xQueueReceive(static_cast<QueueHandle_t>(page->queue_), &data, portMAX_DELAY) == pdPASS) page->render(data);
  }
}

void StatusPage::render(const StatusPageData &data) {
  if (!lvglLock(100)) return;
  if (!labels_[0]) {
#if LVGL_VERSION_MAJOR >= 9
    lv_obj_t *screen = lv_screen_active();
#else
    lv_obj_t *screen = lv_scr_act();
#endif
    labels_[0] = lv_label_create(screen);
    lv_obj_set_pos(static_cast<lv_obj_t *>(labels_[0]), 24, 24);
    for (size_t i = 0; i < 6; ++i) {
      labels_[i + 1] = lv_label_create(screen);
      lv_obj_set_pos(static_cast<lv_obj_t *>(labels_[i + 1]), 24, 70 + i * 48);
    }
  }
  lv_label_set_text(static_cast<lv_obj_t *>(labels_[0]), data.title);
  for (size_t i = 0; i < 6; ++i) lv_label_set_text(static_cast<lv_obj_t *>(labels_[i + 1]), data.lines[i]);
  lvglUnlock();
}

const char *chargerStatusText(uint8_t status) {
  switch (status) {
    case XPOWERS_AXP2101_CHG_TRI_STATE: return "tri-charge";
    case XPOWERS_AXP2101_CHG_PRE_STATE: return "pre-charge";
    case XPOWERS_AXP2101_CHG_CC_STATE: return "constant-charge";
    case XPOWERS_AXP2101_CHG_CV_STATE: return "constant-voltage";
    case XPOWERS_AXP2101_CHG_DONE_STATE: return "charge-done";
    case XPOWERS_AXP2101_CHG_STOP_STATE: return "not-charging";
    default: return "unknown";
  }
}

}  // namespace c6_amoled

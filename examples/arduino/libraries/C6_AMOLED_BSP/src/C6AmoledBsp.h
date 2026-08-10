#pragma once

#include <stdint.h>

#include "c6_amoled_board_config.h"
#include "i2c_bsp.h"

#define XPOWERS_CHIP_AXP2101
#include <XPowersLib.h>

namespace c6_amoled {

struct PmicSnapshot {
  bool ready;
  bool charging;
  uint8_t charger_status;
  uint16_t battery_mv;
};

struct StatusPageData {
  char title[32];
  char lines[6][64];
};

// A timeout of 0 waits indefinitely, matching the original example ports.
bool lvglLock(uint32_t timeout_ms = 0);
void lvglUnlock();
bool setBacklight(uint8_t percent);

class Board {
 public:
  Board();
  bool begin();
  bool beginPmic();
  bool pmicReady() const;
  PmicSnapshot pmicSnapshot();
  I2cMasterBus &i2c();
  XPowersPMU &pmic();
  bool beginDisplay(bool enable_touch = false);

 private:
  static int pmicRead(uint8_t dev_addr, uint8_t reg_addr, uint8_t *data, uint8_t len);
  static int pmicWrite(uint8_t dev_addr, uint8_t reg_addr, uint8_t *data, uint8_t len);
  void configurePmic();

  I2cMasterBus i2c_;
  XPowersPMU pmic_;
  i2c_master_dev_handle_t pmic_device_ = NULL;
  bool pmic_ready_ = false;
  bool display_ready_ = false;
  bool touch_ready_ = false;
};

class StatusPage {
 public:
  bool begin(Board &board);
  bool ready() const;
  bool publish(const StatusPageData &data);

 private:
  static void task(void *context);
  void render(const StatusPageData &data);

  void *queue_ = NULL;
  void *labels_[7] = {};
  bool ready_ = false;
};

const char *chargerStatusText(uint8_t status);

}  // namespace c6_amoled

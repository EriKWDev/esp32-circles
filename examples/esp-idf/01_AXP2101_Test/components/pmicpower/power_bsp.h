#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "esp_err.h"
#include "i2c_bsp.h"

typedef struct {
    bool charging;
    uint8_t charger_status;
    uint16_t battery_mv;
} axp2101_snapshot_t;

esp_err_t Custom_PmicPortInit(I2cMasterBus *i2cbus,uint8_t dev_addr);
void Custom_PmicRegisterInit(void);
void Axp2101_isChargingTask(void *arg);
esp_err_t Axp2101_GetSnapshot(axp2101_snapshot_t *snapshot);
const char *Axp2101_ChargerStatusText(uint8_t charger_status);

void Axp2101_SetAldo2(uint8_t vol);
void Axp2101_SetAldo3(uint8_t vol);

#include <stdio.h>
#include <freertos/FreeRTOS.h>
#include <freertos/semphr.h>
#include <driver/gpio.h>
#include <esp_err.h>
#include <esp_log.h>
#include <esp_sleep.h>
#include "power_bsp.h"
#include "XPowersLib.h"

const char *TAG = "axp2101";

static XPowersPMU axp2101;

static I2cMasterBus           *i2cbus_   = NULL;
static i2c_master_dev_handle_t i2cPMICdev = NULL;
static uint8_t                 i2cPMICAddress;
static SemaphoreHandle_t       pmic_mutex = NULL;

static int AXP2101_SLAVE_Read(uint8_t devAddr, uint8_t regAddr, uint8_t *data, uint8_t len) {
    int ret;
    uint8_t count = 3;
    do
    {
        ret = (i2cbus_->i2c_read_buff(i2cPMICdev, regAddr, data, len) == ESP_OK) ? 0 : -1;
        if (ret == 0)
            break;
        vTaskDelay(pdMS_TO_TICKS(100));
        count--;
    } while (count);
    return ret;
}

static int AXP2101_SLAVE_Write(uint8_t devAddr, uint8_t regAddr, uint8_t *data, uint8_t len) {
    int ret;
    uint8_t count = 3;
    do
    {
        ret = (i2cbus_->i2c_write_buff(i2cPMICdev, regAddr, data, len) == ESP_OK) ? 0 : -1;
        if (ret == 0)
            break;
        vTaskDelay(pdMS_TO_TICKS(100));
        count--;
    } while (count);
    return ret;
}


esp_err_t Custom_PmicPortInit(I2cMasterBus *i2cbus,uint8_t dev_addr) {
    if (i2cbus == NULL) {
        return ESP_ERR_INVALID_ARG;
    }
    if(i2cbus_ == NULL) {
        i2cbus_ = i2cbus;
    }
    if (pmic_mutex == NULL) {
        pmic_mutex = xSemaphoreCreateMutex();
        if (pmic_mutex == NULL) {
            ESP_LOGE(TAG, "PMIC mutex allocation failed");
            return ESP_ERR_NO_MEM;
        }
    }
    if(i2cPMICdev == NULL) {
        i2c_master_bus_handle_t BusHandle = i2cbus_->Get_I2cBusHandle();
        i2c_device_config_t     dev_cfg   = {};
        dev_cfg.dev_addr_length           = I2C_ADDR_BIT_LEN_7;
        dev_cfg.scl_speed_hz              = 100000;
        dev_cfg.device_address            = dev_addr;
        esp_err_t err = i2c_master_bus_add_device(BusHandle, &dev_cfg, &i2cPMICdev);
        if (err != ESP_OK) {
            ESP_LOGE(TAG, "PMIC I2C device registration failed: %s", esp_err_to_name(err));
            return err;
        }
        i2cPMICAddress = dev_addr;
    }
    if (axp2101.begin(i2cPMICAddress, AXP2101_SLAVE_Read, AXP2101_SLAVE_Write)) {
        ESP_LOGI(TAG, "Init PMU SUCCESS!");
    } else {
        ESP_LOGE(TAG, "Init PMU FAILED!");
        return ESP_FAIL;
    }
    Custom_PmicRegisterInit();
    return ESP_OK;
}

void Custom_PmicRegisterInit(void) {
    axp2101.setVbusCurrentLimit(XPOWERS_AXP2101_VBUS_CUR_LIM_2000MA);

    if(axp2101.getDC1Voltage() != 3300) {
        axp2101.setDC1Voltage(3300);
        ESP_LOGW("axp2101_init_log","Set DCDC1 to output 3V3");
    }
    if(axp2101.getALDO1Voltage() != 3300) {
        axp2101.setALDO1Voltage(3300);
        ESP_LOGW("axp2101_init_log","Set ALDO1 to output 3V3");
    }
    if(axp2101.getALDO2Voltage() != 3300) {
        axp2101.setALDO2Voltage(3300);
        ESP_LOGW("axp2101_init_log","Set ALDO2 to output 3V3");
    }
    if(axp2101.getALDO3Voltage() != 3300) {
        axp2101.setALDO3Voltage(3300);
        ESP_LOGW("axp2101_init_log","Set ALDO3 to output 3V3");
    }
    if(axp2101.getALDO4Voltage() != 3300) {
        axp2101.setALDO4Voltage(3300);
        ESP_LOGW("axp2101_init_log","Set ALDO4 to output 3V3");
    }

    axp2101.setPrechargeCurr(XPOWERS_AXP2101_PRECHARGE_50MA);
    axp2101.setChargerConstantCurr(XPOWERS_AXP2101_CHG_CUR_500MA);
    axp2101.setChargerTerminationCurr(XPOWERS_AXP2101_CHG_ITERM_50MA);
}

void Axp2101_isChargingTask(void *arg) {
    for (;;) {
        vTaskDelay(pdMS_TO_TICKS(2000));
        axp2101_snapshot_t snapshot = {};
        if (Axp2101_GetSnapshot(&snapshot) != ESP_OK) {
            ESP_LOGW(TAG, "Unable to read charging status");
            continue;
        }
        ESP_LOGI(TAG, "isCharging: %s", snapshot.charging ? "YES" : "NO");
        ESP_LOGI(TAG, "Charger Status: %s", Axp2101_ChargerStatusText(snapshot.charger_status));
        ESP_LOGI(TAG, "getBattVoltage: %d mV", snapshot.battery_mv);
    }
}

esp_err_t Axp2101_GetSnapshot(axp2101_snapshot_t *snapshot) {
    if (snapshot == NULL || pmic_mutex == NULL) {
        return ESP_ERR_INVALID_STATE;
    }
    if (xSemaphoreTake(pmic_mutex, pdMS_TO_TICKS(100)) != pdTRUE) {
        return ESP_ERR_TIMEOUT;
    }
    snapshot->charging = axp2101.isCharging();
    snapshot->charger_status = axp2101.getChargerStatus();
    snapshot->battery_mv = axp2101.getBattVoltage();
    xSemaphoreGive(pmic_mutex);
    return ESP_OK;
}

const char *Axp2101_ChargerStatusText(uint8_t charger_status) {
    if (charger_status == XPOWERS_AXP2101_CHG_TRI_STATE) {
        return "tri charge";
    }
    if (charger_status == XPOWERS_AXP2101_CHG_PRE_STATE) {
        return "pre charge";
    }
    if (charger_status == XPOWERS_AXP2101_CHG_CC_STATE) {
        return "constant charge";
    }
    if (charger_status == XPOWERS_AXP2101_CHG_CV_STATE) {
        return "constant voltage";
    }
    if (charger_status == XPOWERS_AXP2101_CHG_DONE_STATE) {
        return "charge done";
    }
    if (charger_status == XPOWERS_AXP2101_CHG_STOP_STATE) {
        return "not charging";
    }
    return "unknown";
}

void Axp2101_SetAldo3(uint8_t vol) {
    if (pmic_mutex == NULL || xSemaphoreTake(pmic_mutex, pdMS_TO_TICKS(100)) != pdTRUE) {
        ESP_LOGW(TAG, "Unable to set ALDO3");
        return;
    }
    if(vol) {
        axp2101.enableALDO3();
    } else {
        axp2101.disableALDO3();
    }
    xSemaphoreGive(pmic_mutex);
}

void Axp2101_SetAldo2(uint8_t vol) {
    if (pmic_mutex == NULL || xSemaphoreTake(pmic_mutex, pdMS_TO_TICKS(100)) != pdTRUE) {
        ESP_LOGW(TAG, "Unable to set ALDO2");
        return;
    }
    if(vol) {
        axp2101.enableALDO2();
    } else {
        axp2101.disableALDO2();
    }
    xSemaphoreGive(pmic_mutex);
}

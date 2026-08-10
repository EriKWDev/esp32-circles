
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <freertos/FreeRTOS.h>
#include <esp_log.h>
#include <esp_wifi.h>
#include <nvs_flash.h>

#include "i2c_bsp.h"
#include "power_bsp.h"
#include "status_ui.h"
#include "user_config.h"

#define APP_NAME "WIFI STA & AP"

I2cMasterBus I2cMasterBus_(BSP_I2C_SCL,BSP_I2C_SDA,BSP_I2C_NUM);
esp_event_handler_instance_t wifi_event_instance;
esp_event_handler_instance_t ip_event_instance;

static const char *const kApSsid = "ESP32_AP";
static constexpr uint8_t kApChannel = 1;
static char wifi_ip[16] = {0};

typedef enum {
    AP_CLIENT_EVENT_NONE,
    AP_CLIENT_EVENT_CONNECTED,
    AP_CLIENT_EVENT_DISCONNECTED,
} ap_client_event_t;

static portMUX_TYPE s_ap_event_lock = portMUX_INITIALIZER_UNLOCKED;
static ap_client_event_t s_last_client_event = AP_CLIENT_EVENT_NONE;

static esp_err_t status_ui_panel_power_reset(void *) {
    Axp2101_SetAldo3(1);
    vTaskDelay(pdMS_TO_TICKS(100));
    Axp2101_SetAldo3(0);
    vTaskDelay(pdMS_TO_TICKS(100));
    Axp2101_SetAldo3(1);
    vTaskDelay(pdMS_TO_TICKS(100));
    return ESP_OK;
}

static void set_last_client_event(ap_client_event_t event) {
    portENTER_CRITICAL(&s_ap_event_lock);
    s_last_client_event = event;
    portEXIT_CRITICAL(&s_ap_event_lock);
}

static ap_client_event_t get_last_client_event() {
    portENTER_CRITICAL(&s_ap_event_lock);
    const ap_client_event_t event = s_last_client_event;
    portEXIT_CRITICAL(&s_ap_event_lock);
    return event;
}

static const char *last_client_event_text(ap_client_event_t event) {
    switch (event) {
    case AP_CLIENT_EVENT_CONNECTED:
        return "connected";
    case AP_CLIENT_EVENT_DISCONNECTED:
        return "disconnected";
    case AP_CLIENT_EVENT_NONE:
    default:
        return "none";
    }
}

static void publish_ap_status(esp_err_t sta_list_err, uint16_t client_count) {
    status_ui_snapshot_t ui = {};
    ui.level = sta_list_err == ESP_OK ? STATUS_UI_LEVEL_OK : STATUS_UI_LEVEL_ERROR;
    snprintf(ui.title, sizeof(ui.title), "Wi-Fi Access Point");
    snprintf(ui.lines[0], sizeof(ui.lines[0]), "AP ready");
    snprintf(ui.lines[1], sizeof(ui.lines[1]), "SSID: %s", kApSsid);
    snprintf(ui.lines[2], sizeof(ui.lines[2]), "Channel: %u", (unsigned)kApChannel);
    if (sta_list_err == ESP_OK) {
        snprintf(ui.lines[3], sizeof(ui.lines[3]), "Clients: %u", (unsigned)client_count);
    } else {
        snprintf(ui.lines[3], sizeof(ui.lines[3]), "Client count: read failed");
    }
    snprintf(ui.lines[4], sizeof(ui.lines[4]), "Last client: %s",
             last_client_event_text(get_last_client_event()));
    status_ui_publish(&ui);
}

static void WifiApStatusTask(void *) {
    while (true) {
        wifi_sta_list_t station_list = {};
        const esp_err_t err = esp_wifi_ap_get_sta_list(&station_list);
        publish_ap_status(err, station_list.num);
        vTaskDelay(pdMS_TO_TICKS(2000));
    }
}

void fac_wifi_default_init(void) {
    esp_err_t ret = nvs_flash_init();
    if (ret == ESP_ERR_NVS_NO_FREE_PAGES ||
        ret == ESP_ERR_NVS_NEW_VERSION_FOUND) {
        ESP_ERROR_CHECK(nvs_flash_erase());
        ret = nvs_flash_init();
    }
	esp_netif_init();                                // Initialize TCP/IP stack
    esp_event_loop_create_default();                 // Create default event loop
    esp_netif_create_default_wifi_sta();             // STA
    esp_netif_create_default_wifi_ap();              // AP
}

void FactoryWifiTestStartCallback(void *arg, esp_event_base_t event_base, int32_t event_id, void *event_data) {
    if (event_base == WIFI_EVENT && event_id == WIFI_EVENT_STA_START) {
        esp_wifi_connect();
    } else if (event_base == IP_EVENT && event_id == IP_EVENT_STA_GOT_IP) {
        auto *got_ip_event = (ip_event_got_ip_t *) event_data;
        uint32_t ip_addr = got_ip_event->ip_info.ip.addr;
        snprintf(wifi_ip, sizeof(wifi_ip), "%u.%u.%u.%u",
                (uint8_t)(ip_addr),
                (uint8_t)(ip_addr >> 8),
                (uint8_t)(ip_addr >> 16),
                (uint8_t)(ip_addr >> 24));
        ESP_LOGW(APP_NAME,"STA IP:%s",wifi_ip);
    } else if (event_base == WIFI_EVENT && event_id == WIFI_EVENT_STA_DISCONNECTED) {
        ESP_LOGW(APP_NAME,"STA Mode Device disconnected");
    } else if (event_base == WIFI_EVENT && event_id == WIFI_EVENT_AP_STACONNECTED) {
        wifi_event_ap_staconnected_t *event = (wifi_event_ap_staconnected_t *) event_data;
			ESP_LOGW(APP_NAME,"AP MAC:%02X:%02X:%02X:%02X:%02X:%02X",event->mac[0],event->mac[1],event->mac[2],event->mac[3],event->mac[4],event->mac[5]);
        set_last_client_event(AP_CLIENT_EVENT_CONNECTED);
    } else if (event_base == WIFI_EVENT && event_id == WIFI_EVENT_AP_STADISCONNECTED) {
        ESP_LOGW(APP_NAME,"AP Mode Device disconnected");
        set_last_client_event(AP_CLIENT_EVENT_DISCONNECTED);
    }
}

void fac_wifi_mode_init(bool sta_mode) {
    wifi_init_config_t cfg = WIFI_INIT_CONFIG_DEFAULT();
    esp_wifi_init(&cfg);
    esp_event_handler_instance_register(WIFI_EVENT, ESP_EVENT_ANY_ID, &FactoryWifiTestStartCallback, NULL, &wifi_event_instance);
    esp_event_handler_instance_register(IP_EVENT, IP_EVENT_STA_GOT_IP, &FactoryWifiTestStartCallback, NULL, &ip_event_instance);
    if(sta_mode) {
        wifi_config_t sta_config = {};
        strcpy((char*)sta_config.sta.ssid, "K2P");
        strcpy((char*)sta_config.sta.password, "1234567890");
        esp_wifi_set_mode(WIFI_MODE_STA);
        esp_wifi_set_config(WIFI_IF_STA, &sta_config);
        esp_wifi_start();
    } else {
        wifi_config_t ap_config = {};
        strcpy((char*)ap_config.ap.ssid, kApSsid);
        strcpy((char*)ap_config.ap.password, "12345678");
        ap_config.ap.channel = kApChannel;
        ap_config.ap.max_connection = 4;
        ap_config.ap.authmode = WIFI_AUTH_WPA_WPA2_PSK;
        esp_wifi_set_mode(WIFI_MODE_AP);
        esp_wifi_set_config(WIFI_IF_AP, &ap_config);
        esp_wifi_start();
    }
}

extern "C" void app_main(void) {
    Custom_PmicPortInit(&I2cMasterBus_, 0x34);
    status_ui_config_t ui_config = {};
    ui_config.panel_power_reset = status_ui_panel_power_reset;
    ui_config.panel_power_reset_context = NULL;
    const esp_err_t ui_err = status_ui_init(&ui_config);
    if (ui_err != ESP_OK) {
        ESP_LOGE("main", "Status UI unavailable: %s", esp_err_to_name(ui_err));
    }
    fac_wifi_default_init();
    fac_wifi_mode_init(0);
    xTaskCreate(WifiApStatusTask, "WifiApStatusTask", 3 * 1024, NULL, 3, NULL);
}

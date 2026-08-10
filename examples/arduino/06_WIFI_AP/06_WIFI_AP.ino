#include <C6AmoledBsp.h>
#include <esp_wifi.h>
#include <nvs_flash.h>

static c6_amoled::Board board;
static c6_amoled::StatusPage status_page;
static uint8_t client_count = 0;
static bool client_count_valid = false;
static const char *ap_status = "initializing";
static constexpr char kSsid[] = "ESP32_AP";
static constexpr uint8_t kChannel = 1;
static esp_event_handler_instance_t wifi_event_instance;

static void refresh_client_count() {
  wifi_sta_list_t stations = {};
  esp_err_t err = esp_wifi_ap_get_sta_list(&stations);
  if (err == ESP_OK) {
    client_count = stations.num;
    client_count_valid = true;
  } else {
    client_count_valid = false;
    Serial.printf("AP client query failed: %s\n", esp_err_to_name(err));
  }
}

static void publish_status() {
  c6_amoled::StatusPageData status = {};
  snprintf(status.title, sizeof(status.title), "WiFi AP");
  snprintf(status.lines[0], sizeof(status.lines[0]), "PMIC: %s", board.pmicReady() ? "ready" : "failed");
  snprintf(status.lines[1], sizeof(status.lines[1]), "Ready: %s", ap_status);
  snprintf(status.lines[2], sizeof(status.lines[2]), "SSID: %s", kSsid);
  snprintf(status.lines[3], sizeof(status.lines[3]), "Channel: %u", kChannel);
  if (client_count_valid) {
    snprintf(status.lines[4], sizeof(status.lines[4]), "Clients: %u", client_count);
  } else {
    snprintf(status.lines[4], sizeof(status.lines[4]), "Clients: unavailable");
  }
  status_page.publish(status);
}

static void wifi_event(void *, esp_event_base_t event_base, int32_t event_id, void *) {
  if (event_base != WIFI_EVENT) return;
  if (event_id == WIFI_EVENT_AP_START) ap_status = "ready";
  if (event_id == WIFI_EVENT_AP_STACONNECTED) ap_status = "client connected";
  if (event_id == WIFI_EVENT_AP_STADISCONNECTED) ap_status = "client disconnected";
  publish_status();
}

void setup() {
  Serial.begin(115200);
  delay(2000);
  board.begin();
  if (!status_page.begin(board)) Serial.println("Status UI unavailable; WiFi AP test continues.");
  esp_err_t err = nvs_flash_init();
  if (err == ESP_ERR_NVS_NO_FREE_PAGES || err == ESP_ERR_NVS_NEW_VERSION_FOUND) {
    nvs_flash_erase();
    err = nvs_flash_init();
  }
  if (err != ESP_OK) { ap_status = "NVS failed"; publish_status(); return; }
  esp_netif_init();
  esp_event_loop_create_default();
  esp_netif_create_default_wifi_ap();
  wifi_init_config_t config = WIFI_INIT_CONFIG_DEFAULT();
  if (esp_wifi_init(&config) != ESP_OK) { ap_status = "init failed"; publish_status(); return; }
  esp_event_handler_instance_register(WIFI_EVENT, ESP_EVENT_ANY_ID, wifi_event, NULL, &wifi_event_instance);
  wifi_config_t access_point = {};
  strcpy(reinterpret_cast<char *>(access_point.ap.ssid), kSsid);
  strcpy(reinterpret_cast<char *>(access_point.ap.password), "12345678");
  access_point.ap.channel = kChannel;
  access_point.ap.max_connection = 4;
  access_point.ap.authmode = WIFI_AUTH_WPA_WPA2_PSK;
  esp_wifi_set_mode(WIFI_MODE_AP);
  esp_wifi_set_config(WIFI_IF_AP, &access_point);
  ap_status = "starting";
  esp_wifi_start();
  publish_status();
}

void loop() {
  refresh_client_count();
  publish_status();
  delay(1000);
}

#include <C6AmoledBsp.h>
#include <esp_wifi.h>
#include <nvs_flash.h>

static c6_amoled::Board board;
static c6_amoled::StatusPage status_page;
static char wifi_ip[16] = {};
static const char *wifi_status = "initializing";
static constexpr char kSsid[] = "ESP32";
static esp_event_handler_instance_t wifi_event_instance;
static esp_event_handler_instance_t ip_event_instance;

static void publish_status() {
  c6_amoled::StatusPageData status = {};
  snprintf(status.title, sizeof(status.title), "WiFi STA");
  snprintf(status.lines[0], sizeof(status.lines[0]), "PMIC: %s", board.pmicReady() ? "ready" : "failed");
  snprintf(status.lines[1], sizeof(status.lines[1]), "SSID: %s", kSsid);
  snprintf(status.lines[2], sizeof(status.lines[2]), "Status: %s", wifi_status);
  snprintf(status.lines[3], sizeof(status.lines[3]), "IP: %s", wifi_ip[0] ? wifi_ip : "pending");
  status_page.publish(status);
}

static void wifi_event(void *, esp_event_base_t event_base, int32_t event_id, void *event_data) {
  if (event_base == WIFI_EVENT && event_id == WIFI_EVENT_STA_START) {
    wifi_status = "connecting";
    esp_wifi_connect();
  } else if (event_base == IP_EVENT && event_id == IP_EVENT_STA_GOT_IP) {
    const ip_event_got_ip_t *event = static_cast<ip_event_got_ip_t *>(event_data);
    uint32_t ip = event->ip_info.ip.addr;
    snprintf(wifi_ip, sizeof(wifi_ip), "%u.%u.%u.%u", (uint8_t)ip, (uint8_t)(ip >> 8), (uint8_t)(ip >> 16), (uint8_t)(ip >> 24));
    wifi_status = "connected";
    Serial.printf("STA IP: %s\n", wifi_ip);
  } else if (event_base == WIFI_EVENT && event_id == WIFI_EVENT_STA_DISCONNECTED) {
    wifi_ip[0] = '\0';
    wifi_status = "disconnected";
  }
  publish_status();
}

void setup() {
  Serial.begin(115200);
  delay(2000);
  board.begin();
  if (!status_page.begin(board)) Serial.println("Status UI unavailable; WiFi STA test continues.");
  esp_err_t err = nvs_flash_init();
  if (err == ESP_ERR_NVS_NO_FREE_PAGES || err == ESP_ERR_NVS_NEW_VERSION_FOUND) {
    nvs_flash_erase();
    err = nvs_flash_init();
  }
  if (err != ESP_OK) { wifi_status = "NVS failed"; publish_status(); return; }
  esp_netif_init();
  esp_event_loop_create_default();
  esp_netif_create_default_wifi_sta();
  wifi_init_config_t config = WIFI_INIT_CONFIG_DEFAULT();
  if (esp_wifi_init(&config) != ESP_OK) { wifi_status = "init failed"; publish_status(); return; }
  esp_event_handler_instance_register(WIFI_EVENT, ESP_EVENT_ANY_ID, wifi_event, NULL, &wifi_event_instance);
  esp_event_handler_instance_register(IP_EVENT, IP_EVENT_STA_GOT_IP, wifi_event, NULL, &ip_event_instance);
  wifi_config_t station = {};
  strcpy(reinterpret_cast<char *>(station.sta.ssid), kSsid);
  strcpy(reinterpret_cast<char *>(station.sta.password), "12345678");
  esp_wifi_set_mode(WIFI_MODE_STA);
  esp_wifi_set_config(WIFI_IF_STA, &station);
  wifi_status = "starting";
  esp_wifi_start();
  publish_status();
}

void loop() { delay(1000); }

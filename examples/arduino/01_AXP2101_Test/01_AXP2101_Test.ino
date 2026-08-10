#include <C6AmoledBsp.h>

static c6_amoled::Board board;
static c6_amoled::StatusPage status_page;

static void publish_status() {
  c6_amoled::PmicSnapshot pmic = board.pmicSnapshot();
  c6_amoled::StatusPageData status = {};
  snprintf(status.title, sizeof(status.title), "AXP2101 Power");
  snprintf(status.lines[0], sizeof(status.lines[0]), "PMIC: %s", pmic.ready ? "ready" : "failed");
  if (pmic.ready) {
    snprintf(status.lines[1], sizeof(status.lines[1]), "Charging: %s", pmic.charging ? "yes" : "no");
    snprintf(status.lines[2], sizeof(status.lines[2]), "Charger: %s", c6_amoled::chargerStatusText(pmic.charger_status));
    snprintf(status.lines[3], sizeof(status.lines[3]), "Battery: %u mV", pmic.battery_mv);
    Serial.printf("Charging: %s, charger: %s, battery: %u mV\n", pmic.charging ? "yes" : "no", c6_amoled::chargerStatusText(pmic.charger_status), pmic.battery_mv);
  }
  status_page.publish(status);
}

void setup() {
  Serial.begin(115200);
  delay(2000);
  board.begin();
  if (!status_page.begin(board)) Serial.println("Status UI unavailable; PMIC test continues.");
}

void loop() {
  publish_status();
  delay(2000);
}

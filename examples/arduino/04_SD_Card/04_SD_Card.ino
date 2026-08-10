#include <C6AmoledBsp.h>
#include "sdcard_bsp.h"

static c6_amoled::Board board;
static c6_amoled::StatusPage status_page;
static CustomSDPort *sd_card = NULL;

static void publish_status() {
  c6_amoled::StatusPageData status = {};
  snprintf(status.title, sizeof(status.title), "SD Card");
  snprintf(status.lines[0], sizeof(status.lines[0]), "PMIC: %s", board.pmicReady() ? "ready" : "failed");
  if (!sd_card || !sd_card->SDPort_GetStatus()) {
    snprintf(status.lines[1], sizeof(status.lines[1]), "Mount: failed");
  } else {
    sdmmc_card_t *card = sd_card->SDPort_GetCardHead();
    uint64_t capacity = static_cast<uint64_t>(card->csd.capacity) * card->csd.sector_size;
    snprintf(status.lines[1], sizeof(status.lines[1]), "Mount: ready");
    snprintf(status.lines[2], sizeof(status.lines[2]), "Type: %s", card->csd.csd_ver == 2 ? "SDHC/SDXC" : "SDSC");
    snprintf(status.lines[3], sizeof(status.lines[3]), "Capacity: %llu MB", capacity / (1024 * 1024));
    Serial.printf("SD: %s, %llu MB\n", status.lines[2], capacity / (1024 * 1024));
  }
  status_page.publish(status);
}

void setup() {
  Serial.begin(115200);
  delay(2000);
  board.begin();
  if (!status_page.begin(board)) Serial.println("Status UI unavailable; SD test continues.");
  sd_card = new CustomSDPort("/sdcard");
}

void loop() {
  publish_status();
  delay(2000);
}

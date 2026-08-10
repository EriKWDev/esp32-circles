#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "esp_err.h"

#ifdef __cplusplus
extern "C" {
#endif

#define STATUS_UI_TITLE_MAX_LEN 32
#define STATUS_UI_LINE_COUNT 5
#define STATUS_UI_LINE_MAX_LEN 48

typedef enum {
    STATUS_UI_LEVEL_INFO,
    STATUS_UI_LEVEL_OK,
    STATUS_UI_LEVEL_WARNING,
    STATUS_UI_LEVEL_ERROR,
} status_ui_level_t;

typedef struct {
    status_ui_level_t level;
    char title[STATUS_UI_TITLE_MAX_LEN];
    char lines[STATUS_UI_LINE_COUNT][STATUS_UI_LINE_MAX_LEN];
} status_ui_snapshot_t;

typedef esp_err_t (*status_ui_panel_power_reset_cb_t)(void *context);

typedef struct {
    status_ui_panel_power_reset_cb_t panel_power_reset;
    void *panel_power_reset_context;
} status_ui_config_t;

/**
 * Initializes the SH8601 panel and starts the private UI update task.
 * The caller supplies the board-specific panel power/reset sequence so this
 * component has no dependency on a board PMIC component.
 */
esp_err_t status_ui_init(const status_ui_config_t *config);

/**
 * Replaces the pending snapshot without blocking the caller.
 */
esp_err_t status_ui_publish(const status_ui_snapshot_t *snapshot);

#ifdef __cplusplus
}
#endif

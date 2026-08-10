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

/**
 * Starts the managed board display and the private UI update task.
 */
esp_err_t status_ui_init(void);

/**
 * Replaces the pending snapshot without blocking the caller.
 */
esp_err_t status_ui_publish(const status_ui_snapshot_t *snapshot);

#ifdef __cplusplus
}
#endif

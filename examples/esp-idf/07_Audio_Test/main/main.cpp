#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>

#include "freertos/FreeRTOS.h"
#include "freertos/event_groups.h"
#include "freertos/task.h"
#include "esp_codec_dev.h"
#include "esp_heap_caps.h"
#include "esp_log.h"

#include "bsp/esp-bsp.h"
#include "gui_guider.h"
#include "lvgl.h"

#define APP_NAME "audio"
#define REC_DATA_MAX 512
#define REC_SPIFFS_DATA_MAX 192000

namespace {

lv_ui s_ui;
uint8_t *s_audio_buffer = nullptr;
esp_codec_dev_handle_t s_speaker = nullptr;
esp_codec_dev_handle_t s_microphone = nullptr;
EventGroupHandle_t s_audio_events = nullptr;
bool s_music_enabled = false;

void update_status_label(const char *text)
{
    if (bsp_display_lock(500)) {
        lv_label_set_text(s_ui.screen_label_1, text);
        bsp_display_unlock();
    } else {
        ESP_LOGW(APP_NAME, "Display lock unavailable while setting status: %s", text);
    }
}

void audio_devices_init()
{
    s_speaker = bsp_audio_codec_speaker_init();
    s_microphone = bsp_audio_codec_microphone_init();
    if (s_speaker == nullptr || s_microphone == nullptr) {
        ESP_LOGE(APP_NAME, "Managed BSP audio codec initialization failed");
        abort();
    }

    esp_codec_dev_sample_info_t sample_info = {};
    sample_info.sample_rate = 16000;
    sample_info.channel = 2;
    sample_info.bits_per_sample = 16;
    ESP_ERROR_CHECK(esp_codec_dev_open(s_speaker, &sample_info));
    ESP_ERROR_CHECK(esp_codec_dev_open(s_microphone, &sample_info));
    ESP_ERROR_CHECK(esp_codec_dev_set_out_vol(s_speaker, 100));
    ESP_ERROR_CHECK(esp_codec_dev_set_in_gain(s_microphone, 35.0));

    s_audio_buffer = static_cast<uint8_t *>(heap_caps_malloc(REC_DATA_MAX, MALLOC_CAP_DEFAULT));
    if (s_audio_buffer == nullptr) {
        ESP_LOGE(APP_NAME, "Audio buffer allocation failed");
        abort();
    }
}

void record_to_spiffs(uint8_t *buffer, size_t max_size)
{
    FILE *file = fopen(BSP_SPIFFS_MOUNT_POINT "/rec.raw", "wb");
    if (file == nullptr) {
        ESP_LOGE(APP_NAME, "Failed to open recording file");
        return;
    }

    size_t total_written = 0;
    ESP_LOGI(APP_NAME, "Start recording");
    while (total_written < max_size) {
        const int err = esp_codec_dev_read(s_microphone, buffer, REC_DATA_MAX);
        if (err == ESP_CODEC_DEV_OK) {
            fwrite(buffer, 1, REC_DATA_MAX, file);
            total_written += REC_DATA_MAX;
        } else {
            ESP_LOGE(APP_NAME, "Failed to read microphone data: %d", err);
        }
    }
    fclose(file);
    ESP_LOGI(APP_NAME, "Recording completed, total %u bytes", (unsigned)total_written);
}

void play_recording(uint8_t *buffer)
{
    FILE *file = fopen(BSP_SPIFFS_MOUNT_POINT "/rec.raw", "rb");
    if (file == nullptr) {
        ESP_LOGE(APP_NAME, "Failed to open recording for playback");
        return;
    }

    size_t read_bytes = 0;
    while ((read_bytes = fread(buffer, 1, REC_DATA_MAX, file)) > 0) {
        (void)esp_codec_dev_write(s_speaker, buffer, read_bytes);
    }
    fclose(file);
}

esp_err_t play_pcm(uint8_t *buffer, const char *path)
{
    if (!s_music_enabled) {
        return ESP_OK;
    }
    FILE *file = fopen(path, "rb");
    if (file == nullptr) {
        ESP_LOGE(APP_NAME, "Failed to open PCM file: %s", path);
        return ESP_FAIL;
    }

    while (s_music_enabled) {
        const size_t read_bytes = fread(buffer, 1, REC_DATA_MAX, file);
        if (read_bytes > 0) {
            const int ret = esp_codec_dev_write(s_speaker, buffer, read_bytes);
            if (ret != ESP_CODEC_DEV_OK) {
                ESP_LOGE(APP_NAME, "Codec write failed: %d", ret);
                fclose(file);
                return ESP_FAIL;
            }
        }
        if (read_bytes < REC_DATA_MAX) {
            if (ferror(file)) {
                ESP_LOGE(APP_NAME, "PCM file read error");
                fclose(file);
                return ESP_FAIL;
            }
            break;
        }
    }
    fclose(file);
    return ESP_OK;
}

void audio_test_task(void *)
{
    bool has_recording = false;
    while (true) {
        const EventBits_t event = xEventGroupWaitBits(
            s_audio_events, 0x40 | 0x80 | 0x100, pdFALSE, pdFALSE, portMAX_DELAY);
        if ((event & 0x40) != 0) {
            update_status_label("Recording...");
            record_to_spiffs(s_audio_buffer, REC_SPIFFS_DATA_MAX);
            update_status_label("Recording done");
            has_recording = true;
        } else if ((event & 0x80) != 0 && has_recording) {
            update_status_label("Playing recording...");
            play_recording(s_audio_buffer);
            update_status_label("Playback finished");
        } else if ((event & 0x100) != 0) {
            update_status_label("Playing music...");
            (void)play_pcm(s_audio_buffer, BSP_SPIFFS_MOUNT_POINT "/mood.pcm");
            update_status_label("Playback finished");
        }
        xEventGroupClearBits(s_audio_events, 0x40 | 0x80 | 0x100);
    }
}

void audio_button_callback(lv_event_t *event)
{
    if (lv_event_get_code(event) != LV_EVENT_CLICKED) {
        return;
    }
    const intptr_t id = reinterpret_cast<intptr_t>(lv_event_get_user_data(event));
    switch (id) {
    case 4:
        xEventGroupSetBits(s_audio_events, 0x40);
        break;
    case 5:
        xEventGroupSetBits(s_audio_events, 0x80);
        break;
    case 6:
        s_music_enabled = true;
        xEventGroupSetBits(s_audio_events, 0x100);
        break;
    case 7:
        s_music_enabled = false;
        break;
    default:
        break;
    }
}

} // namespace

extern "C" void app_main(void)
{
    s_audio_events = xEventGroupCreate();
    if (s_audio_events == nullptr) {
        ESP_LOGE(APP_NAME, "Failed to create audio event group");
        return;
    }

    ESP_ERROR_CHECK(bsp_spiffs_mount());
    if (bsp_display_start() == nullptr) {
        ESP_LOGE(APP_NAME, "Managed BSP display initialization failed");
        return;
    }
    audio_devices_init();

    if (!bsp_display_lock(0)) {
        ESP_LOGE(APP_NAME, "Managed BSP display lock failed");
        return;
    }
    setup_ui(&s_ui);
    lv_obj_add_event_cb(s_ui.screen_btn_4, audio_button_callback, LV_EVENT_ALL,
                        reinterpret_cast<void *>(4));
    lv_obj_add_event_cb(s_ui.screen_btn_5, audio_button_callback, LV_EVENT_ALL,
                        reinterpret_cast<void *>(5));
    lv_obj_add_event_cb(s_ui.screen_btn_6, audio_button_callback, LV_EVENT_ALL,
                        reinterpret_cast<void *>(6));
    lv_obj_add_event_cb(s_ui.screen_btn_7, audio_button_callback, LV_EVENT_ALL,
                        reinterpret_cast<void *>(7));
    bsp_display_unlock();

    xTaskCreatePinnedToCore(audio_test_task, "audio_test", 3 * 1024,
                            nullptr, 3, nullptr, 0);
}

idf_component_register(
    SRCS "status_ui.cpp"
    INCLUDE_DIRS "include"
    REQUIRES
        esp_driver_spi
        esp_lcd
        espressif__esp_lcd_sh8601
        espressif__esp_lvgl_adapter
        freertos
        log
        lvgl__lvgl
)

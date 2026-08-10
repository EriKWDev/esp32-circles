idf_component_register(
    SRCS "status_ui.cpp"
    INCLUDE_DIRS "include"
    REQUIRES
        freertos
        log
        lvgl__lvgl
        waveshare__esp32_c6_touch_amoled_2_16
)

idf_component_register(
    SRCS "main.cpp"
    INCLUDE_DIRS "."
    REQUIRES status_ui esp_event esp_netif esp_wifi nvs_flash)

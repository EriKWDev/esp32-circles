idf_component_register(
    SRCS "XPowersLibInterface.cpp"
    INCLUDE_DIRS "." "REG"
    REQUIRES esp_driver_i2c
)

target_compile_definitions(${COMPONENT_LIB} PUBLIC
    XPOWERS_CHIP_AXP2101
    CONFIG_XPOWERS_ESP_IDF_NEW_API
)

idf_component_register(
    SRCS 
    "i2c_bsp.cpp" 
    "display_bsp.cpp" 
    PRIV_REQUIRES 
    driver
    pmicpower
    espressif__esp_lcd_sh8601
    waveshare__esp_lcd_touch_cst9217
    esp_timer
    INCLUDE_DIRS 
    "./")

if(IDF_VERSION_MAJOR GREATER_EQUAL 6)
    target_compile_options(${COMPONENT_LIB} PRIVATE -Wno-error=missing-field-initializers)
endif()

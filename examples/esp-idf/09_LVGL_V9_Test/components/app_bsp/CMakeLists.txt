idf_component_register(
    SRCS
    "lvgl_bsp.cpp"
    PRIV_REQUIRES
    espressif__esp_lvgl_adapter
    port_bsp
    lvgl__lvgl
    main
    INCLUDE_DIRS 
    "./")

if(IDF_VERSION_MAJOR GREATER_EQUAL 6)
    target_compile_options(${COMPONENT_LIB} PRIVATE -Wno-error=missing-field-initializers)
endif()

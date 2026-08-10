if(IDF_VERSION_MAJOR GREATER_EQUAL 6)
    idf_component_get_property(status_ui_idf_compat_lvgl_lib lvgl__lvgl COMPONENT_LIB)
    target_compile_options(${status_ui_idf_compat_lvgl_lib} PRIVATE -Wno-error=attributes)
endif()

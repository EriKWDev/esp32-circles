cmake_minimum_required(VERSION 3.16)
include($ENV{IDF_PATH}/tools/cmake/project.cmake)
project(09_LVGL_V9_Test)

if(IDF_VERSION_MAJOR GREATER_EQUAL 6)
    idf_component_get_property(lvgl_lib lvgl__lvgl COMPONENT_LIB)
    target_compile_options(${lvgl_lib} PRIVATE -Wno-error=attributes)
endif()

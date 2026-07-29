cmake_minimum_required(VERSION 3.16)
list(APPEND EXTRA_COMPONENT_DIRS
    "${CMAKE_CURRENT_LIST_DIR}/../../../components"
    "${CMAKE_CURRENT_LIST_DIR}/components/ExternLib"
)
include($ENV{IDF_PATH}/tools/cmake/project.cmake)
project(07_Audio_Test)

if(IDF_VERSION_MAJOR GREATER_EQUAL 6)
    idf_component_get_property(lvgl_lib lvgl__lvgl COMPONENT_LIB)
    target_compile_options(${lvgl_lib} PRIVATE -Wno-error=attributes)
endif()

# For more information about build system see
# https://docs.espressif.com/projects/esp-idf/en/latest/api-guides/build-system.html
# The following five lines of boilerplate have to be in your project's
# CMakeLists in this exact order for cmake to work correctly
cmake_minimum_required(VERSION 3.16)
list(APPEND EXTRA_COMPONENT_DIRS "${CMAKE_CURRENT_LIST_DIR}/../common/components")
include($ENV{IDF_PATH}/tools/cmake/project.cmake)
project(01_AXP2101_Test)
include("${CMAKE_CURRENT_LIST_DIR}/../common/status_ui_idf_compat.cmake")

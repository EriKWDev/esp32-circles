idf_component_register(
  SRCS "power_bsp.cpp"
  INCLUDE_DIRS "."
  REQUIRES port_bsp xpowers esp_driver_gpio
)

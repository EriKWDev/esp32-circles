idf_component_register(
    SRCS 
    "i2c_bsp.cpp" 
    PRIV_REQUIRES 
    esp_driver_i2c
    INCLUDE_DIRS ".")

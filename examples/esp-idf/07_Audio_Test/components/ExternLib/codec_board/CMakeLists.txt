
set(COMPONENT_PRIV_REQUIRE
    driver
    esp_codec_dev
    fatfs
    esp_lcd
    esp_driver_gpio
    esp_driver_i2c
    esp_driver_i2s
    esp_driver_sdmmc
    esp_driver_spi
)

idf_component_register(SRCS  "cfg_parse.c" "codec_board.c" "codec_init.c" "dummy_codec.c"
                             "lcd_init.c" "drv/tca9554.c"
                       INCLUDE_DIRS ./include
                       PRIV_INCLUDE_DIRS drv
                       REQUIRES "${COMPONENT_PRIV_REQUIRE}"
                       EMBED_TXTFILES "board_cfg.txt")

if(IDF_VERSION_MAJOR GREATER_EQUAL 6)
    target_compile_options(${COMPONENT_LIB} PRIVATE -Wno-error=missing-field-initializers)
endif()

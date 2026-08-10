#ifndef BOARD_CFG_H
#define BOARD_CFG_H

#include <c6_amoled_board_config.h>

#define C6_AMOLED_STRINGIFY_INNER(value) #value
#define C6_AMOLED_STRINGIFY(value) C6_AMOLED_STRINGIFY_INNER(value)

const char board_cfg_data[] = {
  "# support in, out, in_out type\n"
  "# support i2c_port, i2s_port settings\n"
  "# support pa_gain, i2c_addr setting\n"
  "\n"
  "Board: C6_AMOLED_2_16\n"
  "i2c: {sda: " C6_AMOLED_STRINGIFY(C6_AMOLED_I2C_SDA_NUM) ", scl: " C6_AMOLED_STRINGIFY(C6_AMOLED_I2C_SCL_NUM) "}\n"
  "i2s: {mclk: " C6_AMOLED_STRINGIFY(C6_AMOLED_I2S_MCLK_NUM) ", bclk: " C6_AMOLED_STRINGIFY(C6_AMOLED_I2S_BCLK_NUM) ", ws: " C6_AMOLED_STRINGIFY(C6_AMOLED_I2S_LRCK_NUM) ", din: " C6_AMOLED_STRINGIFY(C6_AMOLED_I2S_DIN_NUM) ", dout: " C6_AMOLED_STRINGIFY(C6_AMOLED_I2S_DOUT_NUM) "}\n"
  "out: {codec: ES8311, pa: -1, pa_gain: 6, use_mclk: 1, pa_gain:6}\n"
  "in: {codec: ES7210}\n"
};
const char *board_cfg_start = board_cfg_data;
const char *board_cfg_end = board_cfg_data + sizeof(board_cfg_data) - 1;  // -1去掉末尾\0

#endif

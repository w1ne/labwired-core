#pragma once

#include "cpu-defs.h"

typedef enum {
    R_0_32 = 0,
    R_1_32 = 1,
    R_2_32 = 2,
    R_3_32 = 3,
    R_4_32 = 4,
    R_5_32 = 5,
    R_6_32 = 6,
    R_7_32 = 7,
    R_8_32 = 8,
    R_9_32 = 9,
    R_10_32 = 10,
    R_11_32 = 11,
    R_12_32 = 12,
    R_13_32 = 13,
    R_14_32 = 14,
    R_15_32 = 15,
    R_16_32 = 16,
    R_17_32 = 17,
    R_18_32 = 18,
    R_19_32 = 19,
    R_20_32 = 20,
    R_21_32 = 21,
    R_22_32 = 22,
    R_23_32 = 23,
    R_24_32 = 24,
    R_25_32 = 25,
    R_26_32 = 26,
    R_27_32 = 27,
    R_28_32 = 28,
    R_29_32 = 29,
    R_30_32 = 30,
    R_31_32 = 31,
    ASR_16_32 = 37,
    ASR_17_32 = 38,
    ASR_18_32 = 39,
    ASR_19_32 = 40,
    ASR_20_32 = 41,
    ASR_21_32 = 42,
    ASR_22_32 = 43,
    ASR_23_32 = 44,
    ASR_24_32 = 45,
    ASR_25_32 = 46,
    ASR_26_32 = 47,
    ASR_27_32 = 48,
    ASR_28_32 = 49,
    ASR_29_32 = 50,
    ASR_30_32 = 51,
    ASR_31_32 = 52,
    Y_32 = 64,
    PSR_32 = 65,
    WIM_32 = 66,
    TBR_32 = 67,
    PC_32 = 68,
    NPC_32 = 69,
    FSR_32 = 70,
    CSR_32 = 71
} Registers;

//  Frame and stack pointers correspond to R30 and R14 respectively
#define FP_32 30
#define SP_32 14

#define RA                    R_7_32
#define RETURN_ADDRESS_OFFSET 8

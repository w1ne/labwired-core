#pragma once

/*
 * Instruction decode helpers
 *
 * Author: Sagar Karandikar, sagark@eecs.berkeley.edu
 *
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2 of the License, or (at your option) any later version.
 *
 * This library is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this library; if not, see <http://www.gnu.org/licenses/>.
 */

#define MASK_OP_MAJOR(op) (op & 0x7F)
enum {
    /* rv32i, rv64i, rv32m */
    OPC_RISC_LUI = (0x37),
    OPC_RISC_AUIPC = (0x17),
    OPC_RISC_JAL = (0x6F),
    OPC_RISC_JALR = (0x67),
    OPC_RISC_BRANCH = (0x63),
    OPC_RISC_LOAD = (0x03),
    OPC_RISC_STORE = (0x23),
    OPC_RISC_ARITH_IMM = (0x13),
    OPC_RISC_ARITH = (0x33),
    OPC_RISC_SYNCH = (0x0F),
    OPC_RISC_SYSTEM = (0x73),

    /* rv64i, rv64m */
    OPC_RISC_ARITH_IMM_W = (0x1B),
    OPC_RISC_ARITH_W = (0x3B),

    /* rv32a, rv64a */
    OPC_RISC_ATOMIC = (0x2F),

    /* floating point */
    OPC_RISC_FP_LOAD = (0x7),
    OPC_RISC_FP_STORE = (0x27),

    OPC_RISC_FMADD = (0x43),
    OPC_RISC_FMSUB = (0x47),
    OPC_RISC_FNMSUB = (0x4B),
    OPC_RISC_FNMADD = (0x4F),

    OPC_RISC_FP_ARITH = (0x53),

    OPC_RISC_V = (0x57),
};

#define MASK_OP_ARITH(op) (MASK_OP_MAJOR(op) | (op & ((0x7 << 12) | (0x7F << 25))))
enum {
    OPC_RISC_ADD = OPC_RISC_ARITH | (0x0 << 12) | (0x00 << 25),
    OPC_RISC_SUB = OPC_RISC_ARITH | (0x0 << 12) | (0x20 << 25),
    OPC_RISC_SLL = OPC_RISC_ARITH | (0x1 << 12) | (0x00 << 25),
    OPC_RISC_SLT = OPC_RISC_ARITH | (0x2 << 12) | (0x00 << 25),
    OPC_RISC_SLTU = OPC_RISC_ARITH | (0x3 << 12) | (0x00 << 25),
    OPC_RISC_XOR = OPC_RISC_ARITH | (0x4 << 12) | (0x00 << 25),
    OPC_RISC_SRL = OPC_RISC_ARITH | (0x5 << 12) | (0x00 << 25),
    OPC_RISC_SRA = OPC_RISC_ARITH | (0x5 << 12) | (0x20 << 25),
    OPC_RISC_OR = OPC_RISC_ARITH | (0x6 << 12) | (0x00 << 25),
    OPC_RISC_AND = OPC_RISC_ARITH | (0x7 << 12) | (0x00 << 25),

    /* RV64M */
    OPC_RISC_MUL = OPC_RISC_ARITH | (0x0 << 12) | (0x01 << 25),
    OPC_RISC_MULH = OPC_RISC_ARITH | (0x1 << 12) | (0x01 << 25),
    OPC_RISC_MULHSU = OPC_RISC_ARITH | (0x2 << 12) | (0x01 << 25),
    OPC_RISC_MULHU = OPC_RISC_ARITH | (0x3 << 12) | (0x01 << 25),

    OPC_RISC_DIV = OPC_RISC_ARITH | (0x4 << 12) | (0x01 << 25),
    OPC_RISC_DIVU = OPC_RISC_ARITH | (0x5 << 12) | (0x01 << 25),
    OPC_RISC_REM = OPC_RISC_ARITH | (0x6 << 12) | (0x01 << 25),
    OPC_RISC_REMU = OPC_RISC_ARITH | (0x7 << 12) | (0x01 << 25),

    /* Zba: */
    OPC_RISC_SH1ADD = OPC_RISC_ARITH | (0x2 << 12) | (0x10 << 25),
    OPC_RISC_SH2ADD = OPC_RISC_ARITH | (0x4 << 12) | (0x10 << 25),
    OPC_RISC_SH3ADD = OPC_RISC_ARITH | (0x6 << 12) | (0x10 << 25),

    /* Zbb: */
    OPC_RISC_ROL = OPC_RISC_ARITH | (0x1 << 12) | (0x30 << 25),
    OPC_RISC_ANDN = OPC_RISC_ARITH | (0x7 << 12) | (0x20 << 25),
    OPC_RISC_XNOR = OPC_RISC_ARITH | (0x4 << 12) | (0x20 << 25),
    OPC_RISC_MIN = OPC_RISC_ARITH | (0x4 << 12) | (0x05 << 25),
    OPC_RISC_MINU = OPC_RISC_ARITH | (0x5 << 12) | (0x05 << 25),
    OPC_RISC_ROR = OPC_RISC_ARITH | (0x5 << 12) | (0x30 << 25),
    OPC_RISC_ORN = OPC_RISC_ARITH | (0x6 << 12) | (0x20 << 25),
    OPC_RISC_MAX = OPC_RISC_ARITH | (0x6 << 12) | (0x05 << 25),
    OPC_RISC_MAXU = OPC_RISC_ARITH | (0x7 << 12) | (0x05 << 25),

    /* Zbc: */
    OPC_RISC_CLMUL = OPC_RISC_ARITH | (0x1 << 12) | (0x05 << 25),
    OPC_RISC_CLMULR = OPC_RISC_ARITH | (0x2 << 12) | (0x05 << 25),
    OPC_RISC_CLMULH = OPC_RISC_ARITH | (0x3 << 12) | (0x05 << 25),

    /* Zbs: */
    OPC_RISC_BSET = OPC_RISC_ARITH | (0x1 << 12) | (0x14 << 25),
    OPC_RISC_BCLR = OPC_RISC_ARITH | (0x1 << 12) | (0x24 << 25),
    OPC_RISC_BINV = OPC_RISC_ARITH | (0x1 << 12) | (0x34 << 25),
    OPC_RISC_BEXT = OPC_RISC_ARITH | (0x5 << 12) | (0x24 << 25),

    /* Zicond: */
    OPC_RISC_CZERO_EQZ = OPC_RISC_ARITH | (0x5 << 12) | (0x07 << 25),
    OPC_RISC_CZERO_NEZ = OPC_RISC_ARITH | (0x7 << 12) | (0x07 << 25),

    /* Zbkb: */
    OPC_RISC_PACK = OPC_RISC_ARITH | (0x4 << 12) | (0x4 << 25),
    OPC_RISC_PACKH = OPC_RISC_ARITH | (0x7 << 12) | (0x4 << 25),
};

#define MASK_OP_FENCE(op) (MASK_OP_MAJOR(op) | (op & (0x7 << 12)))
enum { OPC_RISC_FENCE = OPC_RISC_SYNCH | (0x0 << 12), OPC_RISC_FENCE_I = OPC_RISC_SYNCH | (0x1 << 12) };

#define MASK_OP_ARITH_IMM(op) (MASK_OP_MAJOR(op) | (op & (0x7 << 12)))
enum {
    OPC_RISC_ADDI = OPC_RISC_ARITH_IMM | (0x0 << 12),
    OPC_RISC_SLTI = OPC_RISC_ARITH_IMM | (0x2 << 12),
    OPC_RISC_SLTIU = OPC_RISC_ARITH_IMM | (0x3 << 12),
    OPC_RISC_XORI = OPC_RISC_ARITH_IMM | (0x4 << 12),
    OPC_RISC_ORI = OPC_RISC_ARITH_IMM | (0x6 << 12),
    OPC_RISC_ANDI = OPC_RISC_ARITH_IMM | (0x7 << 12),
    OPC_RISC_SLLI = OPC_RISC_ARITH_IMM | (0x1 << 12),         /* additional part of
                                                                 IMM */
    OPC_RISC_SHIFT_RIGHT_I = OPC_RISC_ARITH_IMM | (0x5 << 12) /* SRAI, SRLI, Zbb extensions */
};

#define MASK_OP_ARITH_IMM_ZB_1_12(op) (MASK_OP_MAJOR(op) | (op & (0x7 << 12)) | (op & (0xFFF << 20)))
enum {
    /* Zbb: */
    OPC_RISC_CLZ = OPC_RISC_ARITH_IMM | (0x1 << 12) | (0x600 << 20),
    OPC_RISC_CTZ = OPC_RISC_ARITH_IMM | (0x1 << 12) | (0x601 << 20),
    OPC_RISC_CPOP = OPC_RISC_ARITH_IMM | (0x1 << 12) | (0x602 << 20),
    OPC_RISC_SEXT_B = OPC_RISC_ARITH_IMM | (0x1 << 12) | (0x604 << 20),

    /* Zbkb: */
    OPC_RISC_ZIP = OPC_RISC_ARITH_IMM | (0x1 << 12) | (0x8F << 20),

    OPC_RISC_SEXT_H = OPC_RISC_ARITH_IMM | (0x1 << 12) | (0x605 << 20),
};

#define MASK_OP_ARITH_IMM_ZB_1_12_SHAMT(op) (MASK_OP_MAJOR(op) | (op & (0x7 << 12)) | (op & (0x3F << 26)))
enum {
    /* Zbs: */
    OPC_RISC_BSETI = OPC_RISC_ARITH_IMM | (0x1 << 12) | (0x0A << 26),
    OPC_RISC_BCLRI = OPC_RISC_ARITH_IMM | (0x1 << 12) | (0x12 << 26),
    OPC_RISC_BINVI = OPC_RISC_ARITH_IMM | (0x1 << 12) | (0x1A << 26),

};

#define MASK_OP_ARITH_IMM_ZB_5_12(op) (MASK_OP_MAJOR(op) | (op & (0x7 << 12)) | (op & (0xFFF << 20)))
enum {
    /* Zbb: */
    OPC_RISC_ORC_B = OPC_RISC_ARITH_IMM | (0x5 << 12) | (0x287 << 20),
    OPC_RISC_REV8_32 = OPC_RISC_ARITH_IMM | (0x5 << 12) | (0x698 << 20),
    OPC_RISC_REV8_64 = OPC_RISC_ARITH_IMM | (0x5 << 12) | (0x6B8 << 20),
    OPC_RISC_BREV8 = OPC_RISC_ARITH_IMM | (0x5 << 12) | (0x687 << 20),

    /* Zbkb: */
    OPC_RISC_UNZIP = OPC_RISC_ARITH_IMM | (0x5 << 12) | (0x8F << 20),
};

#define MASK_OP_ARITH_IMM_ZB_5_12_SHAMT(op) (MASK_OP_MAJOR(op) | (op & (0x7 << 12)) | (op & (0x3F << 26)))
enum {
    /* Zbb: */
    OPC_RISC_RORI = OPC_RISC_ARITH_IMM | (0x5 << 12) | (0x18 << 26),

    /* Zbs: */
    OPC_RISC_BEXTI = OPC_RISC_ARITH_IMM | (0x5 << 12) | (0x12 << 26),
};

#define MASK_OP_ARITH_IMM_ZB_5_12_SHAMT_LAST_7(op) (MASK_OP_MAJOR(op) | (op & (0x7 << 12)) | (op & (0x3F << 25)))

#define MASK_OP_BRANCH(op) (MASK_OP_MAJOR(op) | (op & (0x7 << 12)))
enum {
    OPC_RISC_BEQ = OPC_RISC_BRANCH | (0x0 << 12),
    OPC_RISC_BNE = OPC_RISC_BRANCH | (0x1 << 12),
    OPC_RISC_BLT = OPC_RISC_BRANCH | (0x4 << 12),
    OPC_RISC_BGE = OPC_RISC_BRANCH | (0x5 << 12),
    OPC_RISC_BLTU = OPC_RISC_BRANCH | (0x6 << 12),
    OPC_RISC_BGEU = OPC_RISC_BRANCH | (0x7 << 12)
};

enum {
    OPC_RISC_ADDIW = OPC_RISC_ARITH_IMM_W | (0x0 << 12),
    OPC_RISC_SLLIW = OPC_RISC_ARITH_IMM_W | (0x1 << 12),          /* additional part of
                                                                     IMM */
    OPC_RISC_SHIFT_RIGHT_IW = OPC_RISC_ARITH_IMM_W | (0x5 << 12), /* SRAI, SRLI,
                                                                     RORIW */

    /* Zba: */
    OPC_RISC_SLLI_UW = OPC_RISC_ARITH_IMM_W | (0x1 << 12) | (0x1 << 27),

    /* Zbb: */
    OPC_RISC_CLZW = OPC_RISC_ARITH_IMM_W | (0x1 << 12) | (0x600 << 20),
    OPC_RISC_CTZW = OPC_RISC_ARITH_IMM_W | (0x1 << 12) | (0x601 << 20),
    OPC_RISC_CPOPW = OPC_RISC_ARITH_IMM_W | (0x1 << 12) | (0x602 << 20),

    OPC_RISC_RORIW = OPC_RISC_ARITH_IMM_W | (0x5 << 12) | (0x30 << 25)
};

enum {
    OPC_RISC_ADDW = OPC_RISC_ARITH_W | (0x0 << 12) | (0x00 << 25),
    OPC_RISC_SUBW = OPC_RISC_ARITH_W | (0x0 << 12) | (0x20 << 25),
    OPC_RISC_SLLW = OPC_RISC_ARITH_W | (0x1 << 12) | (0x00 << 25),
    OPC_RISC_SRLW = OPC_RISC_ARITH_W | (0x5 << 12) | (0x00 << 25),
    OPC_RISC_SRAW = OPC_RISC_ARITH_W | (0x5 << 12) | (0x20 << 25),

    /* RV64M */
    OPC_RISC_MULW = OPC_RISC_ARITH_W | (0x0 << 12) | (0x01 << 25),
    OPC_RISC_DIVW = OPC_RISC_ARITH_W | (0x4 << 12) | (0x01 << 25),
    OPC_RISC_DIVUW = OPC_RISC_ARITH_W | (0x5 << 12) | (0x01 << 25),
    OPC_RISC_REMW = OPC_RISC_ARITH_W | (0x6 << 12) | (0x01 << 25),
    OPC_RISC_REMUW = OPC_RISC_ARITH_W | (0x7 << 12) | (0x01 << 25),

    /* Zba: */
    OPC_RISC_ADD_UW = OPC_RISC_ARITH_W | (0x0 << 12) | (0x04 << 25),
    OPC_RISC_SH1ADD_UW = OPC_RISC_ARITH_W | (0x2 << 12) | (0x10 << 25),
    OPC_RISC_SH2ADD_UW = OPC_RISC_ARITH_W | (0x4 << 12) | (0x10 << 25),
    OPC_RISC_SH3ADD_UW = OPC_RISC_ARITH_W | (0x6 << 12) | (0x10 << 25),

    /* Zbb: */
    OPC_RISC_ZEXT_H_64 = OPC_RISC_ARITH_W | (0x4 << 12) | (0x80 << 20),
    OPC_RISC_PACKW = OPC_RISC_ARITH_W | (0x4 << 12) | (0x4 << 25),

    OPC_RISC_ROLW = OPC_RISC_ARITH_W | (0x1 << 12) | (0x30 << 25),
    OPC_RISC_RORW = OPC_RISC_ARITH_W | (0x5 << 12) | (0x30 << 25),
};

#define MASK_OP_LOAD(op) (MASK_OP_MAJOR(op) | (op & (0x7 << 12)))
enum {
    OPC_RISC_LB = OPC_RISC_LOAD | (0x0 << 12),
    OPC_RISC_LH = OPC_RISC_LOAD | (0x1 << 12),
    OPC_RISC_LW = OPC_RISC_LOAD | (0x2 << 12),
    OPC_RISC_LD = OPC_RISC_LOAD | (0x3 << 12),
    OPC_RISC_LBU = OPC_RISC_LOAD | (0x4 << 12),
    OPC_RISC_LHU = OPC_RISC_LOAD | (0x5 << 12),
    OPC_RISC_LWU = OPC_RISC_LOAD | (0x6 << 12),
    OPC_RISC_LDU = OPC_RISC_LOAD | (0x7 << 12),
};

#define MASK_OP_STORE(op) (MASK_OP_MAJOR(op) | (op & (0x7 << 12)))
enum {
    OPC_RISC_SB = OPC_RISC_STORE | (0x0 << 12),
    OPC_RISC_SH = OPC_RISC_STORE | (0x1 << 12),
    OPC_RISC_SW = OPC_RISC_STORE | (0x2 << 12),
    OPC_RISC_SD = OPC_RISC_STORE | (0x3 << 12),
};

#define MASK_OP_JALR(op) (MASK_OP_MAJOR(op) | (op & (0x7 << 12)))
/* no enum since OPC_RISC_JALR is the actual value */

#define MASK_OP_ATOMIC(op)          (MASK_OP_MAJOR(op) | (op & ((0x7 << 12) | (0x7F << 25))))
#define MASK_OP_ATOMIC_NO_AQ_RL(op) (MASK_OP_MAJOR(op) | (op & ((0x7 << 12) | (0x1F << 27))))

#define MASK_FUNCT5(op) extract32(op, 27, 5)
enum {
    FUNCT5_LR = 0x02,
    FUNCT5_SC = 0x03,
    FUNCT5_AMOSWAP = 0x01,
    FUNCT5_AMOADD = 0x00,
    FUNCT5_AMOXOR = 0x04,
    FUNCT5_AMOAND = 0x0C,
    FUNCT5_AMOOR = 0x08,
    FUNCT5_AMOMIN = 0x10,
    FUNCT5_AMOMAX = 0x14,
    FUNCT5_AMOMINU = 0x18,
    FUNCT5_AMOMAXU = 0x1C,
    FUNCT5_AMOCAS = 0x05,
};

#define MASK_FUNCT3(op) extract32(op, 12, 3)
enum {
    FUNCT3_WORD = 0x2,
    FUNCT3_DOUBLEWORD = 0x3,
    FUNCT3_QUADWORD = 0x4,
};

enum {
    OPC_RISC_LR_W = OPC_RISC_ATOMIC | FUNCT3_WORD << 12 | FUNCT5_LR << 27,
    OPC_RISC_SC_W = OPC_RISC_ATOMIC | FUNCT3_WORD << 12 | FUNCT5_SC << 27,
    OPC_RISC_AMOSWAP_W = OPC_RISC_ATOMIC | FUNCT3_WORD << 12 | FUNCT5_AMOSWAP << 27,
    OPC_RISC_AMOADD_W = OPC_RISC_ATOMIC | FUNCT3_WORD << 12 | FUNCT5_AMOADD << 27,
    OPC_RISC_AMOXOR_W = OPC_RISC_ATOMIC | FUNCT3_WORD << 12 | FUNCT5_AMOXOR << 27,
    OPC_RISC_AMOAND_W = OPC_RISC_ATOMIC | FUNCT3_WORD << 12 | FUNCT5_AMOAND << 27,
    OPC_RISC_AMOOR_W = OPC_RISC_ATOMIC | FUNCT3_WORD << 12 | FUNCT5_AMOOR << 27,
    OPC_RISC_AMOMIN_W = OPC_RISC_ATOMIC | FUNCT3_WORD << 12 | FUNCT5_AMOMIN << 27,
    OPC_RISC_AMOMAX_W = OPC_RISC_ATOMIC | FUNCT3_WORD << 12 | FUNCT5_AMOMAX << 27,
    OPC_RISC_AMOMINU_W = OPC_RISC_ATOMIC | FUNCT3_WORD << 12 | FUNCT5_AMOMINU << 27,
    OPC_RISC_AMOMAXU_W = OPC_RISC_ATOMIC | FUNCT3_WORD << 12 | FUNCT5_AMOMAXU << 27,

    OPC_RISC_LR_D = OPC_RISC_ATOMIC | FUNCT3_DOUBLEWORD << 12 | FUNCT5_LR << 27,
    OPC_RISC_SC_D = OPC_RISC_ATOMIC | FUNCT3_DOUBLEWORD << 12 | FUNCT5_SC << 27,
    OPC_RISC_AMOSWAP_D = OPC_RISC_ATOMIC | FUNCT3_DOUBLEWORD << 12 | FUNCT5_AMOSWAP << 27,
    OPC_RISC_AMOADD_D = OPC_RISC_ATOMIC | FUNCT3_DOUBLEWORD << 12 | FUNCT5_AMOADD << 27,
    OPC_RISC_AMOXOR_D = OPC_RISC_ATOMIC | FUNCT3_DOUBLEWORD << 12 | FUNCT5_AMOXOR << 27,
    OPC_RISC_AMOAND_D = OPC_RISC_ATOMIC | FUNCT3_DOUBLEWORD << 12 | FUNCT5_AMOAND << 27,
    OPC_RISC_AMOOR_D = OPC_RISC_ATOMIC | FUNCT3_DOUBLEWORD << 12 | FUNCT5_AMOOR << 27,
    OPC_RISC_AMOMIN_D = OPC_RISC_ATOMIC | FUNCT3_DOUBLEWORD << 12 | FUNCT5_AMOMIN << 27,
    OPC_RISC_AMOMAX_D = OPC_RISC_ATOMIC | FUNCT3_DOUBLEWORD << 12 | FUNCT5_AMOMAX << 27,
    OPC_RISC_AMOMINU_D = OPC_RISC_ATOMIC | FUNCT3_DOUBLEWORD << 12 | FUNCT5_AMOMINU << 27,
    OPC_RISC_AMOMAXU_D = OPC_RISC_ATOMIC | FUNCT3_DOUBLEWORD << 12 | FUNCT5_AMOMAXU << 27,
};

enum {
    OPC_RISC_AMOCAS = OPC_RISC_ATOMIC | FUNCT5_AMOCAS << 27,

    OPC_RISC_AMOCAS_W = OPC_RISC_AMOCAS | FUNCT3_WORD << 12,
    OPC_RISC_AMOCAS_D = OPC_RISC_AMOCAS | FUNCT3_DOUBLEWORD << 12,
    OPC_RISC_AMOCAS_Q = OPC_RISC_AMOCAS | FUNCT3_QUADWORD << 12,
};

#define MASK_OP_SYSTEM(op) (MASK_OP_MAJOR(op) | (op & (0x7 << 12)))
enum {
    OPC_RISC_ECALL = OPC_RISC_SYSTEM | (0x0 << 12),
    OPC_RISC_EBREAK = OPC_RISC_SYSTEM | (0x0 << 12),
    OPC_RISC_ERET = OPC_RISC_SYSTEM | (0x0 << 12),
    OPC_RISC_MRTS = OPC_RISC_SYSTEM | (0x0 << 12),
    OPC_RISC_MRTH = OPC_RISC_SYSTEM | (0x0 << 12),
    OPC_RISC_HRTS = OPC_RISC_SYSTEM | (0x0 << 12),
    OPC_RISC_WFI = OPC_RISC_SYSTEM | (0x0 << 12),
    OPC_RISC_SFENCEVM = OPC_RISC_SYSTEM | (0x0 << 12),

    OPC_RISC_CSRRW = OPC_RISC_SYSTEM | (0x1 << 12),
    OPC_RISC_CSRRS = OPC_RISC_SYSTEM | (0x2 << 12),
    OPC_RISC_CSRRC = OPC_RISC_SYSTEM | (0x3 << 12),
    OPC_RISC_CSRRWI = OPC_RISC_SYSTEM | (0x5 << 12),
    OPC_RISC_CSRRSI = OPC_RISC_SYSTEM | (0x6 << 12),
    OPC_RISC_CSRRCI = OPC_RISC_SYSTEM | (0x7 << 12),
};

#define MASK_OP_FP_LOAD(op) (MASK_OP_MAJOR(op) | (op & (0x7 << 12)))
enum {
    OPC_RISC_FLH = OPC_RISC_FP_LOAD | (0x1 << 12),
    OPC_RISC_FLW = OPC_RISC_FP_LOAD | (0x2 << 12),
    OPC_RISC_FLD = OPC_RISC_FP_LOAD | (0x3 << 12),
};

#define MASK_OP_V_LOAD(op) (MASK_OP_MAJOR(op) | (op & (0x3 << 26)))
enum {
    OPC_RISC_VL_US = OPC_RISC_FP_LOAD | (0x0 << 26),
    OPC_RISC_VL_UVI = OPC_RISC_FP_LOAD | (0x1 << 26),
    OPC_RISC_VL_VS = OPC_RISC_FP_LOAD | (0x2 << 26),
    OPC_RISC_VL_OVI = OPC_RISC_FP_LOAD | (0x3 << 26),
};

#define MASK_OP_V_LOAD_US(op) (MASK_OP_V_LOAD(op) | (op & (0x1F << 20)))
enum {
    OPC_RISC_VL_US_WR = OPC_RISC_VL_US | (0x8 << 20),
    OPC_RISC_VL_US_MASK = OPC_RISC_VL_US | (0xB << 20),
    OPC_RISC_VL_US_FOF = OPC_RISC_VL_US | (0x10 << 20),
};

#define MASK_OP_FP_STORE(op) (MASK_OP_MAJOR(op) | (op & (0x7 << 12)))
enum {
    OPC_RISC_FSH = OPC_RISC_FP_STORE | (0x1 << 12),
    OPC_RISC_FSW = OPC_RISC_FP_STORE | (0x2 << 12),
    OPC_RISC_FSD = OPC_RISC_FP_STORE | (0x3 << 12),
};

#define MASK_OP_V_STORE(op) (MASK_OP_MAJOR(op) | (op & (0x3 << 26)))
enum {
    OPC_RISC_VS_US = OPC_RISC_FP_STORE | (0x0 << 26),
    OPC_RISC_VS_UVI = OPC_RISC_FP_STORE | (0x1 << 26),
    OPC_RISC_VS_VS = OPC_RISC_FP_STORE | (0x2 << 26),
    OPC_RISC_VS_OVI = OPC_RISC_FP_STORE | (0x3 << 26),
};

#define MASK_OP_V_STORE_US(op) (MASK_OP_V_STORE(op) | (op & (0x1F << 20)))
enum {
    OPC_RISC_VS_US_WR = OPC_RISC_VS_US | (0x8 << 20),
    OPC_RISC_VS_US_MASK = OPC_RISC_VS_US | (0xB << 20),
};

#define MASK_OP_FP_FMADD(op) (MASK_OP_MAJOR(op) | (op & (0x3 << 25)))
enum {
    OPC_RISC_FMADD_S = OPC_RISC_FMADD | (0x0 << 25),
    OPC_RISC_FMADD_D = OPC_RISC_FMADD | (0x1 << 25),
    OPC_RISC_FMADD_H = OPC_RISC_FMADD | (0x2 << 25),
};

#define MASK_OP_FP_FMSUB(op) (MASK_OP_MAJOR(op) | (op & (0x3 << 25)))
enum {
    OPC_RISC_FMSUB_S = OPC_RISC_FMSUB | (0x0 << 25),
    OPC_RISC_FMSUB_D = OPC_RISC_FMSUB | (0x1 << 25),
    OPC_RISC_FMSUB_H = OPC_RISC_FMSUB | (0x2 << 25),
};

#define MASK_OP_FP_FNMADD(op) (MASK_OP_MAJOR(op) | (op & (0x3 << 25)))
enum {
    OPC_RISC_FNMADD_S = OPC_RISC_FNMADD | (0x0 << 25),
    OPC_RISC_FNMADD_D = OPC_RISC_FNMADD | (0x1 << 25),
    OPC_RISC_FNMADD_H = OPC_RISC_FNMADD | (0x2 << 25),
};

#define MASK_OP_FP_FNMSUB(op) (MASK_OP_MAJOR(op) | (op & (0x3 << 25)))
enum {
    OPC_RISC_FNMSUB_S = OPC_RISC_FNMSUB | (0x0 << 25),
    OPC_RISC_FNMSUB_D = OPC_RISC_FNMSUB | (0x1 << 25),
    OPC_RISC_FNMSUB_H = OPC_RISC_FNMSUB | (0x2 << 25),
};

#define MASK_OP_FP_ARITH(op) (MASK_OP_MAJOR(op) | (op & (0x7F << 25)))
enum {
    /* float */
    OPC_RISC_FADD_S = OPC_RISC_FP_ARITH | (0x0 << 25),
    OPC_RISC_FSUB_S = OPC_RISC_FP_ARITH | (0x4 << 25),
    OPC_RISC_FMUL_S = OPC_RISC_FP_ARITH | (0x8 << 25),
    OPC_RISC_FDIV_S = OPC_RISC_FP_ARITH | (0xC << 25),

    OPC_RISC_FSGNJ_S = OPC_RISC_FP_ARITH | (0x10 << 25),
    OPC_RISC_FSGNJN_S = OPC_RISC_FP_ARITH | (0x10 << 25),
    OPC_RISC_FSGNJX_S = OPC_RISC_FP_ARITH | (0x10 << 25),

    OPC_RISC_FMIN_S = OPC_RISC_FP_ARITH | (0x14 << 25),
    OPC_RISC_FMAX_S = OPC_RISC_FP_ARITH | (0x14 << 25),

    OPC_RISC_FSQRT_S = OPC_RISC_FP_ARITH | (0x2C << 25),

    OPC_RISC_FEQ_S = OPC_RISC_FP_ARITH | (0x50 << 25),
    OPC_RISC_FLT_S = OPC_RISC_FP_ARITH | (0x50 << 25),
    OPC_RISC_FLE_S = OPC_RISC_FP_ARITH | (0x50 << 25),

    OPC_RISC_FCVT_W_S = OPC_RISC_FP_ARITH | (0x60 << 25),
    OPC_RISC_FCVT_WU_S = OPC_RISC_FP_ARITH | (0x60 << 25),
    OPC_RISC_FCVT_L_S = OPC_RISC_FP_ARITH | (0x60 << 25),
    OPC_RISC_FCVT_LU_S = OPC_RISC_FP_ARITH | (0x60 << 25),

    OPC_RISC_FCVT_S_W = OPC_RISC_FP_ARITH | (0x68 << 25),
    OPC_RISC_FCVT_S_WU = OPC_RISC_FP_ARITH | (0x68 << 25),
    OPC_RISC_FCVT_S_L = OPC_RISC_FP_ARITH | (0x68 << 25),
    OPC_RISC_FCVT_S_LU = OPC_RISC_FP_ARITH | (0x68 << 25),

    OPC_RISC_FMV_X_S = OPC_RISC_FP_ARITH | (0x70 << 25),
    OPC_RISC_FCLASS_S = OPC_RISC_FP_ARITH | (0x70 << 25),

    OPC_RISC_FMV_S_X = OPC_RISC_FP_ARITH | (0x78 << 25),

    /* double */
    OPC_RISC_FADD_D = OPC_RISC_FP_ARITH | (0x1 << 25),
    OPC_RISC_FSUB_D = OPC_RISC_FP_ARITH | (0x5 << 25),
    OPC_RISC_FMUL_D = OPC_RISC_FP_ARITH | (0x9 << 25),
    OPC_RISC_FDIV_D = OPC_RISC_FP_ARITH | (0xD << 25),

    OPC_RISC_FSGNJ_D = OPC_RISC_FP_ARITH | (0x11 << 25),
    OPC_RISC_FSGNJN_D = OPC_RISC_FP_ARITH | (0x11 << 25),
    OPC_RISC_FSGNJX_D = OPC_RISC_FP_ARITH | (0x11 << 25),

    OPC_RISC_FMIN_D = OPC_RISC_FP_ARITH | (0x15 << 25),
    OPC_RISC_FMAX_D = OPC_RISC_FP_ARITH | (0x15 << 25),

    OPC_RISC_FCVT_S_D = OPC_RISC_FP_ARITH | (0x20 << 25),

    OPC_RISC_FCVT_D_S = OPC_RISC_FP_ARITH | (0x21 << 25),

    OPC_RISC_FSQRT_D = OPC_RISC_FP_ARITH | (0x2D << 25),

    OPC_RISC_FEQ_D = OPC_RISC_FP_ARITH | (0x51 << 25),
    OPC_RISC_FLT_D = OPC_RISC_FP_ARITH | (0x51 << 25),
    OPC_RISC_FLE_D = OPC_RISC_FP_ARITH | (0x51 << 25),

    OPC_RISC_FCVT_W_D = OPC_RISC_FP_ARITH | (0x61 << 25),
    OPC_RISC_FCVT_WU_D = OPC_RISC_FP_ARITH | (0x61 << 25),
    OPC_RISC_FCVT_L_D = OPC_RISC_FP_ARITH | (0x61 << 25),
    OPC_RISC_FCVT_LU_D = OPC_RISC_FP_ARITH | (0x61 << 25),

    OPC_RISC_FCVT_D_W = OPC_RISC_FP_ARITH | (0x69 << 25),
    OPC_RISC_FCVT_D_WU = OPC_RISC_FP_ARITH | (0x69 << 25),
    OPC_RISC_FCVT_D_L = OPC_RISC_FP_ARITH | (0x69 << 25),
    OPC_RISC_FCVT_D_LU = OPC_RISC_FP_ARITH | (0x69 << 25),

    OPC_RISC_FMV_X_D = OPC_RISC_FP_ARITH | (0x71 << 25),
    OPC_RISC_FCLASS_D = OPC_RISC_FP_ARITH | (0x71 << 25),

    OPC_RISC_FMV_D_X = OPC_RISC_FP_ARITH | (0x79 << 25),

    /* half-precision */
    OPC_RISC_FADD_H = OPC_RISC_FP_ARITH | (0x2 << 25),
    OPC_RISC_FSUB_H = OPC_RISC_FP_ARITH | (0x6 << 25),
    OPC_RISC_FMUL_H = OPC_RISC_FP_ARITH | (0xA << 25),
    OPC_RISC_FDIV_H = OPC_RISC_FP_ARITH | (0xE << 25),

    OPC_RISC_FSGNJ_H = OPC_RISC_FP_ARITH | (0x12 << 25),
    OPC_RISC_FSGNJN_H = OPC_RISC_FP_ARITH | (0x12 << 25),
    OPC_RISC_FSGNJX_H = OPC_RISC_FP_ARITH | (0x12 << 25),

    OPC_RISC_FMIN_H = OPC_RISC_FP_ARITH | (0x16 << 25),
    OPC_RISC_FMAX_H = OPC_RISC_FP_ARITH | (0x16 << 25),

    OPC_RISC_FCVT_H_S = OPC_RISC_FP_ARITH | (0x22 << 25),

    OPC_RISC_FSQRT_H = OPC_RISC_FP_ARITH | (0x2E << 25),

    OPC_RISC_FEQ_H = OPC_RISC_FP_ARITH | (0x52 << 25),
    OPC_RISC_FLT_H = OPC_RISC_FP_ARITH | (0x52 << 25),
    OPC_RISC_FLE_H = OPC_RISC_FP_ARITH | (0x52 << 25),

    OPC_RISC_FCVT_W_H = OPC_RISC_FP_ARITH | (0x62 << 25),
    OPC_RISC_FCVT_WU_H = OPC_RISC_FP_ARITH | (0x62 << 25),
    OPC_RISC_FCVT_L_H = OPC_RISC_FP_ARITH | (0x62 << 25),
    OPC_RISC_FCVT_LU_H = OPC_RISC_FP_ARITH | (0x62 << 25),

    OPC_RISC_FCVT_H_W = OPC_RISC_FP_ARITH | (0x6A << 25),
    OPC_RISC_FCVT_H_WU = OPC_RISC_FP_ARITH | (0x6A << 25),
    OPC_RISC_FCVT_H_L = OPC_RISC_FP_ARITH | (0x6A << 25),
    OPC_RISC_FCVT_H_LU = OPC_RISC_FP_ARITH | (0x6A << 25),

    OPC_RISC_FMV_X_H = OPC_RISC_FP_ARITH | (0x72 << 25),
    OPC_RISC_FCLASS_H = OPC_RISC_FP_ARITH | (0x72 << 25),

    OPC_RISC_FMV_H_X = OPC_RISC_FP_ARITH | (0x7A << 25),
};

#define MASK_OP_V(op) (MASK_OP_MAJOR(op) | (op & (0x7 << 12)))
enum {
    OPC_RISC_V_IVV = OPC_RISC_V | (0x0 << 12),
    OPC_RISC_V_FVV = OPC_RISC_V | (0x1 << 12),
    OPC_RISC_V_MVV = OPC_RISC_V | (0x2 << 12),
    OPC_RISC_V_IVI = OPC_RISC_V | (0x3 << 12),
    OPC_RISC_V_IVX = OPC_RISC_V | (0x4 << 12),
    OPC_RISC_V_FVF = OPC_RISC_V | (0x5 << 12),
    OPC_RISC_V_MVX = OPC_RISC_V | (0x6 << 12),
    OPC_RISC_V_CFG = OPC_RISC_V | (0x7 << 12),
};

enum {
    RISC_V_FUNCT_ADD = 0b000000,          //  OPIVV, OPIVX, OPIVI
    RISC_V_FUNCT_REDSUM = 0b000000,       //                      OPMVV
    RISC_V_FUNCT_FADD = 0b000000,         //                                    OPFVV, OPFVF
    RISC_V_FUNCT_REDAND = 0b000001,       //                      OPMVV
    RISC_V_FUNCT_FREDSUM = 0b000001,      //                                    OPFVV
    RISC_V_FUNCT_SUB = 0b000010,          //  OPIVV, OPIVX
    RISC_V_FUNCT_REDOR = 0b000010,        //                      OPMVV
    RISC_V_FUNCT_FSUB = 0b000010,         //                                    OPFVV, OPFVF
    RISC_V_FUNCT_RSUB = 0b000011,         //        OPIVX, OPIVI
    RISC_V_FUNCT_REDXOR = 0b000011,       //                      OPMVV
    RISC_V_FUNCT_FREDOSUM = 0b000011,     //                                    OPFVV
    RISC_V_FUNCT_MINU = 0b000100,         //  OPIVV, OPIVX
    RISC_V_FUNCT_REDMINU = 0b000100,      //                      OPMVV
    RISC_V_FUNCT_FMIN = 0b000100,         //                                    OPFVV, OPFVF
    RISC_V_FUNCT_MIN = 0b000101,          //  OPIVV, OPIVX
    RISC_V_FUNCT_REDMIN = 0b000101,       //                      OPMVV
    RISC_V_FUNCT_FREDMIN = 0b000101,      //                                    OPFVV
    RISC_V_FUNCT_MAXU = 0b000110,         //  OPIVV, OPIVX
    RISC_V_FUNCT_REDMAXU = 0b000110,      //                      OPMVV
    RISC_V_FUNCT_FMAX = 0b000110,         //                                    OPFVV, OPFVF
    RISC_V_FUNCT_MAX = 0b000111,          //  OPIVV, OPIVX
    RISC_V_FUNCT_REDMAX = 0b000111,       //                      OPMVV
    RISC_V_FUNCT_FREDMAX = 0b000111,      //                                    OPFVV
    RISC_V_FUNCT_AADDU = 0b001000,        //                      OPMVV, OPMVX
    RISC_V_FUNCT_FSGNJ = 0b001000,        //                                    OPFVV, OPFVF
    RISC_V_FUNCT_AND = 0b001001,          //  OPIVV, OPIVX, OPIVI
    RISC_V_FUNCT_AADD = 0b001001,         //                      OPMVV, OPMVX
    RISC_V_FUNCT_FSGNJN = 0b001001,       //                                    OPFVV, OPFVF
    RISC_V_FUNCT_OR = 0b001010,           //  OPIVV, OPIVX, OPIVI
    RISC_V_FUNCT_ASUBU = 0b001010,        //                      OPMVV, OPMVX
    RISC_V_FUNCT_FSGNJX = 0b001010,       //                                    OPFVV, OPFVF
    RISC_V_FUNCT_XOR = 0b001011,          //  OPIVV, OPIVX, OPIVI
    RISC_V_FUNCT_ASUB = 0b001011,         //                      OPMVV, OPMVX
    RISC_V_FUNCT_RGATHER = 0b001100,      //  OPIVV, OPIVX, OPIVI
    RISC_V_FUNCT_SLIDEUP = 0b001110,      //        OPIVX, OPIVI
    RISC_V_FUNCT_RGATHEREI16 = 0b001110,  //  OPIVV
    RISC_V_FUNCT_SLIDE1UP = 0b001110,     //                             OPMVX
    RISC_V_FUNCT_FSLIDE1UP = 0b001110,    //                                           OPFVF
    RISC_V_FUNCT_SLIDEDOWN = 0b001111,    //        OPIVX, OPIVI
    RISC_V_FUNCT_SLIDE1DOWN = 0b001111,   //                             OPMVX
    RISC_V_FUNCT_FSLIDE1DOWN = 0b001111,  //                                           OPFVF
    RISC_V_FUNCT_ADC = 0b010000,          //  OPIVV, OPIVX, OPIVI
    RISC_V_FUNCT_WXUNARY0 = 0b010000,     //                      OPMVV
    RISC_V_FUNCT_RXUNARY0 = 0b010000,     //                             OPMVX
    RISC_V_FUNCT_WFUNARY0 = 0b010000,     //                                    OPFVV
    RISC_V_FUNCT_RFUNARY0 = 0b010000,     //                                           OPFVF
    RISC_V_FUNCT_MADC = 0b010001,         //  OPIVV, OPIVX, OPIVI
    RISC_V_FUNCT_SBC = 0b010010,          //  OPIVV, OPIVX
    RISC_V_FUNCT_XUNARY0 = 0b010010,      //                      OPMVV
    RISC_V_FUNCT_FUNARY0 = 0b010010,      //                                    OPFVV
    RISC_V_FUNCT_MSBC = 0b010011,         //  OPIVV, OPIVX
    RISC_V_FUNCT_FUNARY1 = 0b010011,      //                                    OPFVV
    RISC_V_FUNCT_MUNARY0 = 0b010100,      //                      OPMVV
    RISC_V_FUNCT_MERGE_MV = 0b010111,     //  OPIVV, OPIVX, OPIVI
    RISC_V_FUNCT_COMPRESS = 0b010111,     //                      OPMVV
    RISC_V_FUNCT_FMERGE_FMV = 0b010111,   //                                           OPFVF
    RISC_V_FUNCT_MSEQ = 0b011000,         //  OPIVV, OPIVX, OPIVI
    RISC_V_FUNCT_MANDNOT = 0b011000,      //                      OPMVV
    RISC_V_FUNCT_MFEQ = 0b011000,         //                                    OPFVV, OPFVF
    RISC_V_FUNCT_MSNE = 0b011001,         //  OPIVV, OPIVX, OPIVI
    RISC_V_FUNCT_MAND = 0b011001,         //                      OPMVV
    RISC_V_FUNCT_MFLE = 0b011001,         //                                    OPFVV, OPFVF
    RISC_V_FUNCT_MSLTU = 0b011010,        //  OPIVV, OPIVX
    RISC_V_FUNCT_MOR = 0b011010,          //                      OPMVV
    RISC_V_FUNCT_MSLT = 0b011011,         //  OPIVV, OPIVX
    RISC_V_FUNCT_MXOR = 0b011011,         //                      OPMVV
    RISC_V_FUNCT_MFLT = 0b011011,         //                                    OPFVV, OPFVF
    RISC_V_FUNCT_MSLEU = 0b011100,        //  OPIVV, OPIVX, OPIVI
    RISC_V_FUNCT_MORNOT = 0b011100,       //                      OPMVV
    RISC_V_FUNCT_MFNE = 0b011100,         //                                    OPFVV, OPFVF
    RISC_V_FUNCT_MSLE = 0b011101,         //  OPIVV, OPIVX, OPIVI
    RISC_V_FUNCT_MNAND = 0b011101,        //                      OPMVV
    RISC_V_FUNCT_MFGT = 0b011101,         //                                           OPFVF
    RISC_V_FUNCT_MSGTU = 0b011110,        //        OPIVX, OPIVI
    RISC_V_FUNCT_MNOR = 0b011110,         //                      OPMVV
    RISC_V_FUNCT_MSGT = 0b011111,         //        OPIVX, OPIVI
    RISC_V_FUNCT_MXNOR = 0b011111,        //                      OPMVV
    RISC_V_FUNCT_MFGE = 0b011111,         //                                           OPFVF
    RISC_V_FUNCT_SADDU = 0b100000,        //  OPIVV, OPIVX, OPIVI
    RISC_V_FUNCT_DIVU = 0b100000,         //                      OPMVV, OPMVX
    RISC_V_FUNCT_FDIV = 0b100000,         //                                    OPFVV, OPFVF
    RISC_V_FUNCT_SADD = 0b100001,         //  OPIVV, OPIVX, OPIVI
    RISC_V_FUNCT_DIV = 0b100001,          //                      OPMVV, OPMVX
    RISC_V_FUNCT_FRDIV = 0b100001,        //                                           OPFVF
    RISC_V_FUNCT_SSUBU = 0b100010,        //  OPIVV, OPIVX
    RISC_V_FUNCT_REMU = 0b100010,         //                      OPMVV, OPMVX
    RISC_V_FUNCT_SSUB = 0b100011,         //  OPIVV, OPIVX
    RISC_V_FUNCT_REM = 0b100011,          //                      OPMVV, OPMVX
    RISC_V_FUNCT_MULHU = 0b100100,        //                      OPMVV, OPMVX
    RISC_V_FUNCT_FMUL = 0b100100,         //                                    OPFVV, OPFVF
    RISC_V_FUNCT_SLL = 0b100101,          //  OPIVV, OPIVX, OPIVI
    RISC_V_FUNCT_MUL = 0b100101,          //                      OPMVV, OPMVX
    RISC_V_FUNCT_MULHSU = 0b100110,       //                      OPMVV, OPMVX
    RISC_V_FUNCT_SMUL = 0b100111,         //  OPIVV, OPIVX
    RISC_V_FUNCT_MV_NF_R = 0b100111,      //               OPIVI
    RISC_V_FUNCT_MULH = 0b100111,         //                      OPMVV, OPMVX
    RISC_V_FUNCT_FRSUB = 0b100111,        //                                           OPFVF
    RISC_V_FUNCT_SRL = 0b101000,          //  OPIVV, OPIVX, OPIVI
    RISC_V_FUNCT_FMADD = 0b101000,        //                                    OPFVV, OPFVF
    RISC_V_FUNCT_SRA = 0b101001,          //  OPIVV, OPIVX, OPIVI
    RISC_V_FUNCT_MADD = 0b101001,         //                      OPMVV, OPMVX
    RISC_V_FUNCT_FNMADD = 0b101001,       //                                    OPFVV, OPFVF
    RISC_V_FUNCT_SSRL = 0b101010,         //  OPIVV, OPIVX, OPIVI
    RISC_V_FUNCT_FMSUB = 0b101010,        //                                    OPFVV, OPFVF
    RISC_V_FUNCT_SSRA = 0b101011,         //  OPIVV, OPIVX, OPIVI
    RISC_V_FUNCT_NMSUB = 0b101011,        //                      OPMVV, OPMVX
    RISC_V_FUNCT_FNMSUB = 0b101011,       //                                    OPFVV, OPFVF
    RISC_V_FUNCT_NSRL = 0b101100,         //  OPIVV, OPIVX, OPIVI
    RISC_V_FUNCT_FMACC = 0b101100,        //                                    OPFVV, OPFVF
    RISC_V_FUNCT_NSRA = 0b101101,         //  OPIVV, OPIVX, OPIVI
    RISC_V_FUNCT_MACC = 0b101101,         //                      OPMVV, OPMVX
    RISC_V_FUNCT_FNMACC = 0b101101,       //                                    OPFVV, OPFVF
    RISC_V_FUNCT_NCLIPU = 0b101110,       //  OPIVV, OPIVX, OPIVI
    RISC_V_FUNCT_FMSAC = 0b101110,        //                                    OPFVV, OPFVF
    RISC_V_FUNCT_NCLIP = 0b101111,        //  OPIVV, OPIVX, OPIVI
    RISC_V_FUNCT_NMSAC = 0b101111,        //                      OPMVV, OPMVX
    RISC_V_FUNCT_FNMSAC = 0b101111,       //                                    OPFVV, OPFVF
    RISC_V_FUNCT_WREDSUMU = 0b110000,     //  OPIVV
    RISC_V_FUNCT_WADDU = 0b110000,        //                      OPMVV, OPMVX
    RISC_V_FUNCT_FWADD = 0b110000,        //                                    OPFVV, OPFVF
    RISC_V_FUNCT_WREDSUM = 0b110001,      //  OPIVV
    RISC_V_FUNCT_WADD = 0b110001,         //                      OPMVV, OPMVX
    RISC_V_FUNCT_FWREDSUM = 0b110001,     //                                    OPFVV
    RISC_V_FUNCT_WSUBU = 0b110010,        //                      OPMVV, OPMVX
    RISC_V_FUNCT_FWSUB = 0b110010,        //                                    OPFVV, OPFVF
    RISC_V_FUNCT_WSUB = 0b110011,         //                      OPMVV, OPMVX
    RISC_V_FUNCT_FWREDOSUM = 0b110011,    //                                    OPFVV
    RISC_V_FUNCT_WADDUW = 0b110100,       //                      OPMVV, OPMVX
    RISC_V_FUNCT_FWADDW = 0b110100,       //                                    OPFVV, OPFVF
    RISC_V_FUNCT_WADDW = 0b110101,        //                      OPMVV, OPMVX
    RISC_V_FUNCT_WSUBUW = 0b110110,       //                      OPMVV, OPMVX
    RISC_V_FUNCT_FWSUBW = 0b110110,       //                                    OPFVV, OPFVF
    RISC_V_FUNCT_WSUBW = 0b110111,        //                      OPMVV, OPMVX
    RISC_V_FUNCT_WMULU = 0b111000,        //                      OPMVV, OPMVX
    RISC_V_FUNCT_FWMUL = 0b111000,        //                                    OPFVV, OPFVF
    RISC_V_FUNCT_WMULSU = 0b111010,       //                      OPMVV, OPMVX
    RISC_V_FUNCT_WMUL = 0b111011,         //                      OPMVV, OPMVX
    RISC_V_FUNCT_WMACCU = 0b111100,       //                      OPMVV, OPMVX
    RISC_V_FUNCT_FWMACC = 0b111100,       //                                    OPFVV, OPFVF
    RISC_V_FUNCT_WMACC = 0b111101,        //                      OPMVV, OPMVX
    RISC_V_FUNCT_FWNMACC = 0b111101,      //                                    OPFVV, OPFVF
    RISC_V_FUNCT_WMACCUS = 0b111110,      //                             OPMVX
    RISC_V_FUNCT_FWMSAC = 0b111110,       //                                    OPFVV, OPFVF
    RISC_V_FUNCT_WMACCSU = 0b111111,      //                      OPMVV, OPMVX
    RISC_V_FUNCT_FWNMSAC = 0b111111,      //                                    OPFVV, OPFVF
};

#define MASK_OP_V_CFG(op) (op & OPC_RISC_V_CFG) | (op & (0x3 << 30))
enum {
    OPC_RISC_VSETVLI_0 = OPC_RISC_V_CFG | (0x0 << 30),
    OPC_RISC_VSETVLI_1 = OPC_RISC_V_CFG | (0x1 << 30),
    OPC_RISC_VSETVL = OPC_RISC_V_CFG | (0x2 << 30),
    OPC_RISC_VSETIVLI = OPC_RISC_V_CFG | (0x3 << 30),
};

#define GET_B_IMM(inst)                                                                             \
    ((extract32(inst, 8, 4) << 1) | (extract32(inst, 25, 6) << 5) | (extract32(inst, 7, 1) << 11) | \
     (sextract64(inst, 31, 1) << 12))

#define GET_STORE_IMM(inst) ((extract32(inst, 7, 5)) | (sextract64(inst, 25, 7) << 5))

#define GET_JAL_IMM(inst)                                                                               \
    ((extract32(inst, 21, 10) << 1) | (extract32(inst, 20, 1) << 11) | (extract32(inst, 12, 8) << 12) | \
     (sextract64(inst, 31, 1) << 20))

#define GET_RM(inst)      extract32(inst, 12, 3)
#define GET_RS3(inst)     extract32(inst, 27, 5)
#define GET_RS1(inst)     extract32(inst, 15, 5)
#define GET_RS2(inst)     extract32(inst, 20, 5)
#define GET_RD(inst)      extract32(inst, 7, 5)
#define GET_FUNCT12(inst) extract32(inst, 20, 12)
#define GET_IMM(inst)     sextract64(inst, 20, 12)

/* RVC decoding macros */
#define GET_C_IMM(inst)  (extract32(inst, 2, 5) | (sextract64(inst, 12, 1) << 5))
#define GET_C_ZIMM(inst) (extract32(inst, 2, 5) | (extract32(inst, 12, 1) << 5))
#define GET_C_ADDI4SPN_IMM(inst) \
    ((extract32(inst, 6, 1) << 2) | (extract32(inst, 5, 1) << 3) | (extract32(inst, 11, 2) << 4) | (extract32(inst, 7, 4) << 6))
#define GET_C_ADDI16SP_IMM(inst)                                                                                                 \
    ((extract32(inst, 6, 1) << 4) | (extract32(inst, 2, 1) << 5) | (extract32(inst, 5, 1) << 6) | (extract32(inst, 3, 2) << 7) | \
     (sextract64(inst, 12, 1) << 9))
#define GET_C_LWSP_IMM(inst) ((extract32(inst, 4, 3) << 2) | (extract32(inst, 12, 1) << 5) | (extract32(inst, 2, 2) << 6))
#define GET_C_LDSP_IMM(inst) ((extract32(inst, 5, 2) << 3) | (extract32(inst, 12, 1) << 5) | (extract32(inst, 2, 3) << 6))
#define GET_C_SWSP_IMM(inst) ((extract32(inst, 9, 4) << 2) | (extract32(inst, 7, 2) << 6))
#define GET_C_SDSP_IMM(inst) ((extract32(inst, 10, 3) << 3) | (extract32(inst, 7, 3) << 6))
#define GET_C_LW_IMM(inst)   ((extract32(inst, 6, 1) << 2) | (extract32(inst, 10, 3) << 3) | (extract32(inst, 5, 1) << 6))
#define GET_C_LD_IMM(inst)   ((extract32(inst, 10, 3) << 3) | (extract32(inst, 5, 2) << 6))
#define GET_C_J_IMM(inst)                                                                          \
    ((extract32(inst, 3, 3) << 1) | (extract32(inst, 11, 1) << 4) | (extract32(inst, 2, 1) << 5) | \
     (extract32(inst, 7, 1) << 6) | (extract32(inst, 6, 1) << 7) | (extract32(inst, 9, 2) << 8) |  \
     (extract32(inst, 8, 1) << 10) | (sextract64(inst, 12, 1) << 11))
#define GET_C_B_IMM(inst)                                                                          \
    ((extract32(inst, 3, 2) << 1) | (extract32(inst, 10, 2) << 3) | (extract32(inst, 2, 1) << 5) | \
     (extract32(inst, 5, 2) << 6) | (sextract64(inst, 12, 1) << 8))
#define GET_C_SIMM3(inst) extract32(inst, 10, 3)
#define GET_C_RD(inst)    GET_RD(inst)
#define GET_C_RS1(inst)   GET_RD(inst)
#define GET_C_RS2(inst)   extract32(inst, 2, 5)
#define GET_C_RS1S(inst)  (8 + extract32(inst, 7, 3))
#define GET_C_RS2S(inst)  (8 + extract32(inst, 2, 3))

/* Zcb decoding macros */
#define GET_C_LBU_IMM(inst) ((extract32(inst, 5, 1) << 1) | extract32(inst, 6, 1))
#define GET_C_LH_IMM(inst)  (extract32(inst, 5, 1) << 1)
#define GET_C_LHU_IMM(inst) (extract32(inst, 5, 1) << 1)
#define GET_C_SB_IMM(inst)  ((extract32(inst, 5, 1) << 1) | extract32(inst, 6, 1))
#define GET_C_SH_IMM(inst)  (extract32(inst, 5, 1) << 1)

/* Zcmp decoding macros */
#define GET_C_PUSHPOP_SPIMM(inst) extract32(inst, 2, 2)
#define GET_C_PUSHPOP_RLIST(inst) extract32(inst, 4, 4)
#define GET_C_MVSA01_R1S(inst)    extract32(inst, 7, 3)
#define GET_C_MVSA01_R2S(inst)    extract32(inst, 2, 3)

/* Zcmt decoding macros */
#define GET_C_JT_INDEX(inst) extract32(inst, 2, 8)

#ifdef ARITH_GE
#define GE_ARG , void *gep
#else
#define GE_ARG
#endif

#define clamp(x, low, high) (((x) > (high)) ? (high) : (((x) < (low)) ? (low) : (x)))

#ifdef PFX_Q
#define PFX q
static inline uint16_t glue(unit_add16_, PFX)(uint16_t a, uint16_t b, uint16_t *ge)
{
    int32_t sa, sb, sr;
    uint16_t result;
    *ge = 0;
    sa = (int32_t)((int16_t)a);
    sb = (int32_t)((int16_t)b);
    sr = sa + sb;
    result = (uint16_t)clamp(sr, -32768, 32767);
    return result;
}

static inline uint8_t glue(unit_add8_, PFX)(uint8_t a, uint8_t b, uint16_t *ge)
{
    int32_t sa, sb, sr;
    uint8_t result;
    *ge = 0;
    sa = (int32_t)((int8_t)a);
    sb = (int32_t)((int8_t)b);
    sr = sa + sb;
    result = (uint8_t)clamp(sr, -128, 127);
    return result;
}

static inline uint16_t glue(unit_sub16_, PFX)(uint16_t a, uint16_t b, uint16_t *ge)
{
    int32_t sa, sb, sr;
    uint16_t result;
    *ge = 0;
    sa = (int32_t)((int16_t)a);
    sb = (int32_t)((int16_t)b);
    sr = sa - sb;
    result = (uint16_t)clamp(sr, -32768, 32767);
    return result;
}

static inline uint8_t glue(unit_sub8_, PFX)(uint8_t a, uint8_t b, uint16_t *ge)
{
    int32_t sa, sb, sr;
    uint8_t result;
    *ge = 0;
    sa = (int32_t)((int8_t)a);
    sb = (int32_t)((int8_t)b);
    sr = sa - sb;
    result = (uint8_t)clamp(sr, -128, 127);
    return result;
}
#endif
#ifdef PFX_UQ
#define PFX uq
static inline uint16_t glue(unit_add16_, PFX)(uint16_t a, uint16_t b, uint16_t *ge)
{
    uint32_t result = (uint32_t)a + (uint32_t)b;
    if(result > 65535) {
        result = 65535;
    }
    *ge = 0;
    return (uint16_t)result;
}

static inline uint8_t glue(unit_add8_, PFX)(uint8_t a, uint8_t b, uint16_t *ge)
{
    uint32_t result = (uint32_t)a + (uint32_t)b;
    if(result > 255) {
        result = 255;
    }
    *ge = 0;
    return (uint8_t)result;
}

static inline uint16_t glue(unit_sub16_, PFX)(uint16_t a, uint16_t b, uint16_t *ge)
{
    uint32_t result;
    if(a < b) {
        result = 0;
    } else {
        result = (uint32_t)a - (uint32_t)b;
    }
    *ge = 0;
    return (uint16_t)result;
}

static inline uint8_t glue(unit_sub8_, PFX)(uint8_t a, uint8_t b, uint16_t *ge)
{
    uint32_t result;
    if(a < b) {
        result = 0;
    } else {
        result = (uint32_t)a - (uint32_t)b;
    }
    *ge = 0;
    return (uint8_t)result;
}
#endif
#ifdef PFX_S
#define PFX s
static inline uint16_t glue(unit_add16_, PFX)(uint16_t a, uint16_t b, uint16_t *ge)
{
    int32_t result;
    int32_t sa, sb;
    sa = (int32_t)((int16_t)a);
    sb = (int32_t)((int16_t)b);
    result = sa + sb;
    if(result >= 0) {
        *ge = 1;
    } else {
        *ge = 0;
    }
    return (uint16_t)((int16_t)result);
}

static inline uint8_t glue(unit_add8_, PFX)(uint8_t a, uint8_t b, uint16_t *ge)
{
    int32_t result;
    int32_t sa, sb;
    sa = (int32_t)((int8_t)a);
    sb = (int32_t)((int8_t)b);
    result = sa + sb;
    if(result >= 0) {
        *ge = 1;
    } else {
        *ge = 0;
    }
    return (uint8_t)((int8_t)result);
}

static inline uint16_t glue(unit_sub16_, PFX)(uint16_t a, uint16_t b, uint16_t *ge)
{
    int32_t result;
    int32_t sa, sb;
    sa = (int32_t)((int16_t)a);
    sb = (int32_t)((int16_t)b);
    result = sa - sb;
    if(result >= 0) {
        *ge = 1;
    } else {
        *ge = 0;
    }
    return (uint16_t)(int16_t)result;
}

static inline uint8_t glue(unit_sub8_, PFX)(uint8_t a, uint8_t b, uint16_t *ge)
{
    int32_t result;
    int32_t sa, sb;
    sa = (int32_t)((int8_t)a);
    sb = (int32_t)((int8_t)b);
    result = sa - sb;
    if(result >= 0) {
        *ge = 1;
    } else {
        *ge = 0;
    }
    return (uint8_t)((int8_t)result);
}
#endif
#ifdef PFX_U
#define PFX u
static inline uint16_t glue(unit_add16_, PFX)(uint16_t a, uint16_t b, uint16_t *ge)
{
    uint32_t result;
    result = (uint32_t)a + (uint32_t)b;
    if(result > 65535) {
        *ge = 1;
    } else {
        *ge = 0;
    }
    return (uint16_t)result;
}

static inline uint8_t glue(unit_add8_, PFX)(uint8_t a, uint8_t b, uint16_t *ge)
{
    uint32_t result;
    result = (uint32_t)a + (uint32_t)b;
    if(result > 255) {
        *ge = 1;
    } else {
        *ge = 0;
    }
    return (uint8_t)result;
}

static inline uint16_t glue(unit_sub16_, PFX)(uint16_t a, uint16_t b, uint16_t *ge)
{
    int32_t result;
    int32_t sa, sb;
    sa = (int32_t)((int16_t)a);
    sb = (int32_t)((int16_t)b);
    result = sa - sb;
    if(result >= 0) {
        *ge = 1;
    } else {
        *ge = 0;
    }
    return (uint16_t)((int16_t)result);
}

static inline uint8_t glue(unit_sub8_, PFX)(uint8_t a, uint8_t b, uint16_t *ge)
{
    int32_t result;
    int32_t sa, sb;
    sa = (int32_t)((int16_t)a);
    sb = (int32_t)((int16_t)b);
    result = sa - sb;
    if(result >= 0) {
        *ge = 1;
    } else {
        *ge = 0;
    }
    return (uint8_t)((int8_t)result);
}
#endif
#ifdef PFX_SH
#define PFX sh
static inline uint16_t glue(unit_add16_, PFX)(uint16_t a, uint16_t b, uint16_t *ge)
{
    uint32_t result;
    int32_t sa, sb;
    sa = (int32_t)((int16_t)a);
    sb = (int32_t)((int16_t)b);
    result = (sa + sb) >> 1;
    return (uint16_t)result;
}

static inline uint8_t glue(unit_add8_, PFX)(uint8_t a, uint8_t b, uint16_t *ge)
{
    uint32_t result;
    int32_t sa, sb;
    sa = (int32_t)((int8_t)a);
    sb = (int32_t)((int8_t)b);
    result = (sa + sb) >> 1;
    return (uint8_t)result;
}

static inline uint16_t glue(unit_sub16_, PFX)(uint16_t a, uint16_t b, uint16_t *ge)
{
    uint32_t result;
    int32_t sa, sb;
    sa = (int32_t)((int16_t)a);
    sb = (int32_t)((int16_t)b);
    result = (sa - sb) >> 1;
    return (uint16_t)result;
}

static inline uint8_t glue(unit_sub8_, PFX)(uint8_t a, uint8_t b, uint16_t *ge)
{
    uint32_t result;
    int32_t sa, sb;
    sa = (int32_t)((int8_t)a);
    sb = (int32_t)((int8_t)b);
    result = (sa - sb) >> 1;
    return (uint8_t)result;
}
#endif
#ifdef PFX_UH
#define PFX uh
static inline uint16_t glue(unit_add16_, PFX)(uint16_t a, uint16_t b, uint16_t *ge)
{
    return (uint16_t)(((uint32_t)a + (uint32_t)b) >> 1);
}

static inline uint8_t glue(unit_add8_, PFX)(uint16_t a, uint16_t b, uint16_t *ge)
{
    return (uint8_t)(((uint32_t)a + (uint32_t)b) >> 1);
}

static inline uint16_t glue(unit_sub16_, PFX)(uint16_t a, uint16_t b, uint16_t *ge)
{
    return (uint16_t)(((uint32_t)a - (uint32_t)b) >> 1);
}

static inline uint8_t glue(unit_sub8_, PFX)(uint16_t a, uint16_t b, uint16_t *ge)
{
    return (uint8_t)(((uint32_t)a - (uint32_t)b) >> 1);
}
#endif

uint32_t HELPER(glue(PFX, add16))(uint32_t a, uint32_t b GE_ARG)
{
    uint32_t result = 0;
    uint16_t res1, res2, ge;
#ifdef ARITH_GE
    uint32_t ge_flags = 0;
#endif

    res1 = glue(unit_add16_, PFX)(a, b, &ge);
#ifdef ARITH_GE
    ge_flags |= (3 * ge);
#endif

    res2 = glue(unit_add16_, PFX)(a >> 16, b >> 16, &ge);
#ifdef ARITH_GE
    ge_flags |= (3 * ge) << 2;
#endif

    result = (((uint32_t)res2) << 16) | res1;
#ifdef ARITH_GE
    *(uint32_t *)gep = ge_flags;
#endif

    return result;
}

uint32_t HELPER(glue(PFX, add8))(uint32_t a, uint32_t b GE_ARG)
{
    uint32_t result = 0;
    uint8_t res1, res2, res3, res4;
    uint16_t ge;
#ifdef ARITH_GE
    uint32_t ge_flags = 0;
#endif

    res1 = glue(unit_add8_, PFX)(a, b, &ge);
#ifdef ARITH_GE
    ge_flags |= ge;
#endif

    res2 = glue(unit_add8_, PFX)(a >> 8, b >> 8, &ge);
#ifdef ARITH_GE
    ge_flags |= ge << 1;
#endif

    res3 = glue(unit_add8_, PFX)(a >> 16, b >> 16, &ge);
#ifdef ARITH_GE
    ge_flags |= ge << 2;
#endif

    res4 = glue(unit_add8_, PFX)(a >> 24, b >> 24, &ge);
#ifdef ARITH_GE
    ge_flags |= ge << 3;
#endif

    result = (((uint32_t)res4) << 24) | (((uint32_t)res3) << 16) | (((uint32_t)res2) << 8) | res1;
#ifdef ARITH_GE
    *(uint32_t *)gep = ge_flags;
#endif

    return result;
}

uint32_t HELPER(glue(PFX, sub16))(uint32_t a, uint32_t b GE_ARG)
{
    uint32_t result = 0;
    uint16_t res1, res2, ge;
#ifdef ARITH_GE
    uint32_t ge_flags = 0;
#endif

    res1 = glue(unit_sub16_, PFX)(a, b, &ge);
#ifdef ARITH_GE
    ge_flags |= (3 * ge);
#endif

    res2 = glue(unit_sub16_, PFX)(a >> 16, b >> 16, &ge);
#ifdef ARITH_GE
    ge_flags |= (3 * ge) << 2;
#endif

    result = (((uint32_t)res2) << 16) | res1;
#ifdef ARITH_GE
    *(uint32_t *)gep = ge_flags;
#endif

    return result;
}

uint32_t HELPER(glue(PFX, sub8))(uint32_t a, uint32_t b GE_ARG)
{
    uint32_t result = 0;
    uint8_t res1, res2, res3, res4;
    uint16_t ge;
#ifdef ARITH_GE
    uint32_t ge_flags = 0;
#endif

    res1 = glue(unit_sub8_, PFX)(a, b, &ge);
#ifdef ARITH_GE
    ge_flags |= ge;
#endif

    res2 = glue(unit_sub8_, PFX)(a >> 8, b >> 8, &ge);
#ifdef ARITH_GE
    ge_flags |= ge << 1;
#endif

    res3 = glue(unit_sub8_, PFX)(a >> 16, b >> 16, &ge);
#ifdef ARITH_GE
    ge_flags |= ge << 2;
#endif

    res4 = glue(unit_sub8_, PFX)(a >> 24, b >> 24, &ge);
#ifdef ARITH_GE
    ge_flags |= ge << 3;
#endif

    result = (((uint32_t)res4) << 24) | (((uint32_t)res3) << 16) | (((uint32_t)res2) << 8) | res1;
#ifdef ARITH_GE
    *(uint32_t *)gep = ge_flags;
#endif

    return result;
}

uint32_t HELPER(glue(PFX, subaddx))(uint32_t a, uint32_t b GE_ARG)
{
    uint32_t result = 0;
    uint16_t res_top, res_bottom, ge;
#ifdef ARITH_GE
    uint32_t ge_flags = 0;
#endif

    res_top = glue(unit_sub16_, PFX)(a >> 16, b, &ge);
#ifdef ARITH_GE
    ge_flags |= (3 * ge);
#endif

    res_bottom = glue(unit_add16_, PFX)(a, b >> 16, &ge);
#ifdef ARITH_GE
    ge_flags |= (3 * ge) << 2;
#endif

    result = (((uint32_t)res_top) << 16) | res_bottom;
#ifdef ARITH_GE
    *(uint32_t *)gep = ge_flags;
#endif

    return result;
}

uint32_t HELPER(glue(PFX, addsubx))(uint32_t a, uint32_t b GE_ARG)
{
    uint32_t result = 0;
    uint16_t res_top, res_bottom, ge;
#ifdef ARITH_GE
    uint32_t ge_flags = 0;
#endif

    res_top = glue(unit_add16_, PFX)(a >> 16, b, &ge);
#ifdef ARITH_GE
    ge_flags |= (3 * ge);
#endif

    res_bottom = glue(unit_sub16_, PFX)(a, b >> 16, &ge);
#ifdef ARITH_GE
    ge_flags |= (3 * ge) << 2;
#endif

    result = (((uint32_t)res_top) << 16) | res_bottom;
#ifdef ARITH_GE
    *(uint32_t *)gep = ge_flags;
#endif

    return result;
}

#undef GE_ARG
#undef PFX
#undef _INLINE

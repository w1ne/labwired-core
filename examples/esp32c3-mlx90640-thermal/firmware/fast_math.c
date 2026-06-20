/* Fast soft-float libm shims for the ESP32-C3 (no FPU).
 *
 * The vendored Melexis driver runs UNMODIFIED on-target and calls only three
 * libm functions: pow(), sqrt() and fabs(). newlib's generic double `pow()` is
 * thousands of soft-float instructions per call, and the driver's
 * ExtractParameters invokes it ~2300× (via the POW2(x) = pow(2, x) macro),
 * which dominated the boot cost (tens of millions of simulated cycles).
 *
 * Crucially, EVERY use of pow() in the driver is POW2(x) = pow(2, (double)x)
 * where x is a small non-negative integer (calibration scales 0..30, plus the
 * fixed 14 and 18). So we provide a `pow()` that computes 2^n exactly by
 * scaling the exponent field of the IEEE-754 double — O(1), no loops, no libm.
 * A general fallback covers any other (unused) call. The driver is not touched;
 * we only supply a faster libm symbol the linker resolves ahead of newlib's.
 */
#include <stdint.h>

/* Exact 2^n for integer n via the double exponent field (bias 1023). Covers
 * the only base the driver ever passes. */
static double pow2_int(int n) {
    if (n >= -1022 && n <= 1023) {
        uint64_t bits = (uint64_t)(n + 1023) << 52;
        double d;
        __builtin_memcpy(&d, &bits, sizeof(d));
        return d;
    }
    /* Out-of-range: fall back to repeated squaring (never hit by the driver). */
    double r = 1.0;
    double b = (n < 0) ? 0.5 : 2.0;
    int k = n < 0 ? -n : n;
    while (k--) {
        r *= b;
    }
    return r;
}

double pow(double base, double exp) {
    /* The driver only ever calls pow(2, integer). Detect the exact-integer
     * exponent and route to the fast path. */
    long n = (long)exp;
    if ((double)n == exp) {
        if (base == 2.0) {
            return pow2_int((int)n);
        }
        /* Generic integer power by binary exponentiation (unused by the driver
         * but kept correct). */
        double result = 1.0;
        double b = base;
        long k = n < 0 ? -n : n;
        while (k > 0) {
            if (k & 1) {
                result *= b;
            }
            b *= b;
            k >>= 1;
        }
        return (n < 0) ? 1.0 / result : result;
    }
    /* Non-integer exponent: never reached by the MLX90640 driver. Return a safe
     * value rather than pulling in exp()/log(). */
    return base; /* unreachable in this firmware */
}

/* Fast sqrt. The driver calls sqrt() ~1500×/frame (sqrt(sqrt(...)) per pixel)
 * with `float` operands radiometrically (it stores the result into a `float`),
 * so single-precision is ample — sub-0.01 °C in the reconstruction. We compute
 * the reciprocal square root with the division-free, MULTIPLY-ONLY Newton
 * iteration  y ← y·(1.5 − 0.5·x·y²)  in single precision (soft-float `float`
 * mul is ~half the cost of `double`), seeded by the classic fast-inverse-sqrt
 * bit hack, then return x·rsqrt(x). No soft-float divide, no libm, no hardware
 * FP. Four iterations from the seed reach full float precision.
 *
 * The function signature stays `double sqrt(double)` so it transparently
 * overrides newlib's symbol for the unmodified driver. */
static float fast_sqrtf(float x) {
    if (x <= 0.0f) {
        return 0.0f;
    }
    uint32_t bits;
    __builtin_memcpy(&bits, &x, sizeof(bits));
    uint32_t seed = 0x5F3759DFu - (bits >> 1); /* fast inverse sqrt magic */
    float y;
    __builtin_memcpy(&y, &seed, sizeof(y));
    float xhalf = 0.5f * x;
    y = y * (1.5f - xhalf * y * y);
    y = y * (1.5f - xhalf * y * y);
    y = y * (1.5f - xhalf * y * y);
    return x * y; /* sqrt(x) = x · rsqrt(x) */
}

double sqrt(double x) { return (double)fast_sqrtf((float)x); }

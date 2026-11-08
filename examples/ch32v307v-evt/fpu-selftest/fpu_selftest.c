/* RV32F self-test for the CH32V307 (QingKe V4F), built on ch32fun.
 *
 * The same source builds two ways:
 *   - for the CH32V307 (rv32imafc/ilp32f), where every operation is the
 *     RV32F instruction itself (inline asm, dynamic rounding through frm),
 *     with fflags read back after each one;
 *   - for the build host (x86-64 SSE + FMA3, `fenv.h`), which prints the
 *     expected output. x86 implements binary32 with tininess detected after
 *     rounding, as RISC-V does, for RNE/RTZ/RDN/RUP and the same five flags.
 *
 * The output is one line per (vector, rounding mode) holding an FNV-1a hash
 * of every arithmetic result and its flags, one line per vector for the
 * mode-independent instructions, and a few direct checks. Build with
 * -DVERBOSE to print every result instead of hashes when chasing a mismatch.
 *
 * Where the two differ by design, the host side follows the RISC-V rule:
 * NaN results print as the canonical NaN 0x7fc00000 (x86 propagates
 * payloads), FCVT.W[U].S saturates and raises only NV out of range, and
 * FMIN/FMAX/FCLASS/FSGNJ are written out from the ISA text, since
 * fminf()'s signalling-NaN and signed-zero choices are not RISC-V's.
 */
#include <stdint.h>

#ifdef __riscv
#include "ch32fun.h"
#include <stdio.h>
#else
#include <fenv.h>
#include <math.h>
#include <stdio.h>
#pragma STDC FENV_ACCESS ON
#endif

typedef union {
	float f;
	uint32_t u;
} fbits;

static float F(uint32_t u)
{
	fbits b;
	b.u = u;
	return b.f;
}
static uint32_t U(float f)
{
	fbits b;
	b.f = f;
	return b.u;
}

#define NV 0x10
#define DZ 0x08
#define OF 0x04
#define UF 0x02
#define NX 0x01

#ifdef __riscv
static void set_mode(uint32_t m) { asm volatile("csrw frm, %0" ::"r"(m)); }
static void clear_flags(void) { asm volatile("csrw fflags, zero"); }
static uint32_t get_flags(void)
{
	uint32_t v;
	asm volatile("csrr %0, fflags" : "=r"(v));
	return v;
}
#define OP2(insn)                                                      \
	static uint32_t op_##insn(uint32_t a, uint32_t b)                  \
	{                                                                  \
		float r;                                                       \
		asm volatile(#insn ".s %0, %1, %2" : "=f"(r) : "f"(F(a)), "f"(F(b))); \
		return U(r);                                                   \
	}
OP2(fadd)
OP2(fsub)
OP2(fmul)
OP2(fdiv)
OP2(fmin)
OP2(fmax)
OP2(fsgnj)
OP2(fsgnjn)
OP2(fsgnjx)
#define OP3(insn)                                                      \
	static uint32_t op_##insn(uint32_t a, uint32_t b, uint32_t c)      \
	{                                                                  \
		float r;                                                       \
		asm volatile(#insn ".s %0, %1, %2, %3"                         \
					 : "=f"(r)                                         \
					 : "f"(F(a)), "f"(F(b)), "f"(F(c)));               \
		return U(r);                                                   \
	}
OP3(fmadd)
OP3(fmsub)
OP3(fnmadd)
OP3(fnmsub)
static uint32_t op_fsqrt(uint32_t a)
{
	float r;
	asm volatile("fsqrt.s %0, %1" : "=f"(r) : "f"(F(a)));
	return U(r);
}
static uint32_t op_fcvt_w(uint32_t a)
{
	uint32_t r;
	asm volatile("fcvt.w.s %0, %1" : "=r"(r) : "f"(F(a)));
	return r;
}
static uint32_t op_fcvt_wu(uint32_t a)
{
	uint32_t r;
	asm volatile("fcvt.wu.s %0, %1" : "=r"(r) : "f"(F(a)));
	return r;
}
static uint32_t op_fcvt_s_w(uint32_t a)
{
	float r;
	asm volatile("fcvt.s.w %0, %1" : "=f"(r) : "r"(a));
	return U(r);
}
static uint32_t op_fcvt_s_wu(uint32_t a)
{
	float r;
	asm volatile("fcvt.s.wu %0, %1" : "=f"(r) : "r"(a));
	return U(r);
}
#define CMP(insn)                                                      \
	static uint32_t op_##insn(uint32_t a, uint32_t b)                  \
	{                                                                  \
		uint32_t r;                                                    \
		asm volatile(#insn ".s %0, %1, %2" : "=r"(r) : "f"(F(a)), "f"(F(b))); \
		return r;                                                      \
	}
CMP(feq)
CMP(flt)
CMP(fle)
static uint32_t op_fclass(uint32_t a)
{
	uint32_t r;
	asm volatile("fclass.s %0, %1" : "=r"(r) : "f"(F(a)));
	return r;
}
static uint32_t op_fmv_round_trip(uint32_t a)
{
	uint32_t r;
	float t;
	asm volatile("fmv.w.x %0, %1" : "=f"(t) : "r"(a));
	asm volatile("fmv.x.w %0, %1" : "=r"(r) : "f"(t));
	return r;
}
#else
static const int host_modes[4] = { FE_TONEAREST, FE_TOWARDZERO, FE_DOWNWARD, FE_UPWARD };
static void set_mode(uint32_t m) { fesetround(host_modes[m]); }
static void clear_flags(void) { feclearexcept(FE_ALL_EXCEPT); }
static uint32_t get_flags(void)
{
	int e = fetestexcept(FE_ALL_EXCEPT);
	return ((e & FE_INVALID) ? NV : 0) | ((e & FE_DIVBYZERO) ? DZ : 0) |
		   ((e & FE_OVERFLOW) ? OF : 0) | ((e & FE_UNDERFLOW) ? UF : 0) |
		   ((e & FE_INEXACT) ? NX : 0);
}
static int is_nan(uint32_t a) { return (a & 0x7f800000) == 0x7f800000 && (a & 0x7fffff); }
static int is_snan(uint32_t a) { return is_nan(a) && !(a & 0x400000); }
static uint32_t canon(float r) { return isnan(r) ? 0x7fc00000 : U(r); }
static uint32_t op_fadd(uint32_t a, uint32_t b)
{
	volatile float x = F(a), y = F(b);
	return canon(x + y);
}
static uint32_t op_fsub(uint32_t a, uint32_t b)
{
	volatile float x = F(a), y = F(b);
	return canon(x - y);
}
static uint32_t op_fmul(uint32_t a, uint32_t b)
{
	volatile float x = F(a), y = F(b);
	return canon(x * y);
}
static uint32_t op_fdiv(uint32_t a, uint32_t b)
{
	volatile float x = F(a), y = F(b);
	return canon(x / y);
}
static uint32_t op_fsqrt(uint32_t a)
{
	volatile float x = F(a);
	return canon(__builtin_sqrtf(x));
}
static uint32_t op_fmadd(uint32_t a, uint32_t b, uint32_t c)
{
	volatile float x = F(a), y = F(b), z = F(c);
	return canon(__builtin_fmaf(x, y, z));
}
static uint32_t op_fmsub(uint32_t a, uint32_t b, uint32_t c)
{
	volatile float x = F(a), y = F(b), z = F(c ^ 0x80000000u);
	return canon(__builtin_fmaf(x, y, z));
}
static uint32_t op_fnmsub(uint32_t a, uint32_t b, uint32_t c)
{
	volatile float x = F(a ^ 0x80000000u), y = F(b), z = F(c);
	return canon(__builtin_fmaf(x, y, z));
}
static uint32_t op_fnmadd(uint32_t a, uint32_t b, uint32_t c)
{
	volatile float x = F(a ^ 0x80000000u), y = F(b), z = F(c ^ 0x80000000u);
	return canon(__builtin_fmaf(x, y, z));
}
static uint32_t to_int(uint32_t a, int is_unsigned)
{
	volatile float x = F(a);
	if (is_nan(a)) {
		clear_flags();
		feraiseexcept(FE_INVALID);
		return is_unsigned ? 0xffffffffu : 0x7fffffffu;
	}
	float r = rintf(x);
	double lo = is_unsigned ? 0.0 : -2147483648.0;
	double hi = is_unsigned ? 4294967295.0 : 2147483647.0;
	if ((double)r < lo || (double)r > hi) {
		clear_flags();
		feraiseexcept(FE_INVALID);
		if (is_unsigned)
			return x < 0 ? 0 : 0xffffffffu;
		return x < 0 ? 0x80000000u : 0x7fffffffu;
	}
	return is_unsigned ? (uint32_t)(double)r : (uint32_t)(int32_t)r;
}
static uint32_t op_fcvt_w(uint32_t a) { return to_int(a, 0); }
static uint32_t op_fcvt_wu(uint32_t a) { return to_int(a, 1); }
static uint32_t op_fcvt_s_w(uint32_t a)
{
	volatile int32_t v = (int32_t)a;
	return U((float)v);
}
static uint32_t op_fcvt_s_wu(uint32_t a)
{
	volatile uint32_t v = a;
	return U((float)v);
}
static uint32_t min_max(uint32_t a, uint32_t b, int max)
{
	if (is_snan(a) || is_snan(b))
		feraiseexcept(FE_INVALID);
	if (is_nan(a) && is_nan(b))
		return 0x7fc00000;
	if (is_nan(a))
		return b;
	if (is_nan(b))
		return a;
	float x = F(a), y = F(b);
	int a_first;
	if (x == y)
		a_first = ((a >> 31) != 0) != max;
	else
		a_first = (x < y) != max;
	return a_first ? a : b;
}
static uint32_t op_fmin(uint32_t a, uint32_t b) { return min_max(a, b, 0); }
static uint32_t op_fmax(uint32_t a, uint32_t b) { return min_max(a, b, 1); }
static uint32_t op_fsgnj(uint32_t a, uint32_t b) { return (a & 0x7fffffff) | (b & 0x80000000); }
static uint32_t op_fsgnjn(uint32_t a, uint32_t b) { return (a & 0x7fffffff) | (~b & 0x80000000); }
static uint32_t op_fsgnjx(uint32_t a, uint32_t b) { return a ^ (b & 0x80000000); }
/* == is UCOMISS (signals only on sNaN); < and <= are COMISS (signal on any
 * NaN): FEQ, FLT and FLE exactly. */
static uint32_t op_feq(uint32_t a, uint32_t b)
{
	volatile float x = F(a), y = F(b);
	return x == y;
}
static uint32_t op_flt(uint32_t a, uint32_t b)
{
	volatile float x = F(a), y = F(b);
	return x < y;
}
static uint32_t op_fle(uint32_t a, uint32_t b)
{
	volatile float x = F(a), y = F(b);
	return x <= y;
}
static uint32_t op_fclass(uint32_t a)
{
	int neg = a >> 31;
	uint32_t e = (a >> 23) & 0xff, m = a & 0x7fffff;
	if (e == 0xff)
		return m ? (is_snan(a) ? 1u << 8 : 1u << 9) : (neg ? 1u : 1u << 7);
	if (e == 0)
		return m ? (neg ? 1u << 2 : 1u << 5) : (neg ? 1u << 3 : 1u << 4);
	return neg ? 1u << 1 : 1u << 6;
}
static uint32_t op_fmv_round_trip(uint32_t a) { return a; }
#endif

static const uint32_t vectors[][3] = {
	{ 0x3f800000, 0x40400000, 0x3dcccccd }, /* 1, 3, 0.1 */
	{ 0x3f800001, 0x3f7fffff, 0xbf800000 }, /* fma cancellation */
	{ 0x7f7fffff, 0x40000000, 0xff7fffff }, /* overflow */
	{ 0x00800000, 0x3f000000, 0x00000001 }, /* subnormal results */
	{ 0x00000003, 0x3f400000, 0x80000000 }, /* subnormal rounding */
	{ 0x80000000, 0x00000000, 0x80000000 }, /* signed zeros */
	{ 0x7f800000, 0x00000000, 0x3f800000 }, /* inf x 0 */
	{ 0x7fc00001, 0x3f800000, 0x40000000 }, /* quiet NaN */
	{ 0x7f800001, 0x3f800000, 0x40000000 }, /* signalling NaN */
	{ 0xc0200000, 0x501502f9, 0x40400000 }, /* -2.5, 1e10, 3 */
	{ 0x4f000000, 0xcf000000, 0x4f800000 }, /* 2^31, -2^31, 2^32 */
	{ 0x3effffff, 0xbf200000, 0x7f000000 }, /* below 0.5, -0.625, 2^127 */
	{ 0x0da24260, 0x1e3ce508, 0x000116c2 }, /* 1e-30, 1e-20, 1e-40 */
	{ 0x01000001, 0xffffffff, 0x80000001 }, /* integer-conversion edges */
	{ 0x3fc00000, 0x40200000, 0xbfc00000 }, /* 1.5, 2.5, -1.5: ties */
};
#define NVEC (sizeof(vectors) / sizeof(vectors[0]))

static uint32_t hash;
static void mix(uint32_t v)
{
	for (int i = 0; i < 4; i++) {
		hash ^= (v >> (8 * i)) & 0xff;
		hash *= 16777619u;
	}
}

static const char hexdigits[] = "0123456789abcdef";
static void put_hex(char *out, uint32_t v)
{
	for (int i = 7; i >= 0; i--) {
		out[i] = hexdigits[v & 0xf];
		v >>= 4;
	}
	out[8] = 0;
}

static void emit(const char *tag, uint32_t v)
{
	char hex[9];
	put_hex(hex, v);
	printf("%s %s\n", tag, hex);
}

#ifdef VERBOSE
#define RECORD(name, expr)                                    \
	do {                                                      \
		clear_flags();                                        \
		uint32_t r_ = (expr);                                 \
		uint32_t f_ = get_flags();                            \
		char h_[9], g_[9];                                    \
		put_hex(h_, r_);                                      \
		put_hex(g_, f_);                                      \
		printf("  %s %s %s\n", name, h_, g_ + 6);             \
		mix(r_);                                              \
		mix(f_);                                              \
	} while (0)
#else
#define RECORD(name, expr)        \
	do {                          \
		clear_flags();            \
		uint32_t r_ = (expr);     \
		mix(r_);                  \
		mix(get_flags());         \
	} while (0)
#endif

static const char *const mode_names[4] = { "rne", "rtz", "rdn", "rup" };

static void run(void)
{
	char tag[16];
	uint32_t all = 2166136261u;
	for (unsigned v = 0; v < NVEC; v++) {
		uint32_t a = vectors[v][0], b = vectors[v][1], c = vectors[v][2];
		for (uint32_t m = 0; m < 4; m++) {
			set_mode(m);
			hash = 2166136261u;
			RECORD("fadd", op_fadd(a, b));
			RECORD("fsub", op_fsub(a, c));
			RECORD("fmul", op_fmul(a, b));
			RECORD("fdiv", op_fdiv(a, b));
			RECORD("fdiv-ca", op_fdiv(c, a));
			RECORD("fsqrt", op_fsqrt(a));
			RECORD("fsqrt-b", op_fsqrt(b));
			RECORD("fmadd", op_fmadd(a, b, c));
			RECORD("fmsub", op_fmsub(a, b, c));
			RECORD("fnmadd", op_fnmadd(a, b, c));
			RECORD("fnmsub", op_fnmsub(a, b, c));
			RECORD("fcvt.w", op_fcvt_w(a));
			RECORD("fcvt.w-b", op_fcvt_w(b));
			RECORD("fcvt.wu", op_fcvt_wu(a));
			RECORD("fcvt.wu-b", op_fcvt_wu(b));
			RECORD("fcvt.s.w", op_fcvt_s_w(a));
			RECORD("fcvt.s.w-c", op_fcvt_s_w(c));
			RECORD("fcvt.s.wu", op_fcvt_s_wu(b));
			tag[0] = 'v';
			tag[1] = '0' + v / 10;
			tag[2] = '0' + v % 10;
			tag[3] = ' ';
			for (int i = 0; i < 3; i++)
				tag[4 + i] = mode_names[m][i];
			tag[7] = 0;
			emit(tag, hash);
			all ^= hash;
			all *= 16777619u;
		}
		set_mode(0);
		hash = 2166136261u;
		RECORD("fmin", op_fmin(a, b));
		RECORD("fmax", op_fmax(a, c));
		RECORD("fsgnj", op_fsgnj(a, b));
		RECORD("fsgnjn", op_fsgnjn(a, c));
		RECORD("fsgnjx", op_fsgnjx(b, c));
		RECORD("feq", op_feq(a, b));
		RECORD("feq-cc", op_feq(c, c));
		RECORD("flt", op_flt(a, b));
		RECORD("fle", op_fle(b, a));
		RECORD("fclass-a", op_fclass(a));
		RECORD("fclass-b", op_fclass(b));
		RECORD("fclass-c", op_fclass(c));
		RECORD("fmv", op_fmv_round_trip(c));
		tag[4] = 'c';
		tag[5] = 'm';
		tag[6] = 'p';
		emit(tag, hash);
		all ^= hash;
		all *= 16777619u;
	}
	emit("all", all);

	/* A static rounding mode overrides frm: 1/3 rounded toward zero while
	 * frm says round up. */
	set_mode(3);
#ifdef __riscv
	float q;
	asm volatile("fdiv.s %0, %1, %2, rtz" : "=f"(q) : "f"(F(0x3f800000)), "f"(F(0x40400000)));
	emit("static-rtz", U(q));
#else
	set_mode(1);
	emit("static-rtz", op_fdiv(0x3f800000, 0x40400000));
#endif

	/* Compiler-generated float code (FP loads and stores, fadd/fmul/fdiv,
	 * fcvt): Newton iterations for sqrt(2) and a 1..64 harmonic sum, round
	 * to nearest. Both builds use -ffp-contract=off, so neither fuses a
	 * multiply-add the other rounds twice. */
	set_mode(0);
	volatile float two = 2.0f;
	float x = 1.0f;
	for (int i = 0; i < 6; i++)
		x = 0.5f * (x + two / x);
	emit("newton-sqrt2", U(x));
	volatile float harmonic[64];
	for (int i = 0; i < 64; i++)
		harmonic[i] = 1.0f / (float)(i + 1);
	float sum = 0.0f;
	for (int i = 0; i < 64; i++)
		sum += harmonic[i];
	emit("harmonic64", U(sum));
}

#ifdef __riscv
int main(void)
{
	SystemInit();
	run();
	uint32_t mstatus;
	asm volatile("csrr %0, mstatus" : "=r"(mstatus));
	/* FS = Dirty (3) after FP work, and SD (bit 31) summarises it. */
	emit("mstatus-fs-sd", ((mstatus >> 13) & 3) | (mstatus >> 31) << 4);
	printf("done\n");
	while (1)
		;
}
#else
int main(void)
{
	run();
	emit("mstatus-fs-sd", 0x13);
	printf("done\n");
	return 0;
}
#endif

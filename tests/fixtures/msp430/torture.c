/*
 * MSP430 CPU exercise for the GDB-simulator lockstep
 * (crates/core/tests/msp430_gdb_sim_lockstep.rs). Compiler-generated code
 * only: integer arithmetic at every width, the hardware multiplier, libgcc
 * division and shifts, recursion, indirect calls, a jump table, block moves.
 * It ends in finished(), a jump to itself.
 */
#include <stdint.h>
#include <string.h>

volatile uint16_t seed = 0x1234;
volatile uint32_t sum;
static uint8_t buf[48];
static const uint16_t table[8] = {3, 1, 4, 1, 5, 9, 2, 6};

static uint16_t next(void)
{
    uint16_t s = seed;
    s ^= s << 7;
    s ^= s >> 9;
    s ^= s << 8;
    seed = s;
    return s;
}

static void mix(uint32_t v)
{
    sum = (sum << 5 | sum >> 27) ^ v;
}

__attribute__((noinline)) static uint16_t fib(uint16_t n)
{
    return n < 2 ? n : fib(n - 1) + fib(n - 2);
}

__attribute__((noinline)) static int16_t op_add(int16_t a, int16_t b) { return a + b; }
__attribute__((noinline)) static int16_t op_sub(int16_t a, int16_t b) { return a - b; }
__attribute__((noinline)) static int16_t op_and(int16_t a, int16_t b) { return a & b; }
__attribute__((noinline)) static int16_t op_xor(int16_t a, int16_t b) { return a ^ b; }

typedef int16_t (*binop)(int16_t, int16_t);
static const binop ops[4] = {op_add, op_sub, op_and, op_xor};

__attribute__((noinline)) static uint16_t classify(uint16_t v)
{
    switch (v & 7) {
    case 0: return v + 11;
    case 1: return v ^ 0x5a5a;
    case 2: return v >> 3;
    case 3: return v << 2;
    case 4: return (uint16_t)-v;
    case 5: return ~v;
    case 6: return v * 3;
    default: return v / 3;
    }
}

struct pair {
    uint32_t a;
    int16_t b;
    uint8_t c[5];
};

__attribute__((noinline)) static struct pair swap_pair(struct pair p)
{
    struct pair q = p;
    q.a = p.a * 2654435761UL;
    q.b = -p.b;
    q.c[0] = p.c[4];
    q.c[4] = p.c[0];
    return q;
}

__attribute__((noinline)) void finished(void)
{
    for (;;) {
    }
}

int main(void)
{
    *(volatile uint16_t *)0x0120 = 0x5a80; /* WDTCTL = WDTPW | WDTHOLD */

    for (uint16_t i = 0; i < 8; i++) {
        uint16_t a = next(), b = next() | 1;
        int16_t sa = (int16_t)a, sb = (int16_t)b;
        uint8_t ba = (uint8_t)a, bb = (uint8_t)b;
        int8_t ca = (int8_t)a, cb = (int8_t)b;
        uint32_t la = (uint32_t)a << 16 | b, lb = (uint32_t)b << 13 ^ a;
        int32_t sla = (int32_t)la, slb = (int32_t)lb | 1;
        uint64_t qa = (uint64_t)la << 32 | lb;

        mix(a + b);
        mix(a - b);
        mix((uint32_t)a * b);
        mix((int32_t)sa * sb);
        mix(a / b);
        mix(a % b);
        mix((uint16_t)(sa / sb));
        mix((uint16_t)(sa % sb));
        mix(sa >> (b & 15));
        mix(a >> (b & 15));
        mix(a << (b & 15));
        mix(ba + bb);
        mix((uint8_t)(ba - bb));
        mix((uint8_t)(ba * bb));
        mix(ca >> (bb & 7));
        mix(ca < cb);
        mix(sa < sb);
        mix(a < b);
        mix(sa >= sb);
        mix(la + lb);
        mix(la - lb);
        mix(la * lb);
        mix(la / (lb | 1));
        mix(la % (lb | 1));
        mix((uint32_t)(sla / slb));
        mix(sla >> (b & 31));
        mix(la >> (b & 31));
        mix(la << (b & 31));
        mix(sla < slb);
        mix(la < lb);
        mix((uint32_t)(qa * 6364136223846793005ULL >> 32));
        mix((uint32_t)(qa / (lb | 1)));
        mix(ops[i & 3](sa, sb));
        mix(classify(a));
        mix(classify(b));
        mix(table[a & 7] * table[b & 7]);
    }

    for (uint16_t i = 0; i < sizeof buf; i++)
        buf[i] = (uint8_t)next();
    memmove(buf + 3, buf, 20);
    memset(buf + 30, 0xa5, 9);
    mix(memcmp(buf, buf + 3, 10));
    for (uint16_t i = 0; i < sizeof buf; i++)
        mix(buf[i]);

    struct pair p = {0x89abcdefUL, -1234, {1, 2, 3, 4, 5}};
    p = swap_pair(p);
    mix(p.a);
    mix(p.b);
    mix(p.c[0] << 8 | p.c[4]);

    mix(fib(12));
    finished();
    return 0;
}

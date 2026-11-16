/* LabWired shared-memory co-simulation client (single header, Linux/POSIX).
 *
 * Opt in from the manifest with `config: {transport: shm}`. The host passes the
 * mapping's path in LABWIRED_COSIM_SHM; lw_open() maps it and announces itself
 * with one stdout line so the host knows to use it. A model that never calls
 * lw_open() stays a plain JSONL model.
 *
 *   lw_t m;
 *   if (lw_open(&m)) return 1;
 *   int gpio = lw_input_index(&m, "gpio"), vout = lw_output_index(&m, "v_out");
 *   while (lw_wait(&m)) {                      // 0 once the host closes
 *       double on = lw_in_f64(&m, gpio);       // bool/int/float read as double
 *       lw_set_f64(&m, vout, on > 0.5 ? 3.3 : 0.0);
 *       lw_reply(&m);
 *   }
 *
 * Layout and protocol: crates/core/src/cosim/shm.rs.
 */
#ifndef LABWIRED_SHM_H
#define LABWIRED_SHM_H
#define _GNU_SOURCE
#include <fcntl.h>
#include <linux/futex.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <time.h>
#include <unistd.h>

#define LW_MAGIC 0x4D53574Cu
#define LW_SPIN_NS 150000L

typedef struct { uint32_t present, kind; uint64_t bits; } lw_slot;

typedef struct {
    unsigned char *base;
    size_t len;
    uint32_t n_in, n_out;
    lw_slot *in, *out;
    const char *names;
    uint32_t seen; /* last request sequence number served */
} lw_t;

#define LW_U32(m, off) ((_Atomic uint32_t *)((m)->base + (off)))

static inline long lw_futex(_Atomic uint32_t *w, int op, uint32_t val,
                            const struct timespec *ts) {
    return syscall(SYS_futex, w, op, val, ts, 0, 0);
}

static inline long lw_now_ns(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1000000000L + t.tv_nsec;
}

/* Map the transport and announce it to the host. Returns 0 on success. */
static inline int lw_open(lw_t *m) {
    const char *path = getenv("LABWIRED_COSIM_SHM");
    if (!path) return -1;
    int fd = open(path, O_RDWR);
    if (fd < 0) return -1;
    struct stat st;
    if (fstat(fd, &st) < 0) { close(fd); return -1; }
    m->len = (size_t)st.st_size;
    m->base = mmap(NULL, m->len, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    close(fd);
    if (m->base == MAP_FAILED) return -1;
    if (*(uint32_t *)m->base != LW_MAGIC || *(uint32_t *)(m->base + 4) != 1) return -1;
    m->n_in = *(uint32_t *)(m->base + 8);
    m->n_out = *(uint32_t *)(m->base + 12);
    m->names = (const char *)(m->base + *(uint32_t *)(m->base + 16));
    m->in = (lw_slot *)(m->base + *(uint32_t *)(m->base + 24));
    m->out = m->in + m->n_in;
    m->seen = atomic_load(LW_U32(m, 64));
    fputs("{\"labwired_shm\":1}\n", stdout);
    fflush(stdout);
    return 0;
}

static inline int lw_index(const lw_t *m, uint32_t first, uint32_t count, const char *name) {
    const char *p = m->names;
    for (uint32_t i = 0; i < first; i++) p += strlen(p) + 1;
    for (uint32_t i = 0; i < count; i++, p += strlen(p) + 1)
        if (!strcmp(p, name)) return (int)i;
    return -1;
}
/* Slot index of a declared input / output, or -1. */
static inline int lw_input_index(const lw_t *m, const char *name) {
    return lw_index(m, 0, m->n_in, name);
}
static inline int lw_output_index(const lw_t *m, const char *name) {
    return lw_index(m, m->n_in, m->n_out, name);
}

/* Block for the next step. Returns 1 for a step, 0 when the host is closing. */
static inline int lw_wait(lw_t *m) {
    _Atomic uint32_t *req = LW_U32(m, 64), *sleeping = LW_U32(m, 68);
    long start = lw_now_ns();
    for (;;) {
        uint32_t cur = atomic_load_explicit(req, memory_order_acquire);
        if (cur != m->seen) {
            m->seen = cur;
            return atomic_load(LW_U32(m, 136)) ? 0 : 1;
        }
        if (lw_now_ns() - start < LW_SPIN_NS) continue;
        atomic_store_explicit(sleeping, 1, memory_order_relaxed);
        atomic_thread_fence(memory_order_seq_cst);
        if (atomic_load_explicit(req, memory_order_acquire) == m->seen) {
            struct timespec ts = {0, 50 * 1000000L};
            lw_futex(req, FUTEX_WAIT, m->seen, &ts);
        }
        atomic_store_explicit(sleeping, 0, memory_order_relaxed);
    }
}

static inline uint64_t lw_time_ns(const lw_t *m) { return *(uint64_t *)(m->base + 192); }
static inline uint64_t lw_dt_ns(const lw_t *m) { return *(uint64_t *)(m->base + 200); }

/* Input `i` as a double (0 when absent); lw_in_present tells the two apart. */
static inline int lw_in_present(const lw_t *m, int i) { return m->in[i].present != 0; }
static inline double lw_in_f64(const lw_t *m, int i) {
    const lw_slot *s = &m->in[i];
    if (!s->present) return 0.0;
    if (s->kind == 2) { double d; memcpy(&d, &s->bits, 8); return d; }
    if (s->kind == 1) return (double)(int64_t)s->bits;
    return s->bits ? 1.0 : 0.0;
}
static inline int lw_in_bool(const lw_t *m, int i) { return lw_in_f64(m, i) != 0.0; }

static inline void lw_set_f64(lw_t *m, int o, double v) {
    lw_slot *s = &m->out[o];
    memcpy(&s->bits, &v, 8);
    s->kind = 2;
    s->present = 1;
}
static inline void lw_set_i64(lw_t *m, int o, int64_t v) {
    lw_slot *s = &m->out[o];
    s->bits = (uint64_t)v;
    s->kind = 1;
    s->present = 1;
}
static inline void lw_set_bool(lw_t *m, int o, int v) {
    lw_slot *s = &m->out[o];
    s->bits = v != 0;
    s->kind = 0;
    s->present = 1;
}
static inline void lw_clear_output(lw_t *m, int o) { m->out[o].present = 0; }
/* Report a model failure; the host stops the run. */
static inline void lw_fault(lw_t *m) { atomic_store(LW_U32(m, 140), 1); }

/* Publish this step's outputs. */
static inline void lw_reply(lw_t *m) {
    _Atomic uint32_t *resp = LW_U32(m, 128);
    atomic_store_explicit(resp, m->seen, memory_order_release);
    atomic_thread_fence(memory_order_seq_cst);
    if (atomic_load_explicit(LW_U32(m, 132), memory_order_relaxed))
        lw_futex(resp, FUTEX_WAKE, 0x7fffffff, NULL);
}
#endif

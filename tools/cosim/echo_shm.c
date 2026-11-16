/* Echo plant over the shared-memory transport: v_out = 3.3 when gpio is high.
 * Build: cc -O2 -o echo_shm echo_shm.c */
#include "labwired_shm.h"

int main(void) {
    lw_t m;
    if (lw_open(&m)) return 1;
    int gpio = lw_input_index(&m, "gpio"), vout = lw_output_index(&m, "v_out");
    while (lw_wait(&m)) {
        lw_set_f64(&m, vout, lw_in_f64(&m, gpio) > 0.5 ? 3.3 : 0.0);
        lw_reply(&m);
    }
    return 0;
}

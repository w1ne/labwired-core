# sam4s_xplained — negative control: chip without the TC blocks (2026-11-08)

Same firmware (firmware-sam4s-xplained-renode-smoke) and smoke assertions, but
the chip descriptor with `tc0_1_2` / `tc3_4_5` removed
(`sam4s_xplained-notc-chip.yaml`):

    FAIL  2/5 checks · sam4s_xplained-notc-smoke · 4000000 steps
    FAULT HardFault escalated from a precise BusFault: data access at 0x4001_0004
          (TC0 TC_CMR) ... BFAR=0x40010004

So the TC lines of the passing smoke come from the `sam4s_tc` model, not from
a window that answers anything. Re-run from the repo root:

    labwired test --script docs/coverage/renode-onboarding/attempt-history/sam4s_xplained/2026-11-08-negative-control-no-tc/sam4s_xplained-notc-smoke.yaml --output-dir /tmp/notc

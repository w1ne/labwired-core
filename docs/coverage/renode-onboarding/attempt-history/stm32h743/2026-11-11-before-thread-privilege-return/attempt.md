# STM32H743 CPACR baseline before the Thread privilege return assertion

- Preserves the prior passing target-only measurement before checking the
  Thread privilege state after SVC exception return.
- The baseline passed all 20 smoke assertions after 4,000,000 instructions;
  JUnit recorded 21 cases including the run and zero failures or errors.
- The new assertion checks that the SVC handler's CPACR access does not change
  the interrupted Thread mode's unprivileged CONTROL state on exception return.

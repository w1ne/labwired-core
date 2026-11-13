Negative control: the same smoke with B1 declared `active_high: false`
(pressed = low). FAIL 3/5: PI11 never reads low at idle with the internal
pull-down, so the B1 check spins out the step budget and nothing after the
EXTI software-trigger check prints. UM1907 §6.6: B1 USER connects PI11 to
VDD when pressed.

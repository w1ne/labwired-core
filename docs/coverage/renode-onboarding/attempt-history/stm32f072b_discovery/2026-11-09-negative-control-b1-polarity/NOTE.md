Negative control: the same smoke with B1 declared `active_high: false`
(pressed = low, the polarity the previous disco firmware assumed). FAIL 3/5:
PA0 never reads low at idle with the internal pull-down, so the B1 check
spins out the step budget and nothing after the LED walk prints. UM1690
§6.5: B1 USER connects PA0 to VDD.

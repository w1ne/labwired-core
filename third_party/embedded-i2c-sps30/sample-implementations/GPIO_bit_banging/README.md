# GPIO based Software I2C

This implementation of the I2C HAL uses bit banging on GPIOs to run
the I2C communication. It is different from other sample-implementations.
It uses an additional pair of `.c` and `.h` file to make the implementation
generic, in a way that GPIO bitbanging can be performed on any system providing
access to GPIOs. Note that in this case the pull-up resistors in the electric
circuit are still needed, or need to be enabled manually if your microprocessor
provides this option. These files are called `sensirion_i2c_gpio.[ch]`.
Similar to the I2C HAL, we provide an unimplemented template in this  in this
folder to implement yourself for your platform and sample implementations in the
so called folder.

## Getting started

To use this I2C HAL copy `sensirion_i2c_hal.c` and `sensirion_i2c_gpio.h`
from this folder to the main folder of your driver. Then either choose a
sample implementation or implement `sensirion_i2c_gpio.c` and copy
it to the main driver folder as well.

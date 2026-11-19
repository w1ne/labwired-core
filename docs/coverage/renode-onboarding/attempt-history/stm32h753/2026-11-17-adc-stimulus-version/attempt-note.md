# Failed H753 ADC stimulus schema attempt

The fixture was moved to the system descriptor and parsing advanced to smoke
validation, which rejected `stimuli` under schema version 1.0. Scripted stimuli
require schema version 1.2. This run stopped before boot; its generated logs,
metrics, output, and original smoke input are preserved here.

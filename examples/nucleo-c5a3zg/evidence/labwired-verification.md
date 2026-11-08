# Hosted LabWired verification status

The hosted board catalog does not currently expose NUCLEO-C5A3ZG or an
STM32C5A3ZG target for `labwired_verify`, so this round has no hosted
`model_verified` verdict. The committed onboarding smoke is separate local
simulator evidence. It asserts the expected GPIOA:5 edges and records the
simulator's ideal-VDD assumption.

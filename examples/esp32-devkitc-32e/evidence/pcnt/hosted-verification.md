# Hosted PCNT smoke verification

The LabWired hosted verifier returned `status: model_verified` for this
firmware with the oracle `GPIO16 is high`. The sketch drives GPIO16 high only
after PCNT unit 0 reads exactly eight rising edges from GPIO23. The matched
oracle result was `GPIO16 is high (gpio_pins pad)`. The hosted run used its
generic classic ESP32 Arduino board profile; the committed local smoke uses the
exact DevKitC-32E chip and system descriptors.

The run used the hosted classic ESP32 Arduino profile, completed 5,000,000
steps, and reached the normal empty Arduino `loop()`. The verifier reported one
non-blocking power-model approximation: no power net was routed, so VDD was
assumed to be an ideal 3.3 V rail and power-on/brown-out reset was not
simulated. This is digital-twin evidence, not a physical-board run.

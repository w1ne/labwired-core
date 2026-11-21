# STM32H743 baseline before privileged CPACR denial smoke

This directory preserves the passing Round 540 measurement for the exact
imported `stm32h743` system before adding the CPACR=00 privileged Thread-mode
check. The baseline had 20 smoke assertions and completed 4,000,000 instructions
to `MaxSteps`.

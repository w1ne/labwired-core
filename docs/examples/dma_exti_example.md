# Example: DMA and EXTI Interaction

This example demonstrates how to set up a DMA transfer and trigger an external interrupt in LabWired.

## 1. DMA Memory-to-Memory Transfer

The `DMA1` controller supports memory-to-memory transfers by setting the `MEM2MEM` bit in the `CCR` register.

```rust
// Pseudocode for configuring DMA1 Channel 1 for MEM2MEM
let dma1_base = 0x4002_0000;
let ch1_ccr = dma1_base + 0x08;
let ch1_cndtr = dma1_base + 0x0C;
let ch1_cpar = dma1_base + 0x10;
let ch1_cmar = dma1_base + 0x14;

// 1. Set source and destination addresses
write32(ch1_cpar, src_addr);
write32(ch1_cmar, dest_addr);

// 2. Set number of bytes to transfer
write32(ch1_cndtr, 10);

// 3. Enable DMA channel with MEM2MEM, MINC, PINC, and TCIE
write32(ch1_ccr, (1 << 14) | (1 << 7) | (1 << 6) | (1 << 1) | (1 << 0));
```

## 2. EXTI External Interrupt

The `EXTI` controller maps external signals (from GPIO or other sources) to NVIC interrupts.

```rust
// Pseudocode for triggering EXTI Line 0 (mapped to NVIC IRQ 6)
let exti_base = 0x4001_0400;
let exti_imr = exti_base + 0x00;
let exti_swier = exti_base + 0x10;

// 1. Unmask EXTI Line 0
write32(exti_imr, 1 << 0);

// 2. Trigger software interrupt on Line 0
write32(exti_swier, 1 << 0);
```

### Interrupt Mapping
In LabWired's STM32F103 model:
- `EXTI0` -> NVIC IRQ 6
- `EXTI1` -> NVIC IRQ 7
- `EXTI2` -> NVIC IRQ 8
- `EXTI3` -> NVIC IRQ 9
- `EXTI4` -> NVIC IRQ 10
- `EXTI9_5` -> NVIC IRQ 23
- `EXTI15_10` -> NVIC IRQ 40

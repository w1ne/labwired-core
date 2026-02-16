# Example: DMA and Interrupt Logic

LabWired models Direct Memory Access (DMA) and External Interrupts (EXTI) using a deterministic two-phase execution cycle. This ensures that concurrent hardware events (like memory transfers and IRQ assertion) occur in a predictable order relative to CPU instruction execution.

## 1. DMA Memory-to-Memory Transfer

The DMA controller processes transfer requests at the end of the CPU instruction cycle.

### Configuration
To initiate a memory-to-memory transfer, the firmware must configure the DMA channel with the `MEM2MEM` bit set.

```rust
// DMA1 Channel 1 Configuration
fn configure_dma_transfer(src: u32, dest: u32, len: u16) {
    let dma = unsafe { &*DMA1::ptr() };
    
    // 1. Source and Destination Addresses
    dma.ch1.cpar.write(|w| unsafe { w.bits(src) });
    dma.ch1.cmar.write(|w| unsafe { w.bits(dest) });
    
    // 2. Transfer Length
    dma.ch1.cndtr.write(|w| unsafe { w.bits(len as u32) });
    
    // 3. Control Register (Enable, Mem2Mem, Increment Ptrs)
    dma.ch1.ccr.write(|w| w
        .mem2mem().set_bit() 
        .pl().very_high()
        .minc().enabled()
        .pinc().enabled()
        .en().enabled()
    );
}
```

### Execution Model
1.  **Request Phase**: When `CCR.EN` is set, the DMA controller registers a pending request internally. It does *not* immediately modify memory.
2.  **Bus Arbitration**: At the end of the current CPU cycle, the `SystemBus` polls the DMA controller.
3.  **Transfer Phase**: The bus executes the memory copy operation (read from source, write to destination).

## 2. External Interrupts (EXTI)

External signals are routed through the EXTI controller to the Nested Vectored Interrupt Controller (NVIC).

### Stimulus Configuration
To verify interrupt logic, use a test script to assert a GPIO pin state.

**System Manifest (`system.yaml`):**
```yaml
chip: "../chips/stm32f103.yaml"
inputs:
  - id: "user_button"
    pin: "PA0"
    mode: "PushPull"
```

**Test Script (`test_interrupts.yaml`):**
```yaml
steps:
  - run: 100ms
  - set_pin: 
      pin: "PA0"
      state: "high"
  - run: 1us
  - assert_interrupt: "EXTI0"
```

### Signal Propagation
1.  **GPIO**: The pin state change is detected by the GPIO peripheral.
2.  **AFIO**: The signal is routed to the corresponding EXTI line (e.g., PA0 -> EXTI0) based on the AFIO_EXTICR configuration.
3.  **EXTI**: The controller detects the rising/falling edge and sets the Pending Register (PR) bit.
4.  **NVIC**: The interrupt is forwarded to the NVIC. If the priority acts, the CPU vectors to the ISR on the next instruction fetch.

# Hardware Interaction Guide: DMA and Interrupts

This guide explains how LabWired achieves cycle-accurate, deterministic simulation of complex hardware events like DMA transfers and interrupt propagation.

## The Problem: Non-Deterministic Simulation
In many simulators, peripherals run in separate threads or use real-world timers. This leads to "Heisenbugs"—interrupts that fire at different times on every run, making it impossible to reproduce rare race conditions.

## The Solution: The "Two-Phase" Heartbeat
LabWired eliminates non-determinism by tying all hardware events to the CPU's instruction cycle. Every instruction executed by the CPU is followed by a "Bus Heartbeat" consisting of two phases:

### Phase 1: The Tick (Intention)
The `SystemBus` calls `tick()` on every peripheral. 
-   **Responsibility**: The peripheral updates its internal state (e.g., a timer increments, a UART buffer fills).
-   **Output**: It returns a `PeripheralTickResult`, which is essentially a manifest of what the peripheral *wants* to happen on the bus.

### Phase 2: The Execution (Action)
The `SystemBus` collects all results and performs the requested actions:
-   **Memory Transfers**: Performs `DmaRequest` copies.
-   **Signal Routing**: Forwards `dma_signals` to controllers.
-   **Interrupt Propagation**: Sets pending bits in the NVIC.

---

## Implementing the Peripheral Trait

To bridge your hardware logic with the simulation engine, you must implement the `tick()` method.

```rust
pub struct PeripheralTickResult {
    /// Signal the primary IRQ assigned to this peripheral in the manifest.
    pub irq: bool,
    
    /// Requests for the Bus to perform memory operations (DMA).
    pub dma_requests: Option<Vec<DmaRequest>>,
    
    /// High-fidelity: Specific IRQ numbers to trigger (overrides `irq`).
    pub explicit_irqs: Option<Vec<u32>>,
    
    /// Side-band signals for a DMA Controller (e.g., "RX Buffer Full").
    pub dma_signals: Option<Vec<u32>>,
    
    /// Cycles spent in this tick (usually 0 or 1).
    pub cycles: u32,
}
```

### 1. Handling Interrupts
#### The Simple Way (`irq: true`)
If your peripheral only has one interrupt (common for simple devices like Timers), just set `irq: true`. The `SystemBus` automatically maps this to the IRQ defined in your `system.yaml`.

#### The Advanced Way (`explicit_irqs`)
For complex SoCs where one peripheral might trigger multiple different vectors:
```rust
fn tick(&mut self) -> PeripheralTickResult {
    let mut irqs = Vec::new();
    if self.rx_complete { irqs.push(UART_RX_IRQ); }
    if self.tx_complete { irqs.push(UART_TX_IRQ); }
    
    PeripheralTickResult {
        explicit_irqs: if irqs.is_empty() { None } else { Some(irqs) },
        ..Default::default()
    }
}
```

### 2. Modeling DMA (Direct Memory Access)
LabWired supports two architectural patterns for DMA:

#### Pattern A: EasyDMA (Peripheral-Led)
In modern chips like the **nRF52**, the peripheral itself manages the DMA transfer. It knows the source RAM pointer and just tells the bus: "Read this byte from RAM and move it to my internal register."

**Example: nRF52 UARTE Implementation**
```rust
impl Peripheral for Uarte {
    fn tick(&mut self) -> PeripheralTickResult {
        if !self.transfer_active || self.bytes_sent >= self.total_bytes {
            return PeripheralTickResult::default();
        }

        // 1. Define the DMA transfer for this single cycle
        let req = DmaRequest {
            src_addr: (self.ptr_register + self.bytes_sent) as u64,
            addr: self.fifo_addr as u64,
            direction: DmaDirection::Read, // Pull data from RAM
            val: 0,
        };

        self.bytes_sent += 1;
        
        // 2. Wrap it and potentially signal ENDTX interrupt
        let mut res = PeripheralTickResult::default();
        res.dma_requests = Some(vec![req]);
        
        if self.bytes_sent == self.total_bytes {
            self.transfer_active = false;
            res.irq = true; // Trigger "Transfer Complete" IRQ
        }
        res
    }
}
```

#### Pattern B: Controller-Led (STM32 Style)
Traditional DMA uses a central controller (DMA1, GPDMA). Peripherals send "Requests" (dma_signals) to the controller, which then masters the bus.

1.  **Peripheral** returns `dma_signals: Some(vec![UART_TX_READY])`.
2.  **SystemBus** routes signal `1` to the **DMA Controller**.
3.  **DMA Controller** increments its own pointers and returns a `DmaRequest`.

---

## Architectural Implications
-   **Serialization**: If multiple peripherals return DMA requests in the same cycle, the `SystemBus` executes them in the order they appear in the peripheral list (deterministic arbitration).
-   **Zero-Copy Simulation**: For high-performance "Mem-to-Mem" transfers, use `DmaDirection::Copy`. The `SystemBus` will perform a direct buffer move without the CPU ever seeing the intermediate data.
-   **Error Handling**: If a DMA transfer hits an unmapped memory region, the `SystemBus` returns a `MemoryViolation` error, which can be used to simulate a `BusFault` exception.

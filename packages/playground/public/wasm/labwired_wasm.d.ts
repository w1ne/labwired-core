/* tslint:disable */
/* eslint-disable */

export class WasmSimulator {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Snapshot of the shared virtual-air TX trace ring buffer (last
     * ~200 BLE/proprietary frames pushed by any chip in this WASM
     * instance, most-recent-first). The playground's BLE-on-canvas
     * visualization polls this to render the packet trace panel; the
     * underlying state lives in a Rust static, so any WasmSimulator
     * can return the same snapshot — pick whichever chip is alive.
     */
    air_trace_snapshot(): any;
    apply_agentdeck_quirks(): void;
    /**
     * Apply a binary `MachineRuntimeSnapshot` (LWRS-framed bincode blob,
     * produced by `labwired-cli snapshot capture` or `Machine::take_runtime_snapshot`)
     * to the currently-loaded machine. Bypasses the cold boot — the firmware
     * resumes mid-flight from the captured CPU + peripheral state.
     *
     * Must be called after firmware has been loaded onto the same system
     * manifest (peripheral names + CPU arch must match the snapshot). On
     * mismatch the call returns an error and the machine state is left
     * partially overwritten — callers should treat that as a hard reset.
     */
    apply_runtime_snapshot(bytes: Uint8Array): void;
    /**
     * Bench runner: execute `cycles` `step_with_esp32_aids` iterations
     * and return elapsed milliseconds (measured via
     * `performance.now()`). The caller drives this twice — once with
     * `set_jit_enabled(false)`, once with `set_jit_enabled(true)` —
     * and compares the two numbers to quantify JIT speedup.
     *
     * Returns a `Result<f64, JsValue>`: the `Err` path bubbles step
     * errors so the bench harness can show a useful message.
     */
    bench_jit(cycles: number): number;
    /**
     * Drain UART TX output bytes accumulated since the last call.
     */
    drain_uart_output(): Uint8Array;
    /**
     * Push bytes into all UART RX buffers (bidirectional serial input).
     */
    feed_uart_input(data: Uint8Array): void;
    /**
     * Browser-side GDB stub entry point.
     *
     * Disabled in this build: the GdbStub `Target` impl in `labwired-gdbstub`
     * is concrete on `LabwiredTarget<CortexM>` / `LabwiredTarget<RiscV>`,
     * but `WasmSimulator` now holds `Machine<Box<dyn Cpu>>` so the bound
     * isn't satisfied. The playground has no JS caller for this method,
     * so we return an empty packet rather than refactor `labwired-gdbstub`
     * to be dyn-aware. Track via the v0.6 plan.
     */
    gdb_process_packet(_packet: Uint8Array): Uint8Array;
    /**
     * Read back the current state of all NTC thermistor devices declared in `board_io`.
     *
     * Returns `[{ id, kind: "ntc-thermistor", temperature_c, divider_mv, adc_count }]`.
     * All conversion math (Steinhart-Hart, mV→count) is performed here by calling into
     * core types — no conversion logic in this WASM bridge body.
     */
    get_adc_device_states(): any;
    /**
     * Returns analog state for ADC and PWM board_io bindings.
     */
    get_board_io_analog_states(): any;
    /**
     * Returns the board_io configuration as a JSON array.
     * Each entry: { id, kind, peripheral, pin, signal, active_high }
     */
    get_board_io_config(): any;
    /**
     * Returns the current state of all board_io bindings as a JSON array.
     * Each entry: { id, active }
     * Uses peripheral snapshot() to read ODR regardless of register layout.
     */
    get_board_io_states(): any;
    get_disassembly(): string;
    /**
     * Read back the current sensor data from each I2C sensor declared in `board_io`.
     * Returns `[{ id, kind: "adxl345", x, y, z }, ...]` or `[{ id, kind: "mpu6050", ax, ay, az, gx, gy, gz }, ...]`
     * or `[{ id, kind: "bme280", temperature_c, humidity_pct, pressure_hpa }, ...]`.
     */
    get_i2c_sensor_states(): any;
    /**
     * Return the ILI9341 RGB565 framebuffer for the device identified by `device_id`.
     *
     * `device_id` must match a `board_io` binding with `device_type: "ili9341"`.
     * Returns a 153,600-byte `Uint8Array` (240×320 pixels × 2 bytes, row-major, big-endian RGB565).
     * Returns a JS error if the device is not found.
     */
    get_ili9341_framebuffer(device_id: string): Uint8Array;
    /**
     * Read the IO-Link master peer's live state: `{ link_state, pd_valid,
     * input_byte }`. Returns `null` if no master is wired.
     */
    get_iolink_master_state(): any;
    /**
     * Legacy LED state query (hardcoded GPIOB pin 5 for backward compat).
     */
    get_led_state(): boolean;
    get_pc(): number;
    /**
     * Return the PCD8544 (Nokia 5110) framebuffer for the device identified
     * by `device_id`.
     *
     * `device_id` must match a `board_io` binding with `device_type:
     * "pcd8544"`. Returns 504 bytes: 84 columns × 6 banks, bank-major. Pixel
     * (x, y) is bit `(y % 8)` of byte `[(y / 8) * 84 + x]` (1 = on/dark).
     */
    get_pcd8544_framebuffer(device_id: string): Uint8Array;
    /**
     * List all peripherals: [{ name, base_address }]
     */
    get_peripheral_list(): any;
    /**
     * Get a peripheral's full state snapshot as JSON.
     */
    get_peripheral_snapshot(name: string): any;
    get_register(id: number): number;
    get_register_names(): any;
    /**
     * Read the 74HC165's live input byte (bit `i` = channel `i`), or `-1` if
     * no shifter is wired. Lets the UI reflect the device's real state rather
     * than tracking it in JS.
     */
    get_sn74hc165_inputs(): number;
    /**
     * Read back the current state of each SPI sensor declared in `board_io`.
     * Returns `[{ id, kind: "max31855", tc_c, internal_c }, ...]`.
     */
    get_spi_device_states(): any;
    /**
     * Return the SSD1306 GDDRAM framebuffer for the device identified by `device_id`.
     *
     * `device_id` must match a `board_io` binding with `device_type: "oled-ssd1306"`.
     * Returns a 1024-byte `Uint8Array` (128 columns × 8 pages, page-major).
     * Returns a JS error if the device is not found.
     */
    get_ssd1306_framebuffer(device_id: string): Uint8Array;
    /**
     * Return the SSD1680 tri-color e-paper framebuffer for the device identified by `device_id`.
     *
     * `device_id` must match a `board_io` binding with `device_type: "ssd1680_tricolor_290"`.
     * Returns a 9472-byte `Uint8Array`: first 4736 bytes are the black plane
     * (1 = white / 0 = black), next 4736 bytes are the red plane on the wire
     * (1 = no-red / 0 = red — see GxEPD2 inversion in writeImage). Row-major,
     * 128 pixels wide / 296 tall native, MSB-first packing within each byte.
     * Returns a JS error if the device is not found.
     */
    get_ssd1680_framebuffer(device_id: string): Uint8Array;
    /**
     * Cheap accessor returning just the SSD1680 refresh-generation counter.
     * UI uses this to decide whether to re-fetch the (larger) framebuffer.
     */
    get_ssd1680_refresh_generation(device_id: string): number;
    /**
     * Read back the current state of all NEO-6M GPS devices declared in `board_io`.
     * Returns `[{ id, kind: "neo6m-gps", lat, lon, has_fix }]`.
     */
    get_uart_device_states(): any;
    /**
     * Same shape as [`get_ssd1680_framebuffer`] but for the UC8151D-family
     * tri-color panel attached by [`install_arduino_esp32_quirks`]. The
     * board_io binding type may say `ssd1680_tricolor_290` (since system
     * YAMLs were authored before the UC8151D split); we ignore that and
     * just find a `Uc8151dTricolor290` on the named SPI peripheral.
     */
    get_uc8151d_framebuffer(device_id: string): Uint8Array;
    /**
     * Cheap accessor returning just the UC8151D refresh-generation counter.
     */
    get_uc8151d_refresh_generation(device_id: string): number;
    /**
     * Auto-discovery counterpart to [`Self::install_esp32_arduino_quirks`].
     *
     * Mirrors the CLI's `arduino-esp32` snapshot-capture profile —
     * resolves Arduino-ESP32 thunk PCs from the ELF symbol table instead
     * of hand-curated hardcoded addresses. Works for any GxEPD2-class
     * sketch (labwired-ereader, future user sketches) without needing
     * to know its binary layout in advance.
     *
     * Caller must pass the same ELF bytes that were loaded via
     * `load_firmware`. The thunks are installed as flash patches over
     * the resolved PCs; calling this without the matching ELF is a no-op
     * (symbols don't resolve → no thunks installed).
     *
     * Also attaches a `Uc8151dTricolor290` panel to spi3 (the SSD1680
     * panel attached by default doesn't decode UC8151D opcodes
     * `0x00 PSR` / `0x04 PON` / `0x10 DTM1` / `0x12 DRF` / `0x13 DTM2`
     * that GxEPD2_290_C90c / Z13c emits).
     */
    install_arduino_esp32_quirks(elf_bytes: Uint8Array): void;
    install_esp32_arduino_quirks(): void;
    /**
     * Clear the IO-Link master's trace ring.
     */
    iolink_trace_clear(): void;
    /**
     * Snapshot of the IO-Link master's captured transactions (oldest→newest),
     * for the IO-Link Analyzer instrument. Empty array if no master is wired.
     */
    iolink_trace_snapshot(): any;
    /**
     * Total number of times the browser JIT has dispatched a
     * compiled block. Useful for confirming the JIT path actually
     * fired during a benchmark.
     */
    jit_hits(): bigint;
    /**
     * Total number of JIT refusals (host bus errors, JS-side
     * dispatch failures). Surfaced for the bench harness so it can
     * distinguish "JIT was tried and rejected" from "JIT was never
     * hit because PC never reached the block".
     */
    jit_refusals(): bigint;
    /**
     * Re-write the dual-core handshake bytes. Call every ~10k steps from JS
     * — firmware boot code revisits these and we need them to stay 1.
     */
    keep_alive_esp32_dual_core(): void;
    /**
     * Legacy constructor: hardcoded STM32F107 Cortex-M3 with 128KB flash + 20KB RAM.
     * Kept for backward compatibility with the existing landing page sandbox.
     */
    constructor(firmware: Uint8Array);
    /**
     * Config-driven constructor: initialize from system YAML, chip YAML, and firmware ELF.
     *
     * Dispatches on `chip.arch`:
     *   * `Arm` → `SystemBus::from_config` + `configure_cortex_m` (existing path).
     *   * `Xtensa` → `configure_xtensa_esp32` + inline external-device attach.
     *     ESP32 chip YAMLs declare RAM banks (IRAM/DRAM/flash XIP/ROM) via
     *     `peripherals: [{type: ram, ...}]`, which `from_config` doesn't
     *     understand — it'd stub them out and break instruction fetch. So
     *     ESP32 takes the dedicated path that explicitly registers those
     *     banks before attaching SPI / I²C external devices.
     */
    static new_from_config(system_yaml: string, chip_yaml: string, firmware: Uint8Array): WasmSimulator;
    read_memory(addr: number, len: number): Uint8Array;
    /**
     * Inject an ADC value into a named ADC peripheral's data register.
     */
    set_adc_value(peripheral_name: string, value: number): void;
    /**
     * Set an input board_io binding (e.g. button press).
     * Writes to the GPIO IDR register bit for the specified binding.
     */
    set_board_io_input(id: string, active: boolean): void;
    /**
     * Enable or disable the GPS fix on a NEO-6M module.
     */
    set_gps_fix(device_id: string, active: boolean): void;
    /**
     * Set the simulated position on a NEO-6M GPS module attached to a UART peripheral.
     *
     * `device_id` must match a `board_io` binding with `device_type: "neo6m-gps"`.
     */
    set_gps_position(device_id: string, lat: number, lon: number): void;
    /**
     * Set the distance (cm) reported by an HC-SR04 ultrasonic sensor — the
     * host-controlled "hand position" that drives gesture control. Clamped to
     * the sensor's 2–400 cm range.
     */
    set_hcsr04_distance(id: string, distance_cm: number): void;
    /**
     * Set the simulated X/Y/Z sample on an ADXL345 attached to an I2C peripheral.
     * Looks up the binding in `board_io` by id; the binding must have
     * `device_type: "adxl345"`.
     */
    set_i2c_sensor_sample(device_id: string, x: number, y: number, z: number): void;
    /**
     * Set the simulated 6-DoF sample on an MPU6050 attached to an I2C peripheral.
     */
    set_i2c_sensor_sample_6dof(device_id: string, ax: number, ay: number, az: number, gx: number, gy: number, gz: number): void;
    /**
     * #124 Phase 4: enable/disable the browser-side JIT fast-path. When
     * on, `step_with_esp32_aids` short-circuits any pre-fetch step
     * whose PC matches the JIT'd hot block (`0x400829cc`) into a wasm
     * call constructed via `js_sys::WebAssembly`. Off by default —
     * callers opt in from JS once they've benchmarked.
     */
    set_jit_enabled(enabled: boolean): void;
    /**
     * Set the simulated thermocouple and internal temperatures on a MAX31855 device.
     */
    set_max31855_temperature(device_id: string, tc_c: number, internal_c: number): void;
    /**
     * Set the simulated temperature on an NTC thermistor attached to an ADC channel.
     *
     * All Steinhart-Hart math lives in Rust core (NtcThermistor::divider_output_mv).
     * This function only stores the new temperature, recomputes divider_mv → ADC count
     * via core, and injects the result into the ADC peripheral's channel.
     *
     * `device_id` must match a `board_io` binding with `device_type: "ntc-thermistor"`.
     */
    set_ntc_temperature(device_id: string, temperature_c: number): void;
    /**
     * Toggle a single 74HC165 input channel (0..=7) high or low.
     */
    set_sn74hc165_channel(channel: number, high: boolean): void;
    /**
     * Set all 8 digital inputs of the 74HC165 shift register at once
     * (bit `i` = channel `i`). Returns an error if no shifter is wired.
     */
    set_sn74hc165_inputs(value: number): void;
    step(cycles: number): void;
    /**
     * Execute up to max_cycles steps, returning the number actually executed.
     */
    step_batch(max_cycles: number): number;
    step_single(): void;
    /**
     * Step `cycles` cycles with the ESP32-classic IPI bridge active. Each
     * cycle samples the DPORT FROM_CPU intmatrix mapping and trigger
     * registers, raises the corresponding INTERRUPT bit, and clears the
     * trigger so the next write re-edges. The dual-core handshake bytes
     * are re-applied every 10k cycles (matching the e2e test cadence).
     * Falls back to plain `step` if `install_esp32_arduino_quirks` hasn't
     * been called yet.
     */
    step_with_esp32_aids(cycles: number): void;
    /**
     * Capture the current machine state as a binary `MachineRuntimeSnapshot`
     * (LWRS-framed bincode blob). Mirror of `apply_runtime_snapshot` —
     * returned bytes can be fed back to `apply_runtime_snapshot` on a fresh
     * `WasmSimulator` with the same firmware + bus topology.
     */
    take_runtime_snapshot(): Uint8Array;
}

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_wasmsimulator_free: (a: number, b: number) => void;
    readonly wasmsimulator_air_trace_snapshot: (a: number) => any;
    readonly wasmsimulator_apply_agentdeck_quirks: (a: number) => [number, number];
    readonly wasmsimulator_apply_runtime_snapshot: (a: number, b: number, c: number) => [number, number];
    readonly wasmsimulator_bench_jit: (a: number, b: number) => [number, number, number];
    readonly wasmsimulator_drain_uart_output: (a: number) => [number, number];
    readonly wasmsimulator_feed_uart_input: (a: number, b: number, c: number) => void;
    readonly wasmsimulator_gdb_process_packet: (a: number, b: number, c: number) => [number, number];
    readonly wasmsimulator_get_adc_device_states: (a: number) => any;
    readonly wasmsimulator_get_board_io_analog_states: (a: number) => any;
    readonly wasmsimulator_get_board_io_config: (a: number) => any;
    readonly wasmsimulator_get_board_io_states: (a: number) => any;
    readonly wasmsimulator_get_disassembly: (a: number) => [number, number];
    readonly wasmsimulator_get_i2c_sensor_states: (a: number) => any;
    readonly wasmsimulator_get_ili9341_framebuffer: (a: number, b: number, c: number) => [number, number, number, number];
    readonly wasmsimulator_get_iolink_master_state: (a: number) => any;
    readonly wasmsimulator_get_led_state: (a: number) => number;
    readonly wasmsimulator_get_pc: (a: number) => number;
    readonly wasmsimulator_get_pcd8544_framebuffer: (a: number, b: number, c: number) => [number, number, number, number];
    readonly wasmsimulator_get_peripheral_list: (a: number) => any;
    readonly wasmsimulator_get_peripheral_snapshot: (a: number, b: number, c: number) => any;
    readonly wasmsimulator_get_register: (a: number, b: number) => number;
    readonly wasmsimulator_get_register_names: (a: number) => any;
    readonly wasmsimulator_get_sn74hc165_inputs: (a: number) => number;
    readonly wasmsimulator_get_spi_device_states: (a: number) => any;
    readonly wasmsimulator_get_ssd1306_framebuffer: (a: number, b: number, c: number) => [number, number, number, number];
    readonly wasmsimulator_get_ssd1680_framebuffer: (a: number, b: number, c: number) => [number, number, number, number];
    readonly wasmsimulator_get_ssd1680_refresh_generation: (a: number, b: number, c: number) => [number, number, number];
    readonly wasmsimulator_get_uart_device_states: (a: number) => any;
    readonly wasmsimulator_get_uc8151d_framebuffer: (a: number, b: number, c: number) => [number, number, number, number];
    readonly wasmsimulator_get_uc8151d_refresh_generation: (a: number, b: number, c: number) => [number, number, number];
    readonly wasmsimulator_install_arduino_esp32_quirks: (a: number, b: number, c: number) => [number, number];
    readonly wasmsimulator_install_esp32_arduino_quirks: (a: number) => [number, number];
    readonly wasmsimulator_iolink_trace_clear: (a: number) => void;
    readonly wasmsimulator_iolink_trace_snapshot: (a: number) => any;
    readonly wasmsimulator_jit_hits: (a: number) => bigint;
    readonly wasmsimulator_jit_refusals: (a: number) => bigint;
    readonly wasmsimulator_keep_alive_esp32_dual_core: (a: number) => void;
    readonly wasmsimulator_new: (a: number, b: number) => [number, number, number];
    readonly wasmsimulator_new_from_config: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number, number];
    readonly wasmsimulator_read_memory: (a: number, b: number, c: number) => [number, number];
    readonly wasmsimulator_set_adc_value: (a: number, b: number, c: number, d: number) => [number, number];
    readonly wasmsimulator_set_board_io_input: (a: number, b: number, c: number, d: number) => [number, number];
    readonly wasmsimulator_set_gps_fix: (a: number, b: number, c: number, d: number) => [number, number];
    readonly wasmsimulator_set_gps_position: (a: number, b: number, c: number, d: number, e: number) => [number, number];
    readonly wasmsimulator_set_hcsr04_distance: (a: number, b: number, c: number, d: number) => [number, number];
    readonly wasmsimulator_set_i2c_sensor_sample: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number];
    readonly wasmsimulator_set_i2c_sensor_sample_6dof: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number) => [number, number];
    readonly wasmsimulator_set_jit_enabled: (a: number, b: number) => void;
    readonly wasmsimulator_set_max31855_temperature: (a: number, b: number, c: number, d: number, e: number) => [number, number];
    readonly wasmsimulator_set_ntc_temperature: (a: number, b: number, c: number, d: number) => [number, number];
    readonly wasmsimulator_set_sn74hc165_channel: (a: number, b: number, c: number) => [number, number];
    readonly wasmsimulator_set_sn74hc165_inputs: (a: number, b: number) => [number, number];
    readonly wasmsimulator_step: (a: number, b: number) => [number, number];
    readonly wasmsimulator_step_batch: (a: number, b: number) => [number, number, number];
    readonly wasmsimulator_step_single: (a: number) => [number, number];
    readonly wasmsimulator_step_with_esp32_aids: (a: number, b: number) => [number, number];
    readonly wasmsimulator_take_runtime_snapshot: (a: number) => [number, number, number, number];
    readonly wasm_bindgen__convert__closures_____invoke__hd749741175e04d09: (a: number, b: number, c: number) => number;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_destroy_closure: (a: number, b: number) => void;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;

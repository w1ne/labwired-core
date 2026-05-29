/* @ts-self-types="./labwired_wasm.d.ts" */

export class WasmSimulator {
    static __wrap(ptr) {
        const obj = Object.create(WasmSimulator.prototype);
        obj.__wbg_ptr = ptr;
        WasmSimulatorFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        WasmSimulatorFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_wasmsimulator_free(ptr, 0);
    }
    apply_agentdeck_quirks() {
        const ret = wasm.wasmsimulator_apply_agentdeck_quirks(this.__wbg_ptr);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
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
     * @param {Uint8Array} bytes
     */
    apply_runtime_snapshot(bytes) {
        const ptr0 = passArray8ToWasm0(bytes, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmsimulator_apply_runtime_snapshot(this.__wbg_ptr, ptr0, len0);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Bench runner: execute `cycles` `step_with_esp32_aids` iterations
     * and return elapsed milliseconds (measured via
     * `performance.now()`). The caller drives this twice — once with
     * `set_jit_enabled(false)`, once with `set_jit_enabled(true)` —
     * and compares the two numbers to quantify JIT speedup.
     *
     * Returns a `Result<f64, JsValue>`: the `Err` path bubbles step
     * errors so the bench harness can show a useful message.
     * @param {number} cycles
     * @returns {number}
     */
    bench_jit(cycles) {
        const ret = wasm.wasmsimulator_bench_jit(this.__wbg_ptr, cycles);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        return ret[0];
    }
    /**
     * Drain UART TX output bytes accumulated since the last call.
     * @returns {Uint8Array}
     */
    drain_uart_output() {
        const ret = wasm.wasmsimulator_drain_uart_output(this.__wbg_ptr);
        var v1 = getArrayU8FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
        return v1;
    }
    /**
     * Push bytes into all UART RX buffers (bidirectional serial input).
     * @param {Uint8Array} data
     */
    feed_uart_input(data) {
        const ptr0 = passArray8ToWasm0(data, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        wasm.wasmsimulator_feed_uart_input(this.__wbg_ptr, ptr0, len0);
    }
    /**
     * Browser-side GDB stub entry point.
     *
     * Disabled in this build: the GdbStub `Target` impl in `labwired-gdbstub`
     * is concrete on `LabwiredTarget<CortexM>` / `LabwiredTarget<RiscV>`,
     * but `WasmSimulator` now holds `Machine<Box<dyn Cpu>>` so the bound
     * isn't satisfied. The playground has no JS caller for this method,
     * so we return an empty packet rather than refactor `labwired-gdbstub`
     * to be dyn-aware. Track via the v0.6 plan.
     * @param {Uint8Array} _packet
     * @returns {Uint8Array}
     */
    gdb_process_packet(_packet) {
        const ptr0 = passArray8ToWasm0(_packet, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmsimulator_gdb_process_packet(this.__wbg_ptr, ptr0, len0);
        var v2 = getArrayU8FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
        return v2;
    }
    /**
     * Read back the current state of all NTC thermistor devices declared in `board_io`.
     *
     * Returns `[{ id, kind: "ntc-thermistor", temperature_c, divider_mv, adc_count }]`.
     * All conversion math (Steinhart-Hart, mV→count) is performed here by calling into
     * core types — no conversion logic in this WASM bridge body.
     * @returns {any}
     */
    get_adc_device_states() {
        const ret = wasm.wasmsimulator_get_adc_device_states(this.__wbg_ptr);
        return ret;
    }
    /**
     * Returns analog state for ADC and PWM board_io bindings.
     * @returns {any}
     */
    get_board_io_analog_states() {
        const ret = wasm.wasmsimulator_get_board_io_analog_states(this.__wbg_ptr);
        return ret;
    }
    /**
     * Returns the board_io configuration as a JSON array.
     * Each entry: { id, kind, peripheral, pin, signal, active_high }
     * @returns {any}
     */
    get_board_io_config() {
        const ret = wasm.wasmsimulator_get_board_io_config(this.__wbg_ptr);
        return ret;
    }
    /**
     * Returns the current state of all board_io bindings as a JSON array.
     * Each entry: { id, active }
     * Uses peripheral snapshot() to read ODR regardless of register layout.
     * @returns {any}
     */
    get_board_io_states() {
        const ret = wasm.wasmsimulator_get_board_io_states(this.__wbg_ptr);
        return ret;
    }
    /**
     * @returns {string}
     */
    get_disassembly() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.wasmsimulator_get_disassembly(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Read back the current sensor data from each I2C sensor declared in `board_io`.
     * Returns `[{ id, kind: "adxl345", x, y, z }, ...]` or `[{ id, kind: "mpu6050", ax, ay, az, gx, gy, gz }, ...]`
     * or `[{ id, kind: "bme280", temperature_c, humidity_pct, pressure_hpa }, ...]`.
     * @returns {any}
     */
    get_i2c_sensor_states() {
        const ret = wasm.wasmsimulator_get_i2c_sensor_states(this.__wbg_ptr);
        return ret;
    }
    /**
     * Return the ILI9341 RGB565 framebuffer for the device identified by `device_id`.
     *
     * `device_id` must match a `board_io` binding with `device_type: "ili9341"`.
     * Returns a 153,600-byte `Uint8Array` (240×320 pixels × 2 bytes, row-major, big-endian RGB565).
     * Returns a JS error if the device is not found.
     * @param {string} device_id
     * @returns {Uint8Array}
     */
    get_ili9341_framebuffer(device_id) {
        const ptr0 = passStringToWasm0(device_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmsimulator_get_ili9341_framebuffer(this.__wbg_ptr, ptr0, len0);
        if (ret[3]) {
            throw takeFromExternrefTable0(ret[2]);
        }
        var v2 = getArrayU8FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
        return v2;
    }
    /**
     * Legacy LED state query (hardcoded GPIOB pin 5 for backward compat).
     * @returns {boolean}
     */
    get_led_state() {
        const ret = wasm.wasmsimulator_get_led_state(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * @returns {number}
     */
    get_pc() {
        const ret = wasm.wasmsimulator_get_pc(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * List all peripherals: [{ name, base_address }]
     * @returns {any}
     */
    get_peripheral_list() {
        const ret = wasm.wasmsimulator_get_peripheral_list(this.__wbg_ptr);
        return ret;
    }
    /**
     * Get a peripheral's full state snapshot as JSON.
     * @param {string} name
     * @returns {any}
     */
    get_peripheral_snapshot(name) {
        const ptr0 = passStringToWasm0(name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmsimulator_get_peripheral_snapshot(this.__wbg_ptr, ptr0, len0);
        return ret;
    }
    /**
     * @param {number} id
     * @returns {number}
     */
    get_register(id) {
        const ret = wasm.wasmsimulator_get_register(this.__wbg_ptr, id);
        return ret >>> 0;
    }
    /**
     * @returns {any}
     */
    get_register_names() {
        const ret = wasm.wasmsimulator_get_register_names(this.__wbg_ptr);
        return ret;
    }
    /**
     * Read back the current state of each SPI sensor declared in `board_io`.
     * Returns `[{ id, kind: "max31855", tc_c, internal_c }, ...]`.
     * @returns {any}
     */
    get_spi_device_states() {
        const ret = wasm.wasmsimulator_get_spi_device_states(this.__wbg_ptr);
        return ret;
    }
    /**
     * Return the SSD1306 GDDRAM framebuffer for the device identified by `device_id`.
     *
     * `device_id` must match a `board_io` binding with `device_type: "oled-ssd1306"`.
     * Returns a 1024-byte `Uint8Array` (128 columns × 8 pages, page-major).
     * Returns a JS error if the device is not found.
     * @param {string} device_id
     * @returns {Uint8Array}
     */
    get_ssd1306_framebuffer(device_id) {
        const ptr0 = passStringToWasm0(device_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmsimulator_get_ssd1306_framebuffer(this.__wbg_ptr, ptr0, len0);
        if (ret[3]) {
            throw takeFromExternrefTable0(ret[2]);
        }
        var v2 = getArrayU8FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
        return v2;
    }
    /**
     * Return the SSD1680 tri-color e-paper framebuffer for the device identified by `device_id`.
     *
     * `device_id` must match a `board_io` binding with `device_type: "ssd1680_tricolor_290"`.
     * Returns a 9472-byte `Uint8Array`: first 4736 bytes are the black plane
     * (1 = white / 0 = black), next 4736 bytes are the red plane on the wire
     * (1 = no-red / 0 = red — see GxEPD2 inversion in writeImage). Row-major,
     * 128 pixels wide / 296 tall native, MSB-first packing within each byte.
     * Returns a JS error if the device is not found.
     * @param {string} device_id
     * @returns {Uint8Array}
     */
    get_ssd1680_framebuffer(device_id) {
        const ptr0 = passStringToWasm0(device_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmsimulator_get_ssd1680_framebuffer(this.__wbg_ptr, ptr0, len0);
        if (ret[3]) {
            throw takeFromExternrefTable0(ret[2]);
        }
        var v2 = getArrayU8FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
        return v2;
    }
    /**
     * Cheap accessor returning just the SSD1680 refresh-generation counter.
     * UI uses this to decide whether to re-fetch the (larger) framebuffer.
     * @param {string} device_id
     * @returns {number}
     */
    get_ssd1680_refresh_generation(device_id) {
        const ptr0 = passStringToWasm0(device_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmsimulator_get_ssd1680_refresh_generation(this.__wbg_ptr, ptr0, len0);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        return ret[0] >>> 0;
    }
    /**
     * Read back the current state of all NEO-6M GPS devices declared in `board_io`.
     * Returns `[{ id, kind: "neo6m-gps", lat, lon, has_fix }]`.
     * @returns {any}
     */
    get_uart_device_states() {
        const ret = wasm.wasmsimulator_get_uart_device_states(this.__wbg_ptr);
        return ret;
    }
    /**
     * Same shape as [`get_ssd1680_framebuffer`] but for the UC8151D-family
     * tri-color panel attached by [`install_arduino_esp32_quirks`]. The
     * board_io binding type may say `ssd1680_tricolor_290` (since system
     * YAMLs were authored before the UC8151D split); we ignore that and
     * just find a `Uc8151dTricolor290` on the named SPI peripheral.
     * @param {string} device_id
     * @returns {Uint8Array}
     */
    get_uc8151d_framebuffer(device_id) {
        const ptr0 = passStringToWasm0(device_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmsimulator_get_uc8151d_framebuffer(this.__wbg_ptr, ptr0, len0);
        if (ret[3]) {
            throw takeFromExternrefTable0(ret[2]);
        }
        var v2 = getArrayU8FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
        return v2;
    }
    /**
     * Cheap accessor returning just the UC8151D refresh-generation counter.
     * @param {string} device_id
     * @returns {number}
     */
    get_uc8151d_refresh_generation(device_id) {
        const ptr0 = passStringToWasm0(device_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmsimulator_get_uc8151d_refresh_generation(this.__wbg_ptr, ptr0, len0);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        return ret[0] >>> 0;
    }
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
     * @param {Uint8Array} elf_bytes
     */
    install_arduino_esp32_quirks(elf_bytes) {
        const ptr0 = passArray8ToWasm0(elf_bytes, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmsimulator_install_arduino_esp32_quirks(this.__wbg_ptr, ptr0, len0);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    install_esp32_arduino_quirks() {
        const ret = wasm.wasmsimulator_install_esp32_arduino_quirks(this.__wbg_ptr);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Total number of times the browser JIT has dispatched a
     * compiled block. Useful for confirming the JIT path actually
     * fired during a benchmark.
     * @returns {bigint}
     */
    jit_hits() {
        const ret = wasm.wasmsimulator_jit_hits(this.__wbg_ptr);
        return BigInt.asUintN(64, ret);
    }
    /**
     * Total number of JIT refusals (host bus errors, JS-side
     * dispatch failures). Surfaced for the bench harness so it can
     * distinguish "JIT was tried and rejected" from "JIT was never
     * hit because PC never reached the block".
     * @returns {bigint}
     */
    jit_refusals() {
        const ret = wasm.wasmsimulator_jit_refusals(this.__wbg_ptr);
        return BigInt.asUintN(64, ret);
    }
    /**
     * Re-write the dual-core handshake bytes. Call every ~10k steps from JS
     * — firmware boot code revisits these and we need them to stay 1.
     */
    keep_alive_esp32_dual_core() {
        wasm.wasmsimulator_keep_alive_esp32_dual_core(this.__wbg_ptr);
    }
    /**
     * Legacy constructor: hardcoded STM32F107 Cortex-M3 with 128KB flash + 20KB RAM.
     * Kept for backward compatibility with the existing landing page sandbox.
     * @param {Uint8Array} firmware
     */
    constructor(firmware) {
        const ptr0 = passArray8ToWasm0(firmware, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmsimulator_new(ptr0, len0);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        this.__wbg_ptr = ret[0];
        WasmSimulatorFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
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
     * @param {string} system_yaml
     * @param {string} chip_yaml
     * @param {Uint8Array} firmware
     * @returns {WasmSimulator}
     */
    static new_from_config(system_yaml, chip_yaml, firmware) {
        const ptr0 = passStringToWasm0(system_yaml, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ptr1 = passStringToWasm0(chip_yaml, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len1 = WASM_VECTOR_LEN;
        const ptr2 = passArray8ToWasm0(firmware, wasm.__wbindgen_malloc);
        const len2 = WASM_VECTOR_LEN;
        const ret = wasm.wasmsimulator_new_from_config(ptr0, len0, ptr1, len1, ptr2, len2);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        return WasmSimulator.__wrap(ret[0]);
    }
    /**
     * @param {number} addr
     * @param {number} len
     * @returns {Uint8Array}
     */
    read_memory(addr, len) {
        const ret = wasm.wasmsimulator_read_memory(this.__wbg_ptr, addr, len);
        var v1 = getArrayU8FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
        return v1;
    }
    /**
     * Inject an ADC value into a named ADC peripheral's data register.
     * @param {string} peripheral_name
     * @param {number} value
     */
    set_adc_value(peripheral_name, value) {
        const ptr0 = passStringToWasm0(peripheral_name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmsimulator_set_adc_value(this.__wbg_ptr, ptr0, len0, value);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Set an input board_io binding (e.g. button press).
     * Writes to the GPIO IDR register bit for the specified binding.
     * @param {string} id
     * @param {boolean} active
     */
    set_board_io_input(id, active) {
        const ptr0 = passStringToWasm0(id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmsimulator_set_board_io_input(this.__wbg_ptr, ptr0, len0, active);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Enable or disable the GPS fix on a NEO-6M module.
     * @param {string} device_id
     * @param {boolean} active
     */
    set_gps_fix(device_id, active) {
        const ptr0 = passStringToWasm0(device_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmsimulator_set_gps_fix(this.__wbg_ptr, ptr0, len0, active);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Set the simulated position on a NEO-6M GPS module attached to a UART peripheral.
     *
     * `device_id` must match a `board_io` binding with `device_type: "neo6m-gps"`.
     * @param {string} device_id
     * @param {number} lat
     * @param {number} lon
     */
    set_gps_position(device_id, lat, lon) {
        const ptr0 = passStringToWasm0(device_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmsimulator_set_gps_position(this.__wbg_ptr, ptr0, len0, lat, lon);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Set the simulated X/Y/Z sample on an ADXL345 attached to an I2C peripheral.
     * Looks up the binding in `board_io` by id; the binding must have
     * `device_type: "adxl345"`.
     * @param {string} device_id
     * @param {number} x
     * @param {number} y
     * @param {number} z
     */
    set_i2c_sensor_sample(device_id, x, y, z) {
        const ptr0 = passStringToWasm0(device_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmsimulator_set_i2c_sensor_sample(this.__wbg_ptr, ptr0, len0, x, y, z);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Set the simulated 6-DoF sample on an MPU6050 attached to an I2C peripheral.
     * @param {string} device_id
     * @param {number} ax
     * @param {number} ay
     * @param {number} az
     * @param {number} gx
     * @param {number} gy
     * @param {number} gz
     */
    set_i2c_sensor_sample_6dof(device_id, ax, ay, az, gx, gy, gz) {
        const ptr0 = passStringToWasm0(device_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmsimulator_set_i2c_sensor_sample_6dof(this.__wbg_ptr, ptr0, len0, ax, ay, az, gx, gy, gz);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * #124 Phase 4: enable/disable the browser-side JIT fast-path. When
     * on, `step_with_esp32_aids` short-circuits any pre-fetch step
     * whose PC matches the JIT'd hot block (`0x400829cc`) into a wasm
     * call constructed via `js_sys::WebAssembly`. Off by default —
     * callers opt in from JS once they've benchmarked.
     * @param {boolean} enabled
     */
    set_jit_enabled(enabled) {
        wasm.wasmsimulator_set_jit_enabled(this.__wbg_ptr, enabled);
    }
    /**
     * Set the simulated thermocouple and internal temperatures on a MAX31855 device.
     * @param {string} device_id
     * @param {number} tc_c
     * @param {number} internal_c
     */
    set_max31855_temperature(device_id, tc_c, internal_c) {
        const ptr0 = passStringToWasm0(device_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmsimulator_set_max31855_temperature(this.__wbg_ptr, ptr0, len0, tc_c, internal_c);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Set the simulated temperature on an NTC thermistor attached to an ADC channel.
     *
     * All Steinhart-Hart math lives in Rust core (NtcThermistor::divider_output_mv).
     * This function only stores the new temperature, recomputes divider_mv → ADC count
     * via core, and injects the result into the ADC peripheral's channel.
     *
     * `device_id` must match a `board_io` binding with `device_type: "ntc-thermistor"`.
     * @param {string} device_id
     * @param {number} temperature_c
     */
    set_ntc_temperature(device_id, temperature_c) {
        const ptr0 = passStringToWasm0(device_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmsimulator_set_ntc_temperature(this.__wbg_ptr, ptr0, len0, temperature_c);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * @param {number} cycles
     */
    step(cycles) {
        const ret = wasm.wasmsimulator_step(this.__wbg_ptr, cycles);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Execute up to max_cycles steps, returning the number actually executed.
     * @param {number} max_cycles
     * @returns {number}
     */
    step_batch(max_cycles) {
        const ret = wasm.wasmsimulator_step_batch(this.__wbg_ptr, max_cycles);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        return ret[0] >>> 0;
    }
    step_single() {
        const ret = wasm.wasmsimulator_step_single(this.__wbg_ptr);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Step `cycles` cycles with the ESP32-classic IPI bridge active. Each
     * cycle samples the DPORT FROM_CPU intmatrix mapping and trigger
     * registers, raises the corresponding INTERRUPT bit, and clears the
     * trigger so the next write re-edges. The dual-core handshake bytes
     * are re-applied every 10k cycles (matching the e2e test cadence).
     * Falls back to plain `step` if `install_esp32_arduino_quirks` hasn't
     * been called yet.
     * @param {number} cycles
     */
    step_with_esp32_aids(cycles) {
        const ret = wasm.wasmsimulator_step_with_esp32_aids(this.__wbg_ptr, cycles);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Capture the current machine state as a binary `MachineRuntimeSnapshot`
     * (LWRS-framed bincode blob). Mirror of `apply_runtime_snapshot` —
     * returned bytes can be fed back to `apply_runtime_snapshot` on a fresh
     * `WasmSimulator` with the same firmware + bus topology.
     * @returns {Uint8Array}
     */
    take_runtime_snapshot() {
        const ret = wasm.wasmsimulator_take_runtime_snapshot(this.__wbg_ptr);
        if (ret[3]) {
            throw takeFromExternrefTable0(ret[2]);
        }
        var v1 = getArrayU8FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
        return v1;
    }
}
if (Symbol.dispose) WasmSimulator.prototype[Symbol.dispose] = WasmSimulator.prototype.free;
function __wbg_get_imports() {
    const import0 = {
        __proto__: null,
        __wbg_Error_bce6d499ff0a4aff: function(arg0, arg1) {
            const ret = Error(getStringFromWasm0(arg0, arg1));
            return ret;
        },
        __wbg___wbindgen_debug_string_edece8177ad01481: function(arg0, arg1) {
            const ret = debugString(arg1);
            const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbg___wbindgen_is_function_5cd60d5cf78b4eef: function(arg0) {
            const ret = typeof(arg0) === 'function';
            return ret;
        },
        __wbg___wbindgen_is_string_dde0fd9020db4434: function(arg0) {
            const ret = typeof(arg0) === 'string';
            return ret;
        },
        __wbg___wbindgen_number_get_f73a1244370fcc2c: function(arg0, arg1) {
            const obj = arg1;
            const ret = typeof(obj) === 'number' ? obj : undefined;
            getDataViewMemory0().setFloat64(arg0 + 8 * 1, isLikeNone(ret) ? 0 : ret, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, !isLikeNone(ret), true);
        },
        __wbg___wbindgen_throw_9c31b086c2b26051: function(arg0, arg1) {
            throw new Error(getStringFromWasm0(arg0, arg1));
        },
        __wbg__wbg_cb_unref_3fa391f3fcdb55f8: function(arg0) {
            arg0._wbg_cb_unref();
        },
        __wbg_call_084ee3e860ee9f92: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4) {
            const ret = arg0.call(arg1, arg2, arg3, arg4);
            return ret;
        }, arguments); },
        __wbg_exports_fcb6c7dbab2808fc: function(arg0) {
            const ret = arg0.exports;
            return ret;
        },
        __wbg_get_98fdf51d029a75eb: function(arg0, arg1) {
            const ret = arg0[arg1 >>> 0];
            return ret;
        },
        __wbg_get_dcf82ab8aad1a593: function() { return handleError(function (arg0, arg1) {
            const ret = Reflect.get(arg0, arg1);
            return ret;
        }, arguments); },
        __wbg_isArray_94898ed3aad6947b: function(arg0) {
            const ret = Array.isArray(arg0);
            return ret;
        },
        __wbg_length_2591a0f4f659a55c: function(arg0) {
            const ret = arg0.length;
            return ret;
        },
        __wbg_length_56fcd3e2b7e0299d: function(arg0) {
            const ret = arg0.length;
            return ret;
        },
        __wbg_new_02d162bc6cf02f60: function() {
            const ret = new Object();
            return ret;
        },
        __wbg_new_070df68d66325372: function() {
            const ret = new Map();
            return ret;
        },
        __wbg_new_1f0e50fc5628cc27: function() { return handleError(function (arg0) {
            const ret = new WebAssembly.Module(arg0);
            return ret;
        }, arguments); },
        __wbg_new_22cc98ecc9876bce: function() { return handleError(function (arg0, arg1) {
            const ret = new WebAssembly.Instance(arg0, arg1);
            return ret;
        }, arguments); },
        __wbg_new_310879b66b6e95e1: function() {
            const ret = new Array();
            return ret;
        },
        __wbg_new_with_length_99887c91eae4abab: function(arg0) {
            const ret = new Uint8Array(arg0 >>> 0);
            return ret;
        },
        __wbg_now_d40f50e29aa45633: function() {
            const ret = performance.now();
            return ret;
        },
        __wbg_set_24d0fa9e104112f9: function(arg0, arg1, arg2) {
            arg0.set(getArrayU8FromWasm0(arg1, arg2));
        },
        __wbg_set_6be42768c690e380: function(arg0, arg1, arg2) {
            arg0[arg1] = arg2;
        },
        __wbg_set_78ea6a19f4818587: function(arg0, arg1, arg2) {
            arg0[arg1 >>> 0] = arg2;
        },
        __wbg_set_a0e911be3da02782: function() { return handleError(function (arg0, arg1, arg2) {
            const ret = Reflect.set(arg0, arg1, arg2);
            return ret;
        }, arguments); },
        __wbg_set_facb7a5914e0fa39: function(arg0, arg1, arg2) {
            const ret = arg0.set(arg1, arg2);
            return ret;
        },
        __wbg_warn_6aa887ee9eac6cc8: function(arg0, arg1) {
            console.warn(getStringFromWasm0(arg0, arg1));
        },
        __wbindgen_cast_0000000000000001: function(arg0, arg1) {
            // Cast intrinsic for `Closure(Closure { owned: true, function: Function { arguments: [I32], shim_idx: 1764, ret: I32, inner_ret: Some(I32) }, mutable: true }) -> Externref`.
            const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__h90cbb89e119fd004);
            return ret;
        },
        __wbindgen_cast_0000000000000002: function(arg0) {
            // Cast intrinsic for `F64 -> Externref`.
            const ret = arg0;
            return ret;
        },
        __wbindgen_cast_0000000000000003: function(arg0) {
            // Cast intrinsic for `I64 -> Externref`.
            const ret = arg0;
            return ret;
        },
        __wbindgen_cast_0000000000000004: function(arg0, arg1) {
            // Cast intrinsic for `Ref(String) -> Externref`.
            const ret = getStringFromWasm0(arg0, arg1);
            return ret;
        },
        __wbindgen_cast_0000000000000005: function(arg0) {
            // Cast intrinsic for `U64 -> Externref`.
            const ret = BigInt.asUintN(64, arg0);
            return ret;
        },
        __wbindgen_init_externref_table: function() {
            const table = wasm.__wbindgen_externrefs;
            const offset = table.grow(4);
            table.set(0, undefined);
            table.set(offset + 0, undefined);
            table.set(offset + 1, null);
            table.set(offset + 2, true);
            table.set(offset + 3, false);
        },
    };
    return {
        __proto__: null,
        "./labwired_wasm_bg.js": import0,
    };
}

function wasm_bindgen__convert__closures_____invoke__h90cbb89e119fd004(arg0, arg1, arg2) {
    const ret = wasm.wasm_bindgen__convert__closures_____invoke__h90cbb89e119fd004(arg0, arg1, arg2);
    return ret;
}

const WasmSimulatorFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_wasmsimulator_free(ptr, 1));

function addToExternrefTable0(obj) {
    const idx = wasm.__externref_table_alloc();
    wasm.__wbindgen_externrefs.set(idx, obj);
    return idx;
}

const CLOSURE_DTORS = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(state => wasm.__wbindgen_destroy_closure(state.a, state.b));

function debugString(val) {
    // primitive types
    const type = typeof val;
    if (type == 'number' || type == 'boolean' || val == null) {
        return  `${val}`;
    }
    if (type == 'string') {
        return `"${val}"`;
    }
    if (type == 'symbol') {
        const description = val.description;
        if (description == null) {
            return 'Symbol';
        } else {
            return `Symbol(${description})`;
        }
    }
    if (type == 'function') {
        const name = val.name;
        if (typeof name == 'string' && name.length > 0) {
            return `Function(${name})`;
        } else {
            return 'Function';
        }
    }
    // objects
    if (Array.isArray(val)) {
        const length = val.length;
        let debug = '[';
        if (length > 0) {
            debug += debugString(val[0]);
        }
        for(let i = 1; i < length; i++) {
            debug += ', ' + debugString(val[i]);
        }
        debug += ']';
        return debug;
    }
    // Test for built-in
    const builtInMatches = /\[object ([^\]]+)\]/.exec(toString.call(val));
    let className;
    if (builtInMatches && builtInMatches.length > 1) {
        className = builtInMatches[1];
    } else {
        // Failed to match the standard '[object ClassName]'
        return toString.call(val);
    }
    if (className == 'Object') {
        // we're a user defined class or Object
        // JSON.stringify avoids problems with cycles, and is generally much
        // easier than looping through ownProperties of `val`.
        try {
            return 'Object(' + JSON.stringify(val) + ')';
        } catch (_) {
            return 'Object';
        }
    }
    // errors
    if (val instanceof Error) {
        return `${val.name}: ${val.message}\n${val.stack}`;
    }
    // TODO we could test for more things here, like `Set`s and `Map`s.
    return className;
}

function getArrayU8FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint8ArrayMemory0().subarray(ptr / 1, ptr / 1 + len);
}

let cachedDataViewMemory0 = null;
function getDataViewMemory0() {
    if (cachedDataViewMemory0 === null || cachedDataViewMemory0.buffer.detached === true || (cachedDataViewMemory0.buffer.detached === undefined && cachedDataViewMemory0.buffer !== wasm.memory.buffer)) {
        cachedDataViewMemory0 = new DataView(wasm.memory.buffer);
    }
    return cachedDataViewMemory0;
}

function getStringFromWasm0(ptr, len) {
    return decodeText(ptr >>> 0, len);
}

let cachedUint8ArrayMemory0 = null;
function getUint8ArrayMemory0() {
    if (cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0) {
        cachedUint8ArrayMemory0 = new Uint8Array(wasm.memory.buffer);
    }
    return cachedUint8ArrayMemory0;
}

function handleError(f, args) {
    try {
        return f.apply(this, args);
    } catch (e) {
        const idx = addToExternrefTable0(e);
        wasm.__wbindgen_exn_store(idx);
    }
}

function isLikeNone(x) {
    return x === undefined || x === null;
}

function makeMutClosure(arg0, arg1, f) {
    const state = { a: arg0, b: arg1, cnt: 1 };
    const real = (...args) => {

        // First up with a closure we increment the internal reference
        // count. This ensures that the Rust closure environment won't
        // be deallocated while we're invoking it.
        state.cnt++;
        const a = state.a;
        state.a = 0;
        try {
            return f(a, state.b, ...args);
        } finally {
            state.a = a;
            real._wbg_cb_unref();
        }
    };
    real._wbg_cb_unref = () => {
        if (--state.cnt === 0) {
            wasm.__wbindgen_destroy_closure(state.a, state.b);
            state.a = 0;
            CLOSURE_DTORS.unregister(state);
        }
    };
    CLOSURE_DTORS.register(real, state, state);
    return real;
}

function passArray8ToWasm0(arg, malloc) {
    const ptr = malloc(arg.length * 1, 1) >>> 0;
    getUint8ArrayMemory0().set(arg, ptr / 1);
    WASM_VECTOR_LEN = arg.length;
    return ptr;
}

function passStringToWasm0(arg, malloc, realloc) {
    if (realloc === undefined) {
        const buf = cachedTextEncoder.encode(arg);
        const ptr = malloc(buf.length, 1) >>> 0;
        getUint8ArrayMemory0().subarray(ptr, ptr + buf.length).set(buf);
        WASM_VECTOR_LEN = buf.length;
        return ptr;
    }

    let len = arg.length;
    let ptr = malloc(len, 1) >>> 0;

    const mem = getUint8ArrayMemory0();

    let offset = 0;

    for (; offset < len; offset++) {
        const code = arg.charCodeAt(offset);
        if (code > 0x7F) break;
        mem[ptr + offset] = code;
    }
    if (offset !== len) {
        if (offset !== 0) {
            arg = arg.slice(offset);
        }
        ptr = realloc(ptr, len, len = offset + arg.length * 3, 1) >>> 0;
        const view = getUint8ArrayMemory0().subarray(ptr + offset, ptr + len);
        const ret = cachedTextEncoder.encodeInto(arg, view);

        offset += ret.written;
        ptr = realloc(ptr, len, offset, 1) >>> 0;
    }

    WASM_VECTOR_LEN = offset;
    return ptr;
}

function takeFromExternrefTable0(idx) {
    const value = wasm.__wbindgen_externrefs.get(idx);
    wasm.__externref_table_dealloc(idx);
    return value;
}

let cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
cachedTextDecoder.decode();
const MAX_SAFARI_DECODE_BYTES = 2146435072;
let numBytesDecoded = 0;
function decodeText(ptr, len) {
    numBytesDecoded += len;
    if (numBytesDecoded >= MAX_SAFARI_DECODE_BYTES) {
        cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
        cachedTextDecoder.decode();
        numBytesDecoded = len;
    }
    return cachedTextDecoder.decode(getUint8ArrayMemory0().subarray(ptr, ptr + len));
}

const cachedTextEncoder = new TextEncoder();

if (!('encodeInto' in cachedTextEncoder)) {
    cachedTextEncoder.encodeInto = function (arg, view) {
        const buf = cachedTextEncoder.encode(arg);
        view.set(buf);
        return {
            read: arg.length,
            written: buf.length
        };
    };
}

let WASM_VECTOR_LEN = 0;

let wasmModule, wasmInstance, wasm;
function __wbg_finalize_init(instance, module) {
    wasmInstance = instance;
    wasm = instance.exports;
    wasmModule = module;
    cachedDataViewMemory0 = null;
    cachedUint8ArrayMemory0 = null;
    wasm.__wbindgen_start();
    return wasm;
}

async function __wbg_load(module, imports) {
    if (typeof Response === 'function' && module instanceof Response) {
        if (typeof WebAssembly.instantiateStreaming === 'function') {
            try {
                return await WebAssembly.instantiateStreaming(module, imports);
            } catch (e) {
                const validResponse = module.ok && expectedResponseType(module.type);

                if (validResponse && module.headers.get('Content-Type') !== 'application/wasm') {
                    console.warn("`WebAssembly.instantiateStreaming` failed because your server does not serve Wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n", e);

                } else { throw e; }
            }
        }

        const bytes = await module.arrayBuffer();
        return await WebAssembly.instantiate(bytes, imports);
    } else {
        const instance = await WebAssembly.instantiate(module, imports);

        if (instance instanceof WebAssembly.Instance) {
            return { instance, module };
        } else {
            return instance;
        }
    }

    function expectedResponseType(type) {
        switch (type) {
            case 'basic': case 'cors': case 'default': return true;
        }
        return false;
    }
}

function initSync(module) {
    if (wasm !== undefined) return wasm;


    if (module !== undefined) {
        if (Object.getPrototypeOf(module) === Object.prototype) {
            ({module} = module)
        } else {
            console.warn('using deprecated parameters for `initSync()`; pass a single object instead')
        }
    }

    const imports = __wbg_get_imports();
    if (!(module instanceof WebAssembly.Module)) {
        module = new WebAssembly.Module(module);
    }
    const instance = new WebAssembly.Instance(module, imports);
    return __wbg_finalize_init(instance, module);
}

async function __wbg_init(module_or_path) {
    if (wasm !== undefined) return wasm;


    if (module_or_path !== undefined) {
        if (Object.getPrototypeOf(module_or_path) === Object.prototype) {
            ({module_or_path} = module_or_path)
        } else {
            console.warn('using deprecated parameters for the initialization function; pass a single object instead')
        }
    }

    if (module_or_path === undefined) {
        module_or_path = new URL('labwired_wasm_bg.wasm', import.meta.url);
    }
    const imports = __wbg_get_imports();

    if (typeof module_or_path === 'string' || (typeof Request === 'function' && module_or_path instanceof Request) || (typeof URL === 'function' && module_or_path instanceof URL)) {
        module_or_path = fetch(module_or_path);
    }

    const { instance, module } = await __wbg_load(await module_or_path, imports);

    return __wbg_finalize_init(instance, module);
}

export { initSync, __wbg_init as default };

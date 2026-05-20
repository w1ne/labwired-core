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
    install_esp32_arduino_quirks() {
        const ret = wasm.wasmsimulator_install_esp32_arduino_quirks(this.__wbg_ptr);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
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
}
if (Symbol.dispose) WasmSimulator.prototype[Symbol.dispose] = WasmSimulator.prototype.free;
function __wbg_get_imports() {
    const import0 = {
        __proto__: null,
        __wbg_Error_bce6d499ff0a4aff: function(arg0, arg1) {
            const ret = Error(getStringFromWasm0(arg0, arg1));
            return ret;
        },
        __wbg___wbindgen_is_string_dde0fd9020db4434: function(arg0) {
            const ret = typeof(arg0) === 'string';
            return ret;
        },
        __wbg___wbindgen_throw_9c31b086c2b26051: function(arg0, arg1) {
            throw new Error(getStringFromWasm0(arg0, arg1));
        },
        __wbg_new_02d162bc6cf02f60: function() {
            const ret = new Object();
            return ret;
        },
        __wbg_new_070df68d66325372: function() {
            const ret = new Map();
            return ret;
        },
        __wbg_new_310879b66b6e95e1: function() {
            const ret = new Array();
            return ret;
        },
        __wbg_set_6be42768c690e380: function(arg0, arg1, arg2) {
            arg0[arg1] = arg2;
        },
        __wbg_set_78ea6a19f4818587: function(arg0, arg1, arg2) {
            arg0[arg1 >>> 0] = arg2;
        },
        __wbg_set_facb7a5914e0fa39: function(arg0, arg1, arg2) {
            const ret = arg0.set(arg1, arg2);
            return ret;
        },
        __wbindgen_cast_0000000000000001: function(arg0) {
            // Cast intrinsic for `F64 -> Externref`.
            const ret = arg0;
            return ret;
        },
        __wbindgen_cast_0000000000000002: function(arg0) {
            // Cast intrinsic for `I64 -> Externref`.
            const ret = arg0;
            return ret;
        },
        __wbindgen_cast_0000000000000003: function(arg0, arg1) {
            // Cast intrinsic for `Ref(String) -> Externref`.
            const ret = getStringFromWasm0(arg0, arg1);
            return ret;
        },
        __wbindgen_cast_0000000000000004: function(arg0) {
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

const WasmSimulatorFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_wasmsimulator_free(ptr, 1));

function getArrayU8FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint8ArrayMemory0().subarray(ptr / 1, ptr / 1 + len);
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

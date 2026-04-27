
| Option | Stability | Default&nbsp;value | Allowed&nbsp;values |
|--------|:---------:|:------------------:|:-------------------:|
| <p>**ESP_HAL_CONFIG_PLACE_SPI_MASTER_DRIVER_IN_RAM**</p> <p>Places the SPI master driver in RAM for better performance</p> | ⚠️ Unstable | false | 
| <p>**ESP_HAL_CONFIG_PLACE_SWITCH_TABLES_IN_RAM**</p> <p>Places switch-tables, some lookup tables and constants related to interrupt handling into RAM - resulting in better performance but slightly more RAM consumption.</p> | Stable since 1.0.0 | true | 
| <p>**ESP_HAL_CONFIG_PLACE_ANON_IN_RAM**</p> <p>Places anonymous symbols into RAM - resulting in better performance at the cost of significant more RAM consumption. Best to be combined with `place-switch-tables-in-ram`.</p> | Stable since 1.0.0 | false | 
| <p>**ESP_HAL_CONFIG_PLACE_RMT_DRIVER_IN_RAM**</p> <p>Places the RMT driver in RAM for better performance</p> | ⚠️ Unstable | false | 
| <p>**ESP_HAL_CONFIG_STACK_GUARD_OFFSET**</p> <p>The stack guard variable will be placed this many bytes from the stack's end. Needs to be a multiple of 4.</p> | Stable since 1.0.0 | 60 | 
| <p>**ESP_HAL_CONFIG_STACK_GUARD_VALUE**</p> <p>The value to be written to the stack guard variable.</p> | ⚠️ Unstable | 0xDEEDBAAD | 
| <p>**ESP_HAL_CONFIG_STACK_GUARD_MONITORING**</p> <p>Use a data watchpoint to check if the stack guard was overwritten.</p> | ⚠️ Unstable | true | 
| <p>**ESP_HAL_CONFIG_STACK_GUARD_MONITORING_WITH_DEBUGGER_CONNECTED**</p> <p>Enable the stack guard also with a debugger connected. Also applies to `write-vec-table-monitoring`.</p> | ⚠️ Unstable | true | 
| <p>**ESP_HAL_CONFIG_IMPL_CRITICAL_SECTION**</p> <p>Provide a `critical-section` implementation. Note that if disabled, you will need to provide a `critical-section` implementation which is using `restore-state-u32`.</p> | ⚠️ Unstable | true | 
| <p>**ESP_HAL_CONFIG_INSTRUCTION_CACHE_SIZE**</p> <p>Instruction cache size to be set on application startup.</p> | ⚠️ Unstable | 32KB | One of:<br/><ul style="display: inline-block; text-align: left"><li>16KB</li><li>32KB</li></ul>
| <p>**ESP_HAL_CONFIG_INSTRUCTION_CACHE_LINE_SIZE**</p> <p>Instruction cache line size to be set on application startup.</p> | ⚠️ Unstable | 32B | One of:<br/><ul style="display: inline-block; text-align: left"><li>16B</li><li>32B</li></ul>
| <p>**ESP_HAL_CONFIG_ICACHE_ASSOCIATED_WAYS**</p> <p>Instruction cache associated ways to be set on application startup.</p> | ⚠️ Unstable | 8 | One of:<br/><ul style="display: inline-block; text-align: left"><li>4</li><li>8</li></ul>
| <p>**ESP_HAL_CONFIG_DATA_CACHE_SIZE**</p> <p>Data cache size to be set on application startup.</p> | ⚠️ Unstable | 32KB | One of:<br/><ul style="display: inline-block; text-align: left"><li>16KB</li><li>32KB</li><li>64KB</li></ul>
| <p>**ESP_HAL_CONFIG_DATA_CACHE_LINE_SIZE**</p> <p>Data cache line size to be set on application startup.</p> | ⚠️ Unstable | 32B | One of:<br/><ul style="display: inline-block; text-align: left"><li>16B</li><li>32B</li><li>64B</li></ul>
| <p>**ESP_HAL_CONFIG_DCACHE_ASSOCIATED_WAYS**</p> <p>Data cache associated ways to be set on application startup.</p> | ⚠️ Unstable | 8 | One of:<br/><ul style="display: inline-block; text-align: left"><li>4</li><li>8</li></ul>
| <p>**ESP_HAL_CONFIG_MIN_CHIP_REVISION**</p> <p>The minimum chip revision required for the application to run, in format: major * 100 + minor.</p> | ⚠️ Unstable | 0 | 
| <p>**ESP_HAL_CONFIG_USE_RWDATA_LD_HOOK**</p> <p>Include 'rwdata_hook.x'</p> | ⚠️ Unstable | false | 
| <p>**ESP_HAL_CONFIG_USE_RWTEXT_LD_HOOK**</p> <p>Include 'rwtext_hook.x'</p> | ⚠️ Unstable | false | 

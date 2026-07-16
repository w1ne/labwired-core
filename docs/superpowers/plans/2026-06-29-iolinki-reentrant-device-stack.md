# iolinki Reentrant Device Stack Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Redesign the C `iolinki` device stack so one host process can run several isolated real IO-Link devices against several real C master ports, without fake protocol shims, global singleton state, or prefixed duplicate C builds.

**Architecture:** Follow the proven UDSLib shape: caller-owned protocol contexts, borrowed immutable config, app/transport/storage callbacks with `void *user`, and no hidden allocation. Replace singleton `iolink_*` entry points with `iolink_device_ctx_t` as the public device instance API. Keep `iolinki-master` separate: master role state stays in `iolink_master_port_t` / `iolink_master_controller_t`, while `iolinki` owns device role state, parameters, object dictionary, events, and Data Storage server behavior.

**Tech Stack:** C99, CMake/CTest, CMocka, existing `iolinki` C stack, `iolinki-master` C master, LabWired Rust `iolink-native` feature.

---

## Delegated Architecture Inputs

Three read-only subagents reviewed the architecture before this plan revision:

- `iolinki` audit: lower engines are already mostly context based: `iolink_dll_ctx_t`, `iolink_isdu_ctx_t`, `iolink_ds_ctx_t`, events, and frame helpers. Singleton blockers are `iolink_core.c`, `device_info.c`, `params.c`, `isdu.c` Direct Parameter page 2, and DS calls to global params.
- UDSLib audit: best pattern is caller-owned `uds_ctx_t`, borrowed `uds_config_t`, caller-provided RX/TX buffers, clock/transport/storage callbacks, `void *app_data`, optional lock hooks, and role separation. IO-Link should mirror that.
- `iolinki-master` audit: master side is already reentrant. Multi-device conformance should use `N` `iolink_master_port_t` ports under one `iolink_master_controller_t`, paired with `N` real `iolink_device_ctx_t` instances.

## Scope And Stop Gates

Implementation roots:

- Device stack: `/home/andrii/projects/labwired/core/.worktrees/iolink-simulator-conformance/third_party/iolinki`
- Master stack reference: `/home/andrii/projects/iolinki-master`
- LabWired integration: `/home/andrii/projects/labwired/core/.worktrees/iolink-simulator-conformance`

Before implementation, create a real branch/worktree for the upstream `iolinki` repo. The submodule in LabWired should be advanced only after device-stack tests pass.

Stop gates:

- Do not change `iolinki-master` public behavior to hide device-stack flaws.
- Do not put master ISDU/client state into the device context.
- Do not require consumers to mutate context internals directly. The context may be a complete public struct for static allocation, matching UDSLib, but fields are private by convention and normal consumers use accessor functions.
- Do not preserve the old singleton `iolink_*` API. There are no external users yet; tests, examples, and LabWired integration should migrate to the reentrant API directly.
- Do not rely on process-per-device, prefixed duplicate builds, or fake Rust devices as the final architecture.
- Do not claim multi-device LabWired conformance until one process runs at least two independent real `iolink_device_ctx_t` instances with separate PD, ISDU writable tags, device info, events, Direct Parameter page 2, and Data Storage state.

## Public API Shape

Create `third_party/iolinki/include/iolinki/device.h` with a caller-owned context. Like UDSLib, the struct is complete so embedded users can allocate it on the stack, in static storage, or inside a board/port object. Fields are documented as private; API users should use functions rather than direct field mutation.

```c
#ifndef IOLINK_DEVICE_H
#define IOLINK_DEVICE_H

#include "iolinki/application.h"
#include "iolinki/data_storage.h"
#include "iolinki/device_info.h"
#include "iolinki/dll.h"
#include "iolinki/iolink.h"
#include "iolinki/params.h"
#include "iolinki/phy.h"
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

typedef uint64_t (*iolink_device_time_us_fn)(void* user);
typedef void (*iolink_device_lock_fn)(void* user);
typedef void (*iolink_device_unlock_fn)(void* user);

typedef struct
{
    void* user;
    int (*init)(void* user);
    void (*set_mode)(void* user, iolink_phy_mode_t mode);
    void (*set_baudrate)(void* user, iolink_baudrate_t baudrate);
    int (*send)(void* user, const uint8_t* data, size_t len);
    int (*recv_byte)(void* user, uint8_t* byte);
    int (*detect_wakeup)(void* user);
    void (*set_cq_line)(void* user, uint8_t state);
    int (*get_voltage_mv)(void* user);
    bool (*is_short_circuit)(void* user);
} iolink_device_phy_t;

typedef struct
{
    iolink_device_phy_t phy;
    iolink_config_t stack;
    const iolink_app_callbacks_t* app_callbacks;
    const iolink_device_info_t* device_info;
    const iolink_ds_storage_api_t* ds_storage;
    iolink_device_time_us_fn time_us;
    iolink_device_lock_fn lock;
    iolink_device_unlock_fn unlock;
    void* user;
} iolink_device_config_t;

typedef struct
{
    /* Private fields. Allocate this object directly, but use API functions. */
    iolink_dll_ctx_t dll;
    iolink_config_t stack_config;
    iolink_device_info_ctx_t device_info;
    iolink_params_ctx_t params;
    const iolink_device_config_t* config;
    iolink_reset_handler_t reset_handler;
    uint8_t direct_param_page2[16];
} iolink_device_ctx_t;

size_t iolink_device_ctx_size(void);
int iolink_device_init(iolink_device_ctx_t* ctx, const iolink_device_config_t* config);
void iolink_device_process(iolink_device_ctx_t* ctx);
int iolink_device_pd_input_update(iolink_device_ctx_t* ctx,
                                  const uint8_t* data,
                                  size_t len,
                                  bool valid);
int iolink_device_pd_output_read(iolink_device_ctx_t* ctx, uint8_t* data, size_t len);
void iolink_device_set_reset_handler(iolink_device_ctx_t* ctx, iolink_reset_handler_t handler);
iolink_events_ctx_t* iolink_device_get_events_ctx(iolink_device_ctx_t* ctx);
iolink_ds_ctx_t* iolink_device_get_ds_ctx(iolink_device_ctx_t* ctx);
iolink_dll_state_t iolink_device_get_state(const iolink_device_ctx_t* ctx);
iolink_phy_mode_t iolink_device_get_phy_mode(const iolink_device_ctx_t* ctx);
iolink_baudrate_t iolink_device_get_baudrate(const iolink_device_ctx_t* ctx);
void iolink_device_get_dll_stats(const iolink_device_ctx_t* ctx, iolink_dll_stats_t* out_stats);
void iolink_device_set_timing_enforcement(iolink_device_ctx_t* ctx, bool enable);
void iolink_device_set_t_ren_limit_us(iolink_device_ctx_t* ctx, uint32_t limit_us);
iolink_m_seq_type_t iolink_device_get_m_seq_type(const iolink_device_ctx_t* ctx);
uint8_t iolink_device_get_pd_in_len(const iolink_device_ctx_t* ctx);
uint8_t iolink_device_get_pd_out_len(const iolink_device_ctx_t* ctx);
int iolink_device_set_pd_length(iolink_device_ctx_t* ctx,
                                uint8_t pd_in_len,
                                uint8_t pd_out_len);

#endif
```

Keep `iolink_device_ctx_size()` as a compile/runtime sanity helper, but normal C users can simply declare `iolink_device_ctx_t device;`.

Remove direct production use of the old `iolink_phy_api_t` as part of this migration. New multi-instance code must use `iolink_device_phy_t` so every callback receives its own per-port `user` pointer. This is the same ownership rule as UDSLib transport callbacks and prevents active-context globals in LabWired.

## File Structure

Device stack:

- Create `third_party/iolinki/include/iolinki/device.h`: reentrant public API.
- Modify `third_party/iolinki/include/iolinki/iolink.h`: remove singleton public API declarations or turn it into a transition include for `device.h`.
- Modify `third_party/iolinki/include/iolinki/phy.h`: introduce or document `iolink_device_phy_t` as the new user-aware transport model.
- Modify `third_party/iolinki/include/iolinki/device_info.h`: add `iolink_device_info_ctx_t`.
- Modify `third_party/iolinki/include/iolinki/params.h`: add `iolink_params_ctx_t`.
- Modify `third_party/iolinki/include/iolinki/isdu.h`: replace implicit global dependencies with backend pointers.
- Modify `third_party/iolinki/include/iolinki/data_storage.h`: add parameter backend callbacks to `iolink_ds_ctx_t`.
- Modify `third_party/iolinki/include/iolinki/dll.h`: add owner/dependency pointer only if needed by ISDU/DLL callbacks.
- Modify `third_party/iolinki/src/iolink_core.c`: implement `iolink_device_*` and remove global singleton state.
- Modify `third_party/iolinki/src/device_info.c`: move default identity, app tag, and access locks into context.
- Modify `third_party/iolinki/src/params.c`: move writable tags/NVM shadow into context.
- Modify `third_party/iolinki/src/isdu.c`: route params/device-info/Direct Parameter page 2 through the owning context.
- Modify `third_party/iolinki/src/data_storage.c`: route parameter image build/apply through context callbacks.
- Create `third_party/iolinki/tests/test_reentrant_device.c`: public API and isolation tests.
- Create `third_party/iolinki/tests/test_multi_device_real_master.c`: real master against several real device instances.

LabWired:

- Modify `crates/core/native/iolink_conformance.c`: use per-port link state and several `iolink_device_ctx_t` instances.
- Modify `crates/core/src/peripherals/components/iolink_master.rs`: assert multi-device native conformance.
- Modify `crates/core/build.rs`: compile new device-stack files and keep `_POSIX_C_SOURCE` define.

## Task 1: Red-Test The Public Reentrant API

**Files:**

- Create: `third_party/iolinki/include/iolinki/device.h`
- Create: `third_party/iolinki/tests/test_reentrant_device.c`
- Modify: `third_party/iolinki/tests/CMakeLists.txt`

- [ ] **Step 1: Write the failing public API test**

Create `third_party/iolinki/tests/test_reentrant_device.c`:

```c
#include <setjmp.h>
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>

#include <cmocka.h>

#include "iolinki/device.h"

static int noop_user(void* user)
{
    (void)user;
    return 0;
}

static void test_device_context_api_initializes_two_configs(void** state)
{
    (void)state;
    iolink_device_ctx_t dev_a;
    iolink_device_ctx_t dev_b;
    static iolink_device_phy_t phy = {.init = noop_user};
    iolink_device_config_t cfg_a = {
        .phy = phy,
        .stack = {
            .m_seq_type = IOLINK_M_SEQ_TYPE_1_1,
            .min_cycle_time = 10U,
            .pd_in_len = 1U,
            .pd_out_len = 0U,
            .t_pd_us = 0U,
        },
    };
    iolink_device_config_t cfg_b = {
        .phy = phy,
        .stack = {
            .m_seq_type = IOLINK_M_SEQ_TYPE_2_1,
            .min_cycle_time = 10U,
            .pd_in_len = 2U,
            .pd_out_len = 2U,
            .t_pd_us = 0U,
        },
    };

    assert_true(iolink_device_ctx_size() == sizeof(iolink_device_ctx_t));
    assert_int_equal(iolink_device_init(&dev_a, &cfg_a), 0);
    assert_int_equal(iolink_device_init(&dev_b, &cfg_b), 0);
    assert_int_equal(iolink_device_get_pd_in_len(&dev_a), 1U);
    assert_int_equal(iolink_device_get_pd_out_len(&dev_a), 0U);
    assert_int_equal(iolink_device_get_pd_in_len(&dev_b), 2U);
    assert_int_equal(iolink_device_get_pd_out_len(&dev_b), 2U);
}

int main(void)
{
    const struct CMUnitTest tests[] = {
        cmocka_unit_test(test_device_context_api_initializes_two_configs),
    };
    return cmocka_run_group_tests(tests, NULL, NULL);
}
```

- [ ] **Step 2: Register the failing test**

In `third_party/iolinki/tests/CMakeLists.txt`, add inside `if(CMOCKA_FOUND)`:

```cmake
    add_iolink_test(test_reentrant_device test_reentrant_device.c)
```

- [ ] **Step 3: Verify the red failure**

Run:

```bash
cmake -S third_party/iolinki -B /tmp/iolinki-reentrant-build -DBUILD_TESTING=ON
cmake --build /tmp/iolinki-reentrant-build --target test_reentrant_device
```

Expected: build fails on missing `iolinki/device.h` or undefined `iolink_device_ctx_t`.

- [ ] **Step 4: Add header skeleton only**

Create `third_party/iolinki/include/iolinki/device.h` with the API from the Public API Shape section.

- [ ] **Step 5: Verify the next failure**

Run:

```bash
cmake --build /tmp/iolinki-reentrant-build --target test_reentrant_device
```

Expected: compile or link failure for missing implementation symbols.

- [ ] **Step 6: Commit the red API test**

```bash
git -C third_party/iolinki add include/iolinki/device.h tests/test_reentrant_device.c tests/CMakeLists.txt
git -C third_party/iolinki commit -m "test: define reentrant device API expectations"
```

## Task 2: Introduce Context Backends For Device Info And Parameters

**Files:**

- Modify: `third_party/iolinki/include/iolinki/device_info.h`
- Modify: `third_party/iolinki/include/iolinki/params.h`
- Modify: `third_party/iolinki/src/device_info.c`
- Modify: `third_party/iolinki/src/params.c`

- [ ] **Step 1: Add context types**

In `device_info.h`, add:

```c
typedef struct
{
    const iolink_device_info_t* configured;
    iolink_device_info_t defaults;
    char application_tag[33];
    uint16_t access_locks;
} iolink_device_info_ctx_t;

void iolink_device_info_ctx_init(iolink_device_info_ctx_t* ctx,
                                 const iolink_device_info_t* configured);
const iolink_device_info_t* iolink_device_info_ctx_get(const iolink_device_info_ctx_t* ctx);
int iolink_device_info_ctx_set_application_tag(iolink_device_info_ctx_t* ctx,
                                               const char* tag,
                                               uint8_t len);
uint16_t iolink_device_info_ctx_get_access_locks(const iolink_device_info_ctx_t* ctx);
void iolink_device_info_ctx_set_access_locks(iolink_device_info_ctx_t* ctx, uint16_t locks);
```

In `params.h`, add:

```c
#include "iolinki/device_info.h"

typedef struct
{
    char application_tag[33];
    char function_tag[33];
    char location_tag[33];
    bool application_tag_valid;
    bool function_tag_valid;
    bool location_tag_valid;
    iolink_device_info_ctx_t* device_info;
} iolink_params_ctx_t;

void iolink_params_ctx_init(iolink_params_ctx_t* ctx, iolink_device_info_ctx_t* device_info);
int iolink_params_ctx_get(iolink_params_ctx_t* ctx,
                          uint16_t index,
                          uint8_t subindex,
                          uint8_t* buffer,
                          size_t max_len);
int iolink_params_ctx_set(iolink_params_ctx_t* ctx,
                          uint16_t index,
                          uint8_t subindex,
                          const uint8_t* data,
                          size_t len,
                          bool persist);
void iolink_params_ctx_factory_reset(iolink_params_ctx_t* ctx);
```

- [ ] **Step 2: Implement context functions and remove singleton storage**

Move existing `g_device_info`, `g_default_info`, `g_app_tag_buffer`, and `g_nvm_shadow` behavior into context functions. Delete the file-static parameter/device-info state after the context tests are green. Any existing tests that call singleton helpers must be migrated to context helpers in this task.

The new call shape must be:

```c
iolink_device_info_ctx_t info;
iolink_params_ctx_t params;

iolink_device_info_ctx_init(&info, NULL);
iolink_params_ctx_init(&params, &info);
int n = iolink_params_ctx_get(&params, index, subindex, buffer, sizeof(buffer));
```

- [ ] **Step 3: Add context isolation assertions**

Extend `test_reentrant_device.c` with:

```c
static void test_parameter_contexts_keep_writable_tags_isolated(void** state)
{
    (void)state;
    iolink_device_info_ctx_t info_a;
    iolink_device_info_ctx_t info_b;
    iolink_params_ctx_t params_a;
    iolink_params_ctx_t params_b;
    const uint8_t tag_a[] = "DeviceA";
    const uint8_t tag_b[] = "DeviceB";
    uint8_t out_a[32] = {0};
    uint8_t out_b[32] = {0};

    iolink_device_info_ctx_init(&info_a, NULL);
    iolink_device_info_ctx_init(&info_b, NULL);
    iolink_params_ctx_init(&params_a, &info_a);
    iolink_params_ctx_init(&params_b, &info_b);

    assert_int_equal(iolink_params_ctx_set(&params_a, IOLINK_IDX_APPLICATION_TAG, 0U,
                                           tag_a, sizeof(tag_a) - 1U, true), 0);
    assert_int_equal(iolink_params_ctx_set(&params_b, IOLINK_IDX_APPLICATION_TAG, 0U,
                                           tag_b, sizeof(tag_b) - 1U, true), 0);
    assert_int_equal(iolink_params_ctx_get(&params_a, IOLINK_IDX_APPLICATION_TAG, 0U,
                                           out_a, sizeof(out_a)), (int)(sizeof(tag_a) - 1U));
    assert_int_equal(iolink_params_ctx_get(&params_b, IOLINK_IDX_APPLICATION_TAG, 0U,
                                           out_b, sizeof(out_b)), (int)(sizeof(tag_b) - 1U));
    assert_memory_equal(out_a, tag_a, sizeof(tag_a) - 1U);
    assert_memory_equal(out_b, tag_b, sizeof(tag_b) - 1U);
}
```

- [ ] **Step 4: Verify**

Run:

```bash
cmake --build /tmp/iolinki-reentrant-build --target test_reentrant_device
/tmp/iolinki-reentrant-build/tests/test_reentrant_device
ctest --test-dir /tmp/iolinki-reentrant-build --output-on-failure
```

Expected: context tests pass; any remaining `test_reentrant_device` failure is only from missing full `iolink_device_*` implementation if Task 1 skeleton is still linked.

- [ ] **Step 5: Commit**

```bash
git -C third_party/iolinki add include/iolinki/device_info.h include/iolinki/params.h src/device_info.c src/params.c tests/test_reentrant_device.c
git -C third_party/iolinki commit -m "refactor: add isolated device parameter contexts"
```

## Task 3: Implement `iolink_device_ctx_t` Core And Delete Singleton Entry Points

**Files:**

- Modify: `third_party/iolinki/include/iolinki/iolink.h`
- Modify: `third_party/iolinki/include/iolinki/phy.h`
- Modify: `third_party/iolinki/include/iolinki/dll.h`
- Modify: `third_party/iolinki/src/iolink_core.c`
- Test: `third_party/iolinki/tests/test_reentrant_device.c`

- [ ] **Step 1: Confirm the public context layout compiles**

The complete `iolink_device_ctx_t` layout lives in `device.h`. Do not create a separate heap-allocated private object. This matches UDSLib's public caller-owned `uds_ctx_t` style and keeps embedded allocation explicit.

Add a user-aware PHY pointer to `iolink_dll_ctx_t`:

```c
const iolink_device_phy_t* device_phy;
uint64_t (*time_us)(void* user);
void* time_user;
```

Remove old `const iolink_phy_api_t* phy` use from the core once DLL calls use `device_phy`.

- [ ] **Step 2: Implement instance init**

In `iolink_core.c`, include `device.h` and implement:

```c
size_t iolink_device_ctx_size(void)
{
    return sizeof(iolink_device_ctx_t);
}

int iolink_device_init(iolink_device_ctx_t* ctx, const iolink_device_config_t* config)
{
    if((ctx == NULL) || (config == NULL)) {
        return -1;
    }
    memset(ctx, 0, sizeof(*ctx));
    ctx->config = config;
    memcpy(&ctx->stack_config, &config->stack, sizeof(ctx->stack_config));
    iolink_device_info_ctx_init(&ctx->device_info, config->device_info);
    iolink_params_ctx_init(&ctx->params, &ctx->device_info);
    if(config->phy.init != NULL) {
        int err = config->phy.init(config->phy.user);
        if(err != 0) {
            return err;
        }
    }
    iolink_dll_init_device_phy(&ctx->dll, &config->phy);
    ctx->dll.owner = ctx;
    ctx->dll.time_us = config->time_us;
    ctx->dll.time_user = config->user;
    ctx->dll.m_seq_type = (uint8_t)ctx->stack_config.m_seq_type;
    ctx->dll.pd_in_len = ctx->stack_config.pd_in_len;
    ctx->dll.pd_out_len = ctx->stack_config.pd_out_len;
    ctx->dll.min_cycle_time_us = (uint32_t)ctx->stack_config.min_cycle_time * 100U;
    ctx->dll.t_pd_delay_us = ctx->stack_config.t_pd_us;
    return 0;
}
```

- [ ] **Step 3: Replace DLL PHY calls with user-aware calls**

In `dll.c`, replace direct calls like:

```c
ctx->phy->send(data, len);
ctx->phy->recv_byte(&byte);
ctx->phy->detect_wakeup();
```

with helpers:

```c
static int dll_phy_send(iolink_dll_ctx_t* ctx, const uint8_t* data, size_t len)
{
    if((ctx == NULL) || (ctx->device_phy == NULL) || (ctx->device_phy->send == NULL)) {
        return -1;
    }
    return ctx->device_phy->send(ctx->device_phy->user, data, len);
}
```

Create equivalent helpers for `recv_byte`, `detect_wakeup`, `set_mode`, `set_baudrate`, diagnostics, and SIO line control.

Also replace direct `iolink_time_get_us()` calls in `dll.c` with:

```c
static uint64_t dll_time_us(const iolink_dll_ctx_t* ctx)
{
    if((ctx != NULL) && (ctx->time_us != NULL)) {
        return ctx->time_us(ctx->time_user);
    }
    return iolink_time_get_us();
}
```

Use optional `config->lock(config->user)` / `config->unlock(config->user)` around public `iolink_device_*` operations that can race with RX/tick callers. If no lock hooks are provided, the stack remains single-threaded/cooperative as today.

- [ ] **Step 4: Implement instance methods and remove singleton API**

Move current `g_dll_ctx` behavior to `ctx->dll`. Delete the file-static globals:

```c
g_dll_ctx
g_config
g_reset_handler
g_app_callbacks
```

Remove singleton functions from `iolink.h` and migrate tests/examples to the `iolink_device_*` equivalents. If keeping source compatibility temporarily is useful during the same task, keep static `static` helper functions inside tests only; do not leave public singleton APIs in the library.

- [ ] **Step 5: Verify**

Run:

```bash
cmake --build /tmp/iolinki-reentrant-build --target test_reentrant_device
/tmp/iolinki-reentrant-build/tests/test_reentrant_device
ctest --test-dir /tmp/iolinki-reentrant-build --output-on-failure
```

Expected: public API test and migrated suite pass.

- [ ] **Step 6: Commit**

```bash
git -C third_party/iolinki add include/iolinki/device.h include/iolinki/iolink.h include/iolinki/phy.h include/iolinki/dll.h src/dll.c src/iolink_core.c tests/test_reentrant_device.c
git -C third_party/iolinki commit -m "feat: add caller-owned IO-Link device contexts"
```

## Task 4: Inject ISDU Dependencies Through The Device Context

**Files:**

- Modify: `third_party/iolinki/include/iolinki/isdu.h`
- Modify: `third_party/iolinki/src/dll.c`
- Modify: `third_party/iolinki/src/isdu.c`
- Modify: `third_party/iolinki/src/iolink_core.c`
- Test: `third_party/iolinki/tests/test_reentrant_device.c`

- [ ] **Step 1: Add ISDU backend pointers**

In `iolink_isdu_ctx_t`, replace global access assumptions with:

```c
void* device_ctx;
void* params_ctx;
void* device_info_ctx;
uint8_t* direct_param_page2;
```

Set these in `iolink_device_init`:

```c
ctx->dll.isdu.device_ctx = ctx;
ctx->dll.isdu.params_ctx = &ctx->params;
ctx->dll.isdu.device_info_ctx = &ctx->device_info;
ctx->dll.isdu.direct_param_page2 = ctx->direct_param_page2;
ctx->dll.isdu.event_ctx = &ctx->dll.events;
ctx->dll.isdu.ds_ctx = &ctx->dll.ds;
ctx->dll.isdu.dll_ctx = &ctx->dll;
```

- [ ] **Step 2: Replace global calls in ISDU**

In `isdu.c`, replace:

```c
iolink_params_get
iolink_params_set
iolink_params_factory_reset
iolink_device_info_get
iolink_device_info_get_access_locks
iolink_device_info_set_access_locks
g_direct_param_page2
```

with context-backed calls using `ctx->params_ctx`, `ctx->device_info_ctx`, and `ctx->direct_param_page2`.

- [ ] **Step 3: Add Direct Parameter page 2 isolation assertion**

Add a test that writes different Direct Parameter page 2 data through two ISDU contexts and verifies the buffers differ. Use the existing ISDU direct parameter helpers in `isdu.c`; if no public helper exists, test through an ISDU write/read transaction in `test_reentrant_device.c`.

- [ ] **Step 4: Verify**

Run:

```bash
cmake --build /tmp/iolinki-reentrant-build --target test_reentrant_device
/tmp/iolinki-reentrant-build/tests/test_reentrant_device
ctest --test-dir /tmp/iolinki-reentrant-build --output-on-failure
```

Expected: context isolation and migrated tests pass.

- [ ] **Step 5: Commit**

```bash
git -C third_party/iolinki add include/iolinki/isdu.h src/dll.c src/isdu.c src/iolink_core.c tests/test_reentrant_device.c
git -C third_party/iolinki commit -m "refactor: inject ISDU device dependencies"
```

## Task 5: Make Data Storage Instance-Isolated

**Files:**

- Modify: `third_party/iolinki/include/iolinki/data_storage.h`
- Modify: `third_party/iolinki/src/data_storage.c`
- Modify: `third_party/iolinki/src/iolink_core.c`
- Test: `third_party/iolinki/tests/test_reentrant_device.c`

- [ ] **Step 1: Add DS parameter backend**

In `data_storage.h`, add:

```c
typedef struct
{
    int (*get)(void* user, uint16_t index, uint8_t subindex, uint8_t* buffer, size_t max_len);
    int (*set)(void* user,
               uint16_t index,
               uint8_t subindex,
               const uint8_t* data,
               size_t len,
               bool persist);
    void* user;
} iolink_ds_params_api_t;
```

Add `iolink_ds_params_api_t params;` to `iolink_ds_ctx_t` and:

```c
void iolink_ds_set_params_api(iolink_ds_ctx_t* ctx, const iolink_ds_params_api_t* params);
```

- [ ] **Step 2: Wire device params into DS**

In `iolink_core.c`, create adapters from `iolink_params_ctx_get/set` and call `iolink_ds_set_params_api(&ctx->dll.ds, &api)` from `iolink_device_init`.

- [ ] **Step 3: Replace global params in DS**

In `data_storage.c`, replace `iolink_params_get/set` with `ctx->params.get/set`. Missing callbacks return `-1`.

- [ ] **Step 4: Add DS isolation test**

In `test_reentrant_device.c`, initialize two devices, set different Application Tags, call `iolink_ds_get_image(iolink_device_get_ds_ctx(dev), &len)` for each, and assert the images differ.

- [ ] **Step 5: Verify and commit**

Run:

```bash
cmake --build /tmp/iolinki-reentrant-build --target test_reentrant_device
/tmp/iolinki-reentrant-build/tests/test_reentrant_device
ctest --test-dir /tmp/iolinki-reentrant-build --output-on-failure
```

Then:

```bash
git -C third_party/iolinki add include/iolinki/data_storage.h src/data_storage.c src/iolink_core.c tests/test_reentrant_device.c
git -C third_party/iolinki commit -m "refactor: isolate data storage per device context"
```

## Task 6: Prove Real Master Against Several Real Device Contexts

**Files:**

- Create: `third_party/iolinki/tests/test_multi_device_real_master.c`
- Modify: `third_party/iolinki/tests/CMakeLists.txt`

- [ ] **Step 1: Add real multi-device conformance harness**

Create a harness with per-port state:

```c
typedef struct
{
    link_queue_t master_to_device;
    link_queue_t device_to_master;
    int wakeup_pending;
    uint8_t observed_pd_out[32];
    uint8_t observed_pd_out_len;
    uint8_t device_ctx_storage[512];
    iolink_device_ctx_t* device;
    iolink_master_port_t master_port;
    iolink_device_config_t device_config;
    iolink_master_config_t master_config;
} port_pair_t;
```

Use one `iolink_master_controller_t` with two or four `iolink_master_port_t` ports. Each simulated cycle:

1. Tick the controller.
2. Pump every real `iolink_device_ctx_t`.
3. Poll/tick the controller for received bytes.
4. Repeat until every port reaches `IOLINK_MASTER_STATE_OPERATE`.

Assertions:

```c
assert_int_equal(iolink_master_get_state(&pair_a.master_port), IOLINK_MASTER_STATE_OPERATE);
assert_int_equal(iolink_master_get_state(&pair_b.master_port), IOLINK_MASTER_STATE_OPERATE);
assert_memory_not_equal(pd_in_a, pd_in_b, min_len);
assert_memory_not_equal(pair_a.observed_pd_out, pair_b.observed_pd_out, min_len);
```

- [ ] **Step 2: Keep master/device boundaries clean**

The test may include `iolinki_master/master.h` and link master sources, but production `iolinki` must not depend on `iolinki-master`. Add `IOLINKI_MASTER_DIR` as an optional CMake test-only path.

- [ ] **Step 3: Register and run**

Run:

```bash
cmake -S third_party/iolinki -B /tmp/iolinki-reentrant-build -DBUILD_TESTING=ON -DIOLINKI_MASTER_DIR=/home/andrii/projects/iolinki-master
cmake --build /tmp/iolinki-reentrant-build --target test_multi_device_real_master
/tmp/iolinki-reentrant-build/tests/test_multi_device_real_master
ctest --test-dir /tmp/iolinki-reentrant-build --output-on-failure
```

Expected: several independent real devices reach OPERATE against real master ports in one process.

- [ ] **Step 4: Commit**

```bash
git -C third_party/iolinki add tests/test_multi_device_real_master.c tests/CMakeLists.txt
git -C third_party/iolinki commit -m "test: prove real master multi-device conformance"
```

## Task 7: Update LabWired Native Conformance

**Files:**

- Modify: `crates/core/native/iolink_conformance.c`
- Modify: `crates/core/src/peripherals/components/iolink_master.rs`
- Modify: `crates/core/build.rs`

- [ ] **Step 1: Add failing Rust FFI test**

Add `NativeMultiDeviceConformanceResult` and FFI for:

```c
int lw_iolm_conformance_run_multi_device(lw_iolm_multi_device_result_t* result);
```

Add Rust test:

```rust
#[cfg(feature = "iolink-native")]
#[test]
fn native_real_master_runs_several_real_device_stack_instances() {
    let result = super::native::run_real_multi_device_stack_conformance()
        .expect("real multi-device IO-Link conformance");
    assert_eq!(result.port_count, 2);
    assert_eq!(result.operate_count, 2);
    assert_ne!(
        &result.pd_in_a[..result.pd_in_len_a as usize],
        &result.pd_in_b[..result.pd_in_len_b as usize]
    );
}
```

Run:

```bash
IOLINKI_MASTER_DIR=/home/andrii/projects/iolinki-master \
IOLINKI_DEVICE_DIR=/home/andrii/projects/labwired/core/.worktrees/iolink-simulator-conformance/third_party/iolinki \
cargo test -p labwired-core --features iolink-native native_real_master_runs_several_real_device_stack_instances --lib -- --nocapture
```

Expected: link failure for missing `lw_iolm_conformance_run_multi_device`.

- [ ] **Step 2: Implement LabWired helper with per-device link state**

In `crates/core/native/iolink_conformance.c`, remove shared globals from the multi-device path. Use `port_pair_t` with its own queues, wakeup flag, observed PD buffers, master port, and `iolink_device_ctx_t` storage per device.

Do not hand-author response bytes. All device responses must come from `iolink_device_process()`.

- [ ] **Step 3: Verify**

Run:

```bash
IOLINKI_MASTER_DIR=/home/andrii/projects/iolinki-master \
IOLINKI_DEVICE_DIR=/home/andrii/projects/labwired/core/.worktrees/iolink-simulator-conformance/third_party/iolinki \
cargo test -p labwired-core --features iolink-native native_ --lib -- --nocapture
IOLINKI_MASTER_DIR=/home/andrii/projects/iolinki-master \
IOLINKI_DEVICE_DIR=/home/andrii/projects/labwired/core/.worktrees/iolink-simulator-conformance/third_party/iolinki \
cargo check -p labwired-core --features iolink-native
git diff --check
```

Expected: native selector and feature check pass.

- [ ] **Step 4: Commit**

```bash
git add crates/core/native/iolink_conformance.c crates/core/src/peripherals/components/iolink_master.rs crates/core/build.rs third_party/iolinki
git commit -m "sim: test real IO-Link master against several device contexts"
```

## Task 8: CI And Documentation

**Files:**

- Modify: `.github/workflows/core-iolink-native.yml`
- Modify: `third_party/iolinki/docs/API.md`
- Modify: `third_party/iolinki/docs/ARCHITECTURE.md`

- [ ] **Step 1: Keep hosted native CI**

Ensure the workflow still runs:

```yaml
run: cargo test -p labwired-core --features iolink-native native_ --lib -- --nocapture
```

- [ ] **Step 2: Document UDSLib-style ownership**

Add to `docs/API.md`:

```markdown
## Reentrant Device Contexts

New integrations should use caller-owned `iolink_device_ctx_t` instances with a
borrowed `iolink_device_config_t`. The stack does not allocate memory and does
not own transport, clock, mutex, storage, or application objects. These are
provided through callbacks and `void *user`.

The old singleton `iolink_init()` / `iolink_process()` API was removed before
public adoption. All examples use caller-owned `iolink_device_ctx_t`.
```

Add to `docs/ARCHITECTURE.md`:

```markdown
## Role And Instance Boundaries

`iolinki` owns the IO-Link Device role. `iolinki-master` owns the IO-Link Master
role. Shared code is limited to frame/checksum/protocol helpers and test-only
real-stack conformance harnesses.

Every simulated or physical IO-Link Device should have one
`iolink_device_ctx_t`. Bundled platform PHYs may remain single-port adapters,
but protocol state is not global.
```

- [ ] **Step 3: Final verification**

Run:

```bash
cmake -S third_party/iolinki -B /tmp/iolinki-reentrant-build -DBUILD_TESTING=ON -DIOLINKI_MASTER_DIR=/home/andrii/projects/iolinki-master
cmake --build /tmp/iolinki-reentrant-build
ctest --test-dir /tmp/iolinki-reentrant-build --output-on-failure
IOLINKI_MASTER_DIR=/home/andrii/projects/iolinki-master \
IOLINKI_DEVICE_DIR=/home/andrii/projects/labwired/core/.worktrees/iolink-simulator-conformance/third_party/iolinki \
cargo test -p labwired-core --features iolink-native native_ --lib -- --nocapture
IOLINKI_MASTER_DIR=/home/andrii/projects/iolinki-master \
IOLINKI_DEVICE_DIR=/home/andrii/projects/labwired/core/.worktrees/iolink-simulator-conformance/third_party/iolinki \
cargo check -p labwired-core --features iolink-native
git diff --check
```

Expected: all commands pass.

- [ ] **Step 4: Commit and push**

```bash
git add .github/workflows/core-iolink-native.yml third_party/iolinki/docs/API.md third_party/iolinki/docs/ARCHITECTURE.md third_party/iolinki
git commit -m "docs: describe reentrant IO-Link device architecture"
git push
```

## Self-Review

Coverage:

- UDSLib-style caller-owned contexts and borrowed config: Public API Shape, Tasks 1 and 3.
- Existing `iolinki` context migration order: Tasks 2 through 5.
- Direct Parameter page 2 global: Task 4.
- Master/device role separation: Scope, Task 6, Task 8 docs.
- LabWired multi-device no-shim conformance: Task 7.
- Hosted CI and documentation: Task 8.

Quality checks:

- No fake response generation is allowed in the conformance path.
- The plan removes legacy singleton public APIs instead of preserving wrappers.
- The plan treats bundled singleton PHYs as migration targets, not as the core instance model.

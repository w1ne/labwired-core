# YAML wiring sugar Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Chip YAML accepts `irq: nvic@2`, flattened instance keys, and `include:` for path-loaded descriptors, without breaking existing files.

**Architecture:** Custom serde on `PeripheralConfig` plus a post-parse include expander in `ChipDescriptor::from_file`. Engine still reads `irq: Option<u32>` and `config: HashMap`.

**Tech Stack:** Rust, serde, serde_yaml, labwired-config crate tests.

**Work from:** `/tmp/labwired-yaml-sugar` on branch `feat/yaml-wiring-sugar`.

---

### Task 1: IRQ target sugar (`nvic@2`)

**Files:**
- Modify: `crates/config/src/lib.rs` (`PeripheralConfig`, irq deserialize)
- Modify: `crates/config/tests/config_tests.rs`

- [ ] **Step 1: Write failing tests** in `crates/config/tests/config_tests.rs`:

```rust
#[test]
fn irq_accepts_bare_number() {
    let p: PeripheralConfig = serde_yaml::from_str(
        r#"
id: uart0
type: uart
base_address: 0x40002000
irq: 2
"#,
    )
    .unwrap();
    assert_eq!(p.irq, Some(2));
    assert_eq!(p.irq_controller.as_deref(), None);
}

#[test]
fn irq_accepts_controller_at_line() {
    let p: PeripheralConfig = serde_yaml::from_str(
        r#"
id: uart0
type: uart
base_address: 0x40002000
irq: nvic@2
"#,
    )
    .unwrap();
    assert_eq!(p.irq, Some(2));
    assert_eq!(p.irq_controller.as_deref(), Some("nvic"));
}

#[test]
fn irq_rejects_malformed_target() {
    let err = serde_yaml::from_str::<PeripheralConfig>(
        r#"
id: uart0
type: uart
base_address: 0x40002000
irq: nvic@
"#,
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("irq"), "{msg}");
}
```

- [ ] **Step 2: Run tests, expect FAIL** (no `irq_controller` / parse of `nvic@2`)

```
cd /tmp/labwired-yaml-sugar
cargo test -p labwired-config --test config_tests irq_accepts -- --nocapture
```

- [ ] **Step 3: Implement**

On `PeripheralConfig` add:

```rust
#[serde(default)]
pub irq_controller: Option<String>,
```

Replace `irq: Option<u32>` deserialize with a custom function that accepts YAML number, string digits, or `controller@digits`. Set `irq` to the number and `irq_controller` when a controller prefix is present. Keep `#[serde(default)]` on `irq`.

Existing `irq: 37` fixtures must still deserialize.

- [ ] **Step 4: Run tests, expect PASS** including the whole `config_tests` binary.

- [ ] **Step 5: Commit** `feat(config): accept irq: nvic@2 wiring sugar`

---

### Task 2: Flatten instance properties into `config`

**Files:**
- Modify: `crates/config/src/lib.rs` (`PeripheralConfig`)
- Modify: `crates/config/tests/config_tests.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn flattened_keys_land_in_config() {
    let p: PeripheralConfig = serde_yaml::from_str(
        r#"
id: uart0
type: nrf52840_uart
base_address: 0x40002000
easyDMA: true
"#,
    )
    .unwrap();
    assert_eq!(p.config.get("easyDMA").and_then(|v| v.as_bool()), Some(true));
}

#[test]
fn nested_config_wins_over_flattened_key() {
    let p: PeripheralConfig = serde_yaml::from_str(
        r#"
id: uart0
type: uart
base_address: 0x4000
easyDMA: true
config:
  easyDMA: false
"#,
    )
    .unwrap();
    assert_eq!(p.config.get("easyDMA").and_then(|v| v.as_bool()), Some(false));
}
```

- [ ] **Step 2: Run, expect FAIL**

- [ ] **Step 3: Implement** with `#[serde(flatten)] extra: HashMap<String, Value>` (or a custom deserializer). After parse, insert extra keys into `config` only when the key is not already present (`config:` wins). Do not put `id`/`type`/`base_address`/`size`/`irq`/`clock`/`config`/`irq_controller` into `config`.

- [ ] **Step 4: Run full `cargo test -p labwired-config`** — PASS.

- [ ] **Step 5: Commit** `feat(config): flatten peripheral instance keys into config`

---

### Task 3: `include:` for path-loaded chip YAML

**Files:**
- Modify: `crates/config/src/lib.rs` (`ChipDescriptor`, `from_file`)
- Modify: `crates/config/tests/config_tests.rs`

- [ ] **Step 1: Write failing tests** using `tempfile` or `std::env::temp_dir()`:

Include file `common.yaml` with `arch`, `cpu_hz`, `flash`, `ram`, and a peripheral `uart0`. Child file `include: common.yaml`, overrides `name` and adds `uart1`, overrides `uart0` irq.

Assert: child's `name`, both peripherals, child's uart0 irq wins, include-only fields (`cpu_hz`) survive.

Second test: missing include path errors.

Third test: A includes B includes A → cycle error containing "cycle".

- [ ] **Step 2: Run, expect FAIL**

- [ ] **Step 3: Implement**

Add to `ChipDescriptor`:

```rust
#[serde(default, skip_serializing)]
include: Option<ChipInclude>, // string or seq of strings
```

Do **not** require include on serde_yaml::from_str of builtins.

`from_file`:
1. Deserialize raw YAML to `ChipDescriptor` (includes not yet expanded).
2. `expand_includes(self, file_dir, stack: &mut Vec<PathBuf>)`.
3. For each include path (relative to `file_dir`), `from_file` recursively.
4. Merge as in the spec.
5. Detect cycles via canonicalized paths on the stack.

`resolve()` path branch already calls `from_file` — inherits expansion. Builtin `from_str` does not.

- [ ] **Step 4: `cargo test -p labwired-config` PASS**

- [ ] **Step 5: Commit** `feat(config): chip YAML include with id-override merge`

---

### Task 4: Rustdoc + example fixture

**Files:**
- Modify: rustdoc on `PeripheralConfig` and `ChipDescriptor` in `crates/config/src/lib.rs`
- Create: `crates/config/tests/fixtures/wiring-sugar-common.yaml` and `wiring-sugar-child.yaml` used by Task 3 or a load test

- [ ] Document the three sugars with YAML snippets in rustdoc.
- [ ] Commit `docs(config): document chip YAML wiring sugar`

---

Self-review: IRQ, flatten, include covered. No `.repl`. No catalog migration. No builtin include.

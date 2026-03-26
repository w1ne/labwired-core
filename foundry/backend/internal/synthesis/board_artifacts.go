package synthesis

import (
	"fmt"
	"sort"
	"strings"
)

func buildBoardDraft(req Request, resolution boardResolution) *BoardDraft {
	boardID := inferBoardID(req)
	chipGuess := resolution.ChipGuess
	examples := recommendedExamples(req, resolution)
	deferredScope := []string{}
	if boardHasWirelessScope(req, resolution.Profile, chipGuess) {
		deferredScope = []string{
			"wireless coprocessor / CPU2 behavior",
			"IPCC/HCI transport",
			"radio timing fidelity",
		}
	}

	return &BoardDraft{
		BoardID:               boardID,
		ChipGuess:             chipGuess,
		RequestedCapabilities: append([]string(nil), requestedBoardCapabilities(req)...),
		ValidatedCapabilities: append([]string(nil), validatedBoardCapabilities(req)...),
		ValidationTargets:     append([]string(nil), req.ValidationTargets...),
		BringupScope:          []string{"app-core boot", "rcc", "gpio", "uart console", "board LEDs", "user button"},
		DeferredScope:         deferredScope,
		RepoArtifacts: []DraftArtifact{
			{Path: fmt.Sprintf("core/configs/chips/%s.yaml", chipFileName(chipGuess, boardID)), Purpose: "chip descriptor draft"},
			{Path: fmt.Sprintf("core/configs/systems/%s.yaml", boardID), Purpose: "board system manifest draft"},
			{Path: fmt.Sprintf("core/examples/%s/README.md", boardID), Purpose: "board example README"},
			{Path: fmt.Sprintf("core/examples/%s/system.yaml", boardID), Purpose: "reproducible local system manifest"},
			{Path: fmt.Sprintf("core/examples/%s/VALIDATION.md", boardID), Purpose: "validation record template"},
		},
		RecommendedExamples: examples,
		SourceRequirements: []string{
			"MCU reference manual",
			"datasheet with memory map and package details",
			"board user manual / schematic for LED and button mapping",
			"official vendor firmware example for proof workload",
		},
		ValidationPlan: boardValidationPlan(req),
		OpenQuestions:  boardOpenQuestions(req, resolution),
	}
}

func buildBoardContractResult(req Request, draft *BoardDraft) *ContractResult {
	if draft == nil {
		return nil
	}
	deferredCapabilities := []string{}
	if boardHasWirelessScope(req, boardProfile{}, draft.ChipGuess) {
		deferredCapabilities = []string{
			"wireless_coprocessor",
			"ipcc_hci_transport",
			"radio_timing_fidelity",
		}
	}
	return &ContractResult{
		RequestKind:           pick(req.Kind, req.Kind != "", "board_onboarding"),
		RequestedCapabilities: append([]string(nil), draft.RequestedCapabilities...),
		ValidatedCapabilities: append([]string(nil), draft.ValidatedCapabilities...),
		DeferredCapabilities:  deferredCapabilities,
		MissingCapabilities:   missingBoardCapabilities(req),
		ValidationTargets:     append([]string(nil), draft.ValidationTargets...),
		EvidenceArtifacts: []string{
			"repo_bundle.files",
			"validate_chip.json",
			"validate_system.json",
			"uart_smoke_result.json",
			"unsupported_audit_report.md",
		},
		PromotionMode: boardPromotionMode(req),
	}
}

func missingBoardCapabilities(req Request) []string {
	requested := requestedBoardCapabilities(req)
	validated := map[string]bool{}
	for _, capability := range validatedBoardCapabilities(req) {
		validated[strings.TrimSpace(capability)] = true
	}
	missing := []string{}
	for _, capability := range requested {
		if !validated[strings.TrimSpace(capability)] {
			missing = append(missing, capability)
		}
	}
	return missing
}

func buildPeripheralContractResult(req Request) *ContractResult {
	return &ContractResult{
		RequestKind:           pick(req.Kind, req.Kind != "", "peripheral_model_ingest"),
		RequestedCapabilities: inferBusHints(req.Requirements),
		ValidatedCapabilities: []string{"register_draft_generated", "strict_ir_generated"},
		ValidationTargets:     append([]string(nil), req.ValidationTargets...),
		EvidenceArtifacts:     []string{"output.json"},
		PromotionMode:         "artifact_only",
	}
}

func buildBoardRepoBundle(req Request, draft *BoardDraft, sourceDocs []SourceDoc) *RepoBundle {
	chipFile := chipFileName(draft.ChipGuess, draft.BoardID)
	systemPath := fmt.Sprintf("core/configs/systems/%s.yaml", draft.BoardID)
	exampleDir := fmt.Sprintf("core/examples/%s", draft.BoardID)
	chipPath := fmt.Sprintf("core/configs/chips/%s.yaml", chipFile)
	firmwarePackage := fmt.Sprintf("firmware-%s-demo", draft.BoardID)
	firmwareDir := fmt.Sprintf("%s/board_firmware", exampleDir)
	docsSection := renderSourceDocs(sourceDocs)
	resolution := resolveBoard(req, sourceDocs)
	profile, hasProfile := resolution.Profile, resolution.HasProfile
	rustTarget := boardRustTarget(profile, hasProfile)
	stackTop := boardStackTop(profile, hasProfile)
	vendorPackage := boardVendorExamplesPackage(profile, req)
	recommended := renderRecommendedExamples(recommendedExamples(req, resolution))
	chipName := strings.ToUpper(chipFile)
	uartID := pick(profile.UARTID, hasProfile, "uart_console")
	uartBase := pick(profile.UARTBase, hasProfile, "TODO")
	uartIRQ := pick(profile.UARTIRQ, hasProfile, "TODO")
	defaultBoardPort := boardDefaultPort(profile, hasProfile)
	arch := boardArch(profile, hasProfile)
	flashBase := boardFlashBase(profile, hasProfile)
	ramBase := boardRAMBase(profile, hasProfile)

	chipYAML := fmt.Sprintf(`schema_version: "1.0"
name: "%s"
arch: "%s"
flash:
  base: %s
  size: "%s"
ram:
  base: %s
  size: "%s"

peripherals:
%s
`, chipName, arch, flashBase, pick(profile.FlashSize, hasProfile, "TODO"), ramBase, pick(profile.RAMSize, hasProfile, "TODO"), renderChipPeripherals(profile, hasProfile, uartID, uartBase, uartIRQ))

	systemYAML := fmt.Sprintf(`schema_version: "1.0"
name: "%s"
chip: "../chips/%s.yaml"

board_io:
%s%s
`, draft.BoardID, chipFile, renderBoardIOList(profile.LEDs, profile, hasProfile, "led_user", defaultBoardPort, "led"), renderBoardIOList(profile.Buttons, profile, hasProfile, "button_user", defaultBoardPort, "button"))

	readme := fmt.Sprintf(`# %s

This example was synthesized by Foundry as a board onboarding starter.

## Scope

- App-core boot path
- RCC/GPIO/UART baseline
- Board LED and user button mapping
%s

## Recommended Vendor Example

Reference package: %s

%s

## Required Source Confirmation

- Confirm MCU part number and package
- Confirm VCP UART instance on ST-LINK
- Confirm LED and button GPIO mappings from schematic
%s

## Auto-Resolved Source Docs

%s
`, draft.BoardID, boardScopeLine(req, profile), vendorPackage, recommended, boardConfirmationLine(req, profile, draft.ChipGuess), docsSection)

	requiredDocs := fmt.Sprintf(`# Required Docs

%s
`, docsSection)

	externalComponents := `# External Components

- ST-LINK virtual COM port connection
- On-board user LEDs
- On-board user button
`
	if boardHasWirelessScope(req, profile, draft.ChipGuess) {
		externalComponents += "- BLE/radio subsystem is deferred unless engine support is explicitly added\n"
	}

	validation := fmt.Sprintf(`# Validation

Run from `+"`core/`"+`:

`+"```bash"+`
cargo build --manifest-path examples/%s/board_firmware/Cargo.toml --release --target %s
cargo run -q -p labwired-cli -- test \
  --script examples/%s/uart-smoke.yaml \
  --output-dir out/%s/uart-smoke \
  --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware examples/%s/board_firmware/target/%s/release/%s \
  --system configs/systems/%s.yaml \
  --max-steps 200000 \
  --out-dir out/unsupported-audit/%s
`+"```"+`

Expected evidence:

- PC/SP initialize correctly
- UART smoke output is deterministic
- LED/button paths are mapped and exercised
`, draft.BoardID, rustTarget, draft.BoardID, draft.BoardID, draft.BoardID, rustTarget, firmwarePackage, draft.BoardID, draft.BoardID)

	systemCopy := fmt.Sprintf(`schema_version: "1.0"
name: "%s"
chip: ../../configs/chips/%s.yaml
board_io:
%s%s
`, draft.BoardID, chipFile, renderBoardIOList(profile.LEDs, profile, hasProfile, "led_user", defaultBoardPort, "led"), renderBoardIOList(profile.Buttons, profile, hasProfile, "button_user", defaultBoardPort, "button"))

	smokeCargoToml := renderSmokeCargoToml(firmwarePackage, arch)
	smokeBuildRS := renderSmokeBuildRS(arch)
	smokeMemory := renderSmokeMemory(profile, hasProfile, flashBase, ramBase)
	smokeLinker := renderSmokeLinker(profile, hasProfile, arch, flashBase, ramBase, stackTop)
	smokeMain := renderSmokeMain(arch, uartID, uartBase)
	smokeMaxSteps := boardSmokeMaxSteps(arch)

	uartSmoke := fmt.Sprintf(`# LabWired - %s UART smoke test
schema_version: "1.0"
inputs:
  firmware: "./board_firmware/target/%s/release/%s"
  system: "./system.yaml"
limits:
  max_steps: %d
assertions:
  - uart_contains: "OK"
  - expected_stop_reason: max_steps
`, strings.ToUpper(draft.BoardID), rustTarget, firmwarePackage, smokeMaxSteps)

	return &RepoBundle{
		Files: []GeneratedFile{
			{Path: chipPath, Description: "Chip descriptor starter with required placeholder peripherals.", Content: chipYAML},
			{Path: systemPath, Description: "Board system manifest starter wired to console and board IO placeholders.", Content: systemYAML},
			{Path: fmt.Sprintf("%s/README.md", exampleDir), Description: "Board example README starter.", Content: readme},
			{Path: fmt.Sprintf("%s/system.yaml", exampleDir), Description: "Local reproducible system manifest copy.", Content: systemCopy},
			{Path: fmt.Sprintf("%s/uart-smoke.yaml", exampleDir), Description: "Deterministic UART smoke test script.", Content: uartSmoke},
			{Path: fmt.Sprintf("%s/REQUIRED_DOCS.md", exampleDir), Description: "Source checklist for onboarding evidence.", Content: requiredDocs},
			{Path: fmt.Sprintf("%s/EXTERNAL_COMPONENTS.md", exampleDir), Description: "External component and deferred-scope notes.", Content: externalComponents},
			{Path: fmt.Sprintf("%s/VALIDATION.md", exampleDir), Description: "Validation command template.", Content: validation},
			{Path: fmt.Sprintf("%s/Cargo.toml", firmwareDir), Description: "Standalone smoke firmware crate manifest.", Content: smokeCargoToml},
			{Path: fmt.Sprintf("%s/build.rs", firmwareDir), Description: "Smoke firmware linker script copier.", Content: smokeBuildRS},
			{Path: fmt.Sprintf("%s/memory.x", firmwareDir), Description: "Smoke firmware memory map.", Content: smokeMemory},
			{Path: fmt.Sprintf("%s/minimal.ld", firmwareDir), Description: "Smoke firmware minimal linker script.", Content: smokeLinker},
			{Path: fmt.Sprintf("%s/src/main.rs", firmwareDir), Description: "Smoke firmware main entrypoint.", Content: smokeMain},
		},
	}
}

func recommendedExamples(req Request, resolution boardResolution) []ReferenceCandidate {
	if len(resolution.Profile.PreferredExamples) > 0 {
		return append([]ReferenceCandidate(nil), resolution.Profile.PreferredExamples...)
	}
	examples := []ReferenceCandidate{}
	lower := strings.ToLower(req.ComponentName + " " + req.Requirements + " " + strings.Join(req.DesiredCapabilities, " "))
	if req.Workload != nil && strings.TrimSpace(req.Workload.Example) != "" {
		examples = append(examples, ReferenceCandidate{
			Name:   strings.TrimSpace(req.Workload.Example),
			Reason: "Caller-requested workload reference for the onboarding proof.",
		})
	}
	if strings.Contains(lower, "wba") {
		examples = append(examples, ReferenceCandidate{
			Name:   "STM32CubeWBA BLE_p2pServer",
			Reason: "Board-level BLE starter aligned with STM32WBA Nucleo bring-up.",
		})
	}
	if strings.Contains(lower, "wb55") || strings.Contains(lower, "stm32wb") || strings.Contains(lower, "ble") {
		examples = append(examples,
			ReferenceCandidate{Name: "STM32CubeWB BLE_p2pServer", Reason: "Good proof target for NUCLEO-WB55RG board IO plus BLE-facing application flow."},
			ReferenceCandidate{Name: "STM32CubeWB BLE_LLD_Pressbutton", Reason: "Smaller board-level LED/button proof when radio fidelity is not the first milestone."},
		)
	}
	return dedupeExamples(examples)
}

func dedupeExamples(examples []ReferenceCandidate) []ReferenceCandidate {
	seen := map[string]bool{}
	out := make([]ReferenceCandidate, 0, len(examples))
	for _, example := range examples {
		name := strings.TrimSpace(example.Name)
		if name == "" || seen[name] {
			continue
		}
		seen[name] = true
		out = append(out, example)
	}
	return out
}

func renderRecommendedExamples(examples []ReferenceCandidate) string {
	if len(examples) == 0 {
		return "- No vendor examples resolved."
	}
	lines := make([]string, 0, len(examples))
	for _, example := range examples {
		line := "- " + example.Name
		if strings.TrimSpace(example.Reason) != "" {
			line += ": " + example.Reason
		}
		lines = append(lines, line)
	}
	return strings.Join(lines, "\n")
}

func renderChipPeripherals(profile boardProfile, hasProfile bool, uartID string, uartBase string, uartIRQ string) string {
	lines := []string{}
	if base := strings.TrimSpace(profile.RCCBase); base != "" {
		lines = append(lines,
			`  - id: "rcc"`,
			`    type: "rcc"`,
			fmt.Sprintf("    base_address: %s", base),
			`    config:`,
			`      profile: "stm32v2"`,
		)
	}
	gpioProfile := strings.HasPrefix(strings.ToLower(profile.Family), "stm32") || strings.HasPrefix(strings.ToLower(profile.Family), "gd32")
	for _, gpio := range boardGPIOPeripheralEntries(profile, hasProfile) {
		entry := []string{
			fmt.Sprintf(`  - id: "%s"`, gpio.ID),
			`    type: "gpio"`,
			fmt.Sprintf("    base_address: %s", gpio.BaseAddress),
			`    size: "1KB"`,
		}
		if gpioProfile {
			entry = append(entry, `    config:`, `      profile: "stm32v2"`)
		}
		lines = append(lines, entry...)
	}
	uartEntry := []string{
		`  - id: "systick"`,
		`    type: "systick"`,
		`    base_address: 0xE000E010`,
		fmt.Sprintf(`  - id: "%s"`, uartID),
		`    type: "uart"`,
		fmt.Sprintf("    base_address: %s", uartBase),
		fmt.Sprintf("    irq: %s", uartIRQ),
	}
	if uartProfile := boardUARTProfile(profile); uartProfile != "" {
		uartEntry = append(uartEntry, `    config:`, fmt.Sprintf(`      profile: "%s"`, uartProfile))
	}
	lines = append(lines, uartEntry...)
	return strings.Join(lines, "\n")
}

type gpioPeripheralEntry struct {
	ID          string
	BaseAddress string
}

func boardGPIOPeripheralEntries(profile boardProfile, hasProfile bool) []gpioPeripheralEntry {
	if !hasProfile {
		return []gpioPeripheralEntry{{ID: "gpioa", BaseAddress: "TODO"}}
	}
	entries := []gpioPeripheralEntry{}
	seen := map[string]bool{}
	add := func(id string, base string) {
		id = strings.TrimSpace(strings.ToLower(id))
		if id == "" || seen[id] {
			return
		}
		base = strings.TrimSpace(base)
		if base == "" {
			return
		}
		for _, entry := range entries {
			if entry.BaseAddress == base {
				return
			}
		}
		seen[id] = true
		entries = append(entries, gpioPeripheralEntry{ID: id, BaseAddress: base})
	}
	add("gpioa", profile.GPIOABase)
	add("gpiob", profile.GPIOBBase)
	add("gpioc", profile.GPIOCBase)
	add("gpiod", profile.GPIODBase)
	add("gpioh", profile.GPIOHBase)
	fallbackBase := firstNonEmpty(profile.GPIOABase, profile.GPIOBBase, profile.GPIOCBase, profile.GPIODBase, profile.GPIOHBase)
	for _, item := range append(append([]boardGPIO{}, profile.LEDs...), profile.Buttons...) {
		portID := boardPortPeripheralID(item.Port)
		if portID != "" {
			add(portID, fallbackBase)
		}
	}
	if len(entries) == 0 {
		entries = append(entries, gpioPeripheralEntry{ID: "gpioa", BaseAddress: "TODO"})
	}
	sort.Slice(entries, func(i, j int) bool { return entries[i].ID < entries[j].ID })
	return entries
}

func boardPortPeripheralID(port string) string {
	port = strings.ToUpper(strings.TrimSpace(port))
	if port == "GPIO" {
		return "gpio"
	}
	if !strings.HasPrefix(port, "GPIO") || len(port) != 5 {
		return ""
	}
	return strings.ToLower(port)
}

func firstNonEmpty(values ...string) string {
	for _, value := range values {
		if trimmed := strings.TrimSpace(value); trimmed != "" {
			return trimmed
		}
	}
	return ""
}

func boardRustTarget(profile boardProfile, hasProfile bool) string {
	return pick(profile.RustTarget, hasProfile, "thumbv7em-none-eabi")
}

func boardArch(profile boardProfile, hasProfile bool) string {
	return pick(profile.Arch, hasProfile, "arm")
}

func boardFlashBase(profile boardProfile, hasProfile bool) string {
	return pick(profile.FlashBase, hasProfile, "0x08000000")
}

func boardRAMBase(profile boardProfile, hasProfile bool) string {
	return pick(profile.RAMBase, hasProfile, "0x20000000")
}

func boardStackTop(profile boardProfile, hasProfile bool) string {
	return pick(profile.StackTop, hasProfile, "0x20040000")
}

func boardVendorExamplesPackage(profile boardProfile, req Request) string {
	if strings.TrimSpace(profile.VendorExamplesPkg) != "" {
		return profile.VendorExamplesPkg
	}
	lower := strings.ToLower(req.ComponentName + " " + req.Requirements)
	if req.Board != nil {
		lower += " " + strings.ToLower(req.Board.MCU+" "+req.Board.MarketingName+" "+req.Board.BoardID)
	}
	if req.Workload != nil {
		lower += " " + strings.ToLower(req.Workload.Example)
	}
	switch {
	case strings.Contains(lower, "cubewba") || strings.Contains(lower, "stm32wba"):
		return "STM32CubeWBA"
	case strings.Contains(lower, "cubeg4") || strings.Contains(lower, "stm32g4") || strings.Contains(lower, "g474"):
		return "STM32CubeG4"
	case strings.Contains(lower, "cubel4") || strings.Contains(lower, "stm32l4") || strings.Contains(lower, "l476"):
		return "STM32CubeL4"
	case strings.Contains(lower, "cubef4") || strings.Contains(lower, "stm32f4") || strings.Contains(lower, "f429"):
		return "STM32CubeF4"
	default:
		return "STM32CubeWB"
	}
}

func boardDefaultPort(profile boardProfile, hasProfile bool) string {
	for _, entry := range boardGPIOPeripheralEntries(profile, hasProfile) {
		return entry.ID
	}
	return "gpioa"
}

func boardScopeLine(req Request, profile boardProfile) string {
	if boardHasWirelessScope(req, profile, "") {
		return "- BLE example selection reference only"
	}
	if strings.Contains(strings.ToLower(profile.VendorExamplesPkg), "cube") {
		return "- Vendor example selection reference only"
	}
	return "- Deferred subsystem scope is declared in the generated docs"
}

func boardConfirmationLine(req Request, profile boardProfile, chipGuess string) string {
	if boardHasWirelessScope(req, profile, chipGuess) {
		return "- Confirm whether BLE scope is documentation-only or full simulator behavior"
	}
	return "- Confirm deferred subsystem scope matches the requested onboarding contract"
}

func boardHasWirelessScope(req Request, profile boardProfile, chipGuess string) bool {
	lower := strings.ToLower(strings.TrimSpace(req.Requirements))
	if req.Constraints != nil && strings.TrimSpace(req.Constraints.BLEScope) != "" {
		return true
	}
	if strings.Contains(lower, " ble") || strings.HasPrefix(lower, "ble") || strings.Contains(lower, "bluetooth") || strings.Contains(lower, "radio") || strings.Contains(lower, "802.15.4") {
		return true
	}
	if req.Workload != nil {
		workload := strings.ToLower(req.Workload.Type + " " + req.Workload.Example)
		if strings.Contains(workload, "ble") || strings.Contains(workload, "bluetooth") || strings.Contains(workload, "802.15.4") {
			return true
		}
	}
	identity := chipGuess + " " + profile.VendorExamplesPkg
	if req.Board != nil {
		identity += " " + req.Board.MarketingName + " " + req.Board.BoardID + " " + req.Board.MCU
	}
	identity = strings.ToLower(identity)
	wirelessHints := []string{"stm32wb", "stm32wba", "nrf52", "nrf528", "nano 33 ble", "ble", "bluetooth", "efr32bg", "bg22", "802.15.4"}
	for _, hint := range wirelessHints {
		if strings.Contains(identity, hint) {
			return true
		}
	}
	return false
}

func linkerSize(size string) string {
	normalized := strings.ToUpper(strings.TrimSpace(size))
	normalized = strings.ReplaceAll(normalized, "IB", "I")
	normalized = strings.ReplaceAll(normalized, "KB", "K")
	normalized = strings.ReplaceAll(normalized, "MB", "M")
	return normalized
}

func renderSmokeCargoToml(firmwarePackage string, arch string) string {
	deps := "[dependencies]\npanic-halt = \"0.2\"\n"
	if arch == "riscv" {
		deps += "riscv = \"0.10\"\nriscv-rt = \"0.12\"\n"
	}
	return fmt.Sprintf(`[workspace]
members = []

[package]
name = "%s"
version = "0.1.0"
edition = "2021"

%s
[[bin]]
name = "%s"
path = "src/main.rs"
test = false
bench = false
`, firmwarePackage, deps, firmwarePackage)
}

func renderSmokeBuildRS(arch string) string {
	if arch == "riscv" {
		return `use std::env;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

fn main() {
    let out = &PathBuf::from(env::var_os("OUT_DIR").unwrap());
    File::create(out.join("memory.x"))
        .unwrap()
        .write_all(include_bytes!("memory.x"))
        .unwrap();
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rustc-link-arg=-Tmemory.x");
    println!("cargo:rustc-link-arg=-Tlink.x");
    println!("cargo:rerun-if-changed=memory.x");
}
`
	}
	return `use std::env;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

fn main() {
    let out = &PathBuf::from(env::var_os("OUT_DIR").unwrap());
    File::create(out.join("memory.x"))
        .unwrap()
        .write_all(include_bytes!("memory.x"))
        .unwrap();
    File::create(out.join("minimal.ld"))
        .unwrap()
        .write_all(include_bytes!("minimal.ld"))
        .unwrap();
    File::create(out.join("link.x"))
        .unwrap()
        .write_all(b"/* generated placeholder; minimal.ld provides the actual linker script */\n")
        .unwrap();
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rustc-link-arg=-Tminimal.ld");
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rerun-if-changed=minimal.ld");
}
`
}

func renderSmokeMemory(profile boardProfile, hasProfile bool, flashBase string, ramBase string) string {
	if boardArch(profile, hasProfile) == "riscv" {
		return fmt.Sprintf(`MEMORY
{
  FLASH : ORIGIN = %s, LENGTH = %s
  RAM : ORIGIN = %s, LENGTH = %s
}

REGION_ALIAS("REGION_TEXT", FLASH);
REGION_ALIAS("REGION_RODATA", FLASH);
REGION_ALIAS("REGION_DATA", RAM);
REGION_ALIAS("REGION_BSS", RAM);
REGION_ALIAS("REGION_HEAP", RAM);
REGION_ALIAS("REGION_STACK", RAM);
_max_hart_id = 0;
_hart_stack_size = 512;
`, flashBase, linkerSize(pick(profile.FlashSize, hasProfile, "512K")), ramBase, linkerSize(pick(profile.RAMSize, hasProfile, "256K")))
	}
	return fmt.Sprintf(`MEMORY
{
  FLASH : ORIGIN = %s, LENGTH = %s
  RAM : ORIGIN = %s, LENGTH = %s
}
`, flashBase, linkerSize(pick(profile.FlashSize, hasProfile, "512K")), ramBase, linkerSize(pick(profile.RAMSize, hasProfile, "256K")))
}

func renderSmokeLinker(profile boardProfile, hasProfile bool, arch string, flashBase string, ramBase string, stackTop string) string {
	if arch == "riscv" {
		return `/* RISC-V uses riscv-rt's built-in link.x; memory.x provides the memory layout. */` + "\n"
	}
	return fmt.Sprintf(`MEMORY
{
  FLASH : ORIGIN = %s, LENGTH = %s
  RAM : ORIGIN = %s, LENGTH = %s
}

ENTRY(Reset)

SECTIONS
{
  .vector_table %s :
  {
    LONG(%s);
    LONG(Reset + 1);
  } > FLASH

  .text :
  {
    *(.text*)
    *(.rodata*)
  } > FLASH

  /DISCARD/ :
  {
    *(.ARM.exidx*)
    *(.note.gnu.build-id*)
  }
}
`, flashBase, linkerSize(pick(profile.FlashSize, hasProfile, "512K")), ramBase, linkerSize(pick(profile.RAMSize, hasProfile, "256K")), flashBase, stackTop)
}

func renderSmokeMain(arch string, uartID string, uartBase string) string {
	txOffset := boardUARTTxOffset(arch, uartID)
	if arch == "riscv" {
		return fmt.Sprintf(`#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;
use riscv_rt::entry;

const %s_TDR_PTR: *mut u8 = (%s + %s) as *mut u8;

#[entry]
fn main() -> ! {
    unsafe {
        core::ptr::write_volatile(%s_TDR_PTR, b'O');
        core::ptr::write_volatile(%s_TDR_PTR, b'K');
        core::ptr::write_volatile(%s_TDR_PTR, b'\n');
    }

    loop {}
}
`, strings.ToUpper(uartID), uartBase, txOffset, strings.ToUpper(uartID), strings.ToUpper(uartID), strings.ToUpper(uartID))
	}
	return fmt.Sprintf(`#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

const %s_TDR_PTR: *mut u8 = (%s + %s) as *mut u8;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn main() -> ! {
    unsafe {
        core::ptr::write_volatile(%s_TDR_PTR, b'O');
        core::ptr::write_volatile(%s_TDR_PTR, b'K');
        core::ptr::write_volatile(%s_TDR_PTR, b'\n');
    }

    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
`, strings.ToUpper(uartID), uartBase, txOffset, strings.ToUpper(uartID), strings.ToUpper(uartID), strings.ToUpper(uartID))
}

func boardUARTProfile(profile boardProfile) string {
	family := strings.ToLower(strings.TrimSpace(profile.Family))
	switch {
	case strings.HasPrefix(family, "nrf52"):
		return "nrf52"
	case family == "generic-rv32i", family == "ch32v", family == "fe310", family == "esp32c3":
		return "stm32f1"
	case family != "":
		return "stm32v2"
	default:
		return "stm32v2"
	}
}

func boardUARTTxOffset(arch string, uartID string) string {
	switch {
	case strings.Contains(strings.ToLower(strings.TrimSpace(uartID)), "uarte"):
		return "0x51C"
	case arch == "riscv":
		return "0x04"
	default:
		return "0x28"
	}
}

func boardSmokeMaxSteps(arch string) int {
	if arch == "riscv" {
		return 4096
	}
	return 64
}

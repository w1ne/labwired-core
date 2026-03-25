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

	return &BoardDraft{
		BoardID:               boardID,
		ChipGuess:             chipGuess,
		RequestedCapabilities: append([]string(nil), requestedBoardCapabilities(req)...),
		ValidatedCapabilities: append([]string(nil), validatedBoardCapabilities(req)...),
		ValidationTargets:     append([]string(nil), req.ValidationTargets...),
		BringupScope:          []string{"app-core boot", "rcc", "gpio", "uart console", "board LEDs", "user button"},
		DeferredScope: []string{
			"wireless coprocessor / CPU2 behavior",
			"IPCC/HCI transport",
			"radio timing fidelity",
		},
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
	return &ContractResult{
		RequestKind:           pick(req.Kind, req.Kind != "", "board_onboarding"),
		RequestedCapabilities: append([]string(nil), draft.RequestedCapabilities...),
		ValidatedCapabilities: append([]string(nil), draft.ValidatedCapabilities...),
		DeferredCapabilities: []string{
			"wireless_coprocessor",
			"ipcc_hci_transport",
			"radio_timing_fidelity",
		},
		MissingCapabilities: missingBoardCapabilities(req),
		ValidationTargets:   append([]string(nil), draft.ValidationTargets...),
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

	chipYAML := fmt.Sprintf(`schema_version: "1.0"
name: "%s"
arch: "arm"
flash:
  base: 0x08000000
  size: "%s"
ram:
  base: 0x20000000
  size: "%s"

peripherals:
%s
`, chipName, pick(profile.FlashSize, hasProfile, "TODO"), pick(profile.RAMSize, hasProfile, "TODO"), renderChipPeripherals(profile, hasProfile, uartID, uartBase, uartIRQ))

	systemYAML := fmt.Sprintf(`schema_version: "1.0"
name: "%s"
chip: "../chips/%s.yaml"

board_io:
%s%s
`, draft.BoardID, chipFile, renderBoardIOList(profile.LEDs, hasProfile, "led_user", defaultBoardPort, "led"), renderBoardIOList(profile.Buttons, hasProfile, "button_user", defaultBoardPort, "button"))

	readme := fmt.Sprintf(`# %s

This example was synthesized by Foundry as a board onboarding starter.

## Scope

- App-core boot path
- RCC/GPIO/UART baseline
- Board LED and user button mapping
- BLE example selection reference only

## Recommended Vendor Example

Reference package: %s

%s

## Required Source Confirmation

- Confirm MCU part number and package
- Confirm VCP UART instance on ST-LINK
- Confirm LED and button GPIO mappings from schematic
- Confirm whether BLE scope is documentation-only or full simulator behavior

## Auto-Resolved Source Docs

%s
`, draft.BoardID, vendorPackage, recommended, docsSection)

	requiredDocs := fmt.Sprintf(`# Required Docs

%s
`, docsSection)

	externalComponents := `# External Components

- ST-LINK virtual COM port connection
- On-board user LEDs
- On-board user button
- BLE/radio subsystem is deferred unless engine support is explicitly added
`

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
`, draft.BoardID, chipFile, renderBoardIOList(profile.LEDs, hasProfile, "led_user", defaultBoardPort, "led"), renderBoardIOList(profile.Buttons, hasProfile, "button_user", defaultBoardPort, "button"))

	smokeCargoToml := fmt.Sprintf(`[workspace]
members = []

[package]
name = "%s"
version = "0.1.0"
edition = "2021"

[dependencies]
panic-halt = "0.2"

[[bin]]
name = "%s"
path = "src/main.rs"
test = false
bench = false
`, firmwarePackage, firmwarePackage)

	smokeBuildRS := `use std::env;
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
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rustc-link-arg=-Tminimal.ld");
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rerun-if-changed=minimal.ld");
}
`

	smokeMemory := fmt.Sprintf(`MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = %s
  RAM : ORIGIN = 0x20000000, LENGTH = %s
}
`, linkerSize(pick(profile.FlashSize, hasProfile, "512K")), linkerSize(pick(profile.RAMSize, hasProfile, "256K")))

	smokeLinker := fmt.Sprintf(`MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = %s
  RAM : ORIGIN = 0x20000000, LENGTH = %s
}

ENTRY(Reset)

SECTIONS
{
  .vector_table 0x08000000 :
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
`, linkerSize(pick(profile.FlashSize, hasProfile, "512K")), linkerSize(pick(profile.RAMSize, hasProfile, "256K")), stackTop)

	smokeMain := fmt.Sprintf(`#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

const %s_TDR_PTR: *mut u8 = (%s + 0x28) as *mut u8;

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
`, strings.ToUpper(uartID), uartBase, strings.ToUpper(uartID), strings.ToUpper(uartID), strings.ToUpper(uartID))

	uartSmoke := fmt.Sprintf(`# LabWired - %s UART smoke test
schema_version: "1.0"
inputs:
  firmware: "./board_firmware/target/%s/release/%s"
  system: "./system.yaml"
limits:
  max_steps: 64
assertions:
  - uart_contains: "OK"
  - expected_stop_reason: max_steps
`, strings.ToUpper(draft.BoardID), rustTarget, firmwarePackage)

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
	lines := []string{
		`  - id: "rcc"`,
		`    type: "rcc"`,
		fmt.Sprintf("    base_address: %s", pick(profile.RCCBase, hasProfile, "TODO")),
		`    config:`,
		`      profile: "stm32v2"`,
	}
	for _, gpio := range boardGPIOPeripheralEntries(profile, hasProfile) {
		lines = append(lines,
			fmt.Sprintf(`  - id: "%s"`, gpio.ID),
			`    type: "gpio"`,
			fmt.Sprintf("    base_address: %s", gpio.BaseAddress),
			`    size: "1KB"`,
			`    config:`,
			`      profile: "stm32v2"`,
		)
	}
	lines = append(lines,
		`  - id: "systick"`,
		`    type: "systick"`,
		`    base_address: 0xE000E010`,
		fmt.Sprintf(`  - id: "%s"`, uartID),
		`    type: "uart"`,
		fmt.Sprintf("    base_address: %s", uartBase),
		fmt.Sprintf("    irq: %s", uartIRQ),
		`    config:`,
		`      profile: "stm32v2"`,
	)
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
	add := func(id string, base string) {
		base = strings.TrimSpace(base)
		if base == "" {
			return
		}
		entries = append(entries, gpioPeripheralEntry{ID: id, BaseAddress: base})
	}
	add("gpioa", profile.GPIOABase)
	add("gpiob", profile.GPIOBBase)
	add("gpioc", profile.GPIOCBase)
	add("gpiod", profile.GPIODBase)
	add("gpioh", profile.GPIOHBase)
	if len(entries) == 0 {
		entries = append(entries, gpioPeripheralEntry{ID: "gpioa", BaseAddress: "TODO"})
	}
	sort.Slice(entries, func(i, j int) bool { return entries[i].ID < entries[j].ID })
	return entries
}

func boardRustTarget(profile boardProfile, hasProfile bool) string {
	return pick(profile.RustTarget, hasProfile, "thumbv7em-none-eabi")
}

func boardStackTop(profile boardProfile, hasProfile bool) string {
	return pick(profile.StackTop, hasProfile, "0x20040000")
}

func boardVendorExamplesPackage(profile boardProfile, req Request) string {
	if strings.TrimSpace(profile.VendorExamplesPkg) != "" {
		return profile.VendorExamplesPkg
	}
	if req.Workload != nil && strings.Contains(strings.ToLower(req.Workload.Example), "cubewba") {
		return "STM32CubeWBA"
	}
	return "STM32CubeWB"
}

func boardDefaultPort(profile boardProfile, hasProfile bool) string {
	for _, entry := range boardGPIOPeripheralEntries(profile, hasProfile) {
		return entry.ID
	}
	return "gpioa"
}

func linkerSize(size string) string {
	normalized := strings.ToUpper(strings.TrimSpace(size))
	normalized = strings.ReplaceAll(normalized, "IB", "I")
	normalized = strings.ReplaceAll(normalized, "KB", "K")
	normalized = strings.ReplaceAll(normalized, "MB", "M")
	return normalized
}

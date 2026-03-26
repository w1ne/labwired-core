package synthesis

import (
	"context"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestGenerateArtifact_BoardOnboardingProducesInspectableContract(t *testing.T) {
	req := Request{
		Kind:          "board_onboarding",
		ComponentName: "MB1355C / NUCLEO-WB55RG board onboarding proof",
		Board: &BoardSpec{
			Vendor:        "ST",
			MarketingName: "NUCLEO-WB55RG",
			BoardID:       "MB1355C",
			MCU:           "STM32WB55RG",
		},
		DesiredCapabilities: []string{"boot", "uart_console", "led_control", "button_input"},
		ValidationTargets:   []string{"uart_smoke", "io_smoke", "unsupported_instruction_audit"},
		Constraints: &ConstraintSpec{
			MustWriteRepoAssets: false,
		},
	}

	artifact, err := GenerateArtifact(context.Background(), req)
	if err != nil {
		t.Fatalf("GenerateArtifact failed: %v", err)
	}
	if artifact.ArtifactType != "board_onboarding_draft" {
		t.Fatalf("unexpected artifact type: %s", artifact.ArtifactType)
	}
	if artifact.ContractResult == nil {
		t.Fatal("expected contract_result")
	}
	if artifact.ContractResult.RequestKind != "board_onboarding" {
		t.Fatalf("unexpected request kind: %s", artifact.ContractResult.RequestKind)
	}
	if artifact.ContractResult.PromotionMode != "artifact_only" {
		t.Fatalf("unexpected promotion mode: %s", artifact.ContractResult.PromotionMode)
	}
	if artifact.BoardFacts == nil || len(artifact.BoardFacts.DerivedFacts) == 0 {
		t.Fatalf("expected board_facts with derived facts, got %+v", artifact.BoardFacts)
	}
	if len(artifact.ContractResult.ValidatedCapabilities) != 4 {
		t.Fatalf("expected 4 validated capabilities, got %d", len(artifact.ContractResult.ValidatedCapabilities))
	}
	if artifact.BoardDraft == nil || artifact.BoardDraft.BoardID != "mb1355c" {
		t.Fatalf("unexpected board draft: %+v", artifact.BoardDraft)
	}
	if artifact.RepoBundle == nil {
		t.Fatal("expected repo bundle")
	}

	paths := map[string]string{}
	for _, file := range artifact.RepoBundle.Files {
		paths[file.Path] = file.Content
	}
	required := []string{
		"core/configs/chips/stm32wb55.yaml",
		"core/configs/systems/mb1355c.yaml",
		"core/examples/mb1355c/system.yaml",
		"core/examples/mb1355c/VALIDATION.md",
	}
	for _, path := range required {
		if _, ok := paths[path]; !ok {
			t.Fatalf("missing generated file %s", path)
		}
	}
	systemManifest := paths["core/configs/systems/mb1355c.yaml"]
	if !strings.Contains(systemManifest, "led_blue") || !strings.Contains(systemManifest, "button_sw1") {
		t.Fatalf("expected board system manifest to contain LED and button mappings, got: %s", systemManifest)
	}
	if !strings.Contains(paths["core/examples/mb1355c/VALIDATION.md"], "unsupported_instruction_audit.sh") {
		t.Fatalf("expected validation doc to include audit command, got: %s", paths["core/examples/mb1355c/VALIDATION.md"])
	}

	assertions, err := ValidateArtifact(artifact)
	if err != nil {
		t.Fatalf("ValidateArtifact failed: %v", err)
	}
	if assertions != 5 {
		t.Fatalf("unexpected assertion count: %d", assertions)
	}
}

func TestGenerateArtifact_PeripheralModelIngestProducesStrictIR(t *testing.T) {
	req := Request{
		Kind:          "peripheral_model_ingest",
		ComponentName: "ADXL345",
		Requirements:  "I2C interface required. Register 0x00 should return Device ID 0xE5.",
		DatasheetURL:  "https://www.analog.com/media/en/technical-documentation/data-sheets/ADXL345.pdf",
	}

	artifact, err := GenerateArtifact(context.Background(), req)
	if err != nil {
		t.Fatalf("GenerateArtifact failed: %v", err)
	}
	if artifact.ArtifactType != "strict_ir_draft" {
		t.Fatalf("unexpected artifact type: %s", artifact.ArtifactType)
	}
	if artifact.ModelDraft == nil || artifact.ModelDraft.StrictIRDraft == nil {
		t.Fatalf("expected strict IR draft, got %+v", artifact.ModelDraft)
	}
	if artifact.ContractResult == nil {
		t.Fatal("expected contract_result")
	}
	if artifact.ContractResult.RequestKind != "peripheral_model_ingest" {
		t.Fatalf("unexpected request kind: %s", artifact.ContractResult.RequestKind)
	}
	if len(artifact.ModelDraft.Registers) != 1 {
		t.Fatalf("expected 1 inferred register, got %d", len(artifact.ModelDraft.Registers))
	}
	reg := artifact.ModelDraft.Registers[0]
	if reg.Offset != "0x00" || reg.ResetValue != "0xe5" {
		t.Fatalf("unexpected inferred register: %+v", reg)
	}

	assertions, err := ValidateArtifact(artifact)
	if err != nil {
		t.Fatalf("ValidateArtifact failed: %v", err)
	}
	if assertions != 3 {
		t.Fatalf("unexpected assertion count: %d", assertions)
	}
}

func TestGenerateArtifact_UnknownBoardWithoutGroundedFactsFails(t *testing.T) {
	req := Request{
		Kind:          "board_onboarding",
		ComponentName: "ProtoSpark X9 bring-up",
		DatasheetURL:  "https://example.com/protospark-x9-mcu.pdf",
		DocumentationURLs: []string{
			"https://example.com/protospark-x9-board.pdf",
			"https://example.com/protospark-x9-schematic.pdf",
		},
		Board: &BoardSpec{
			Vendor:        "Acme",
			MarketingName: "ProtoSpark X9",
			BoardID:       "PSX9-REV-A",
			MCU:           "XMegaFoo123",
		},
		DesiredCapabilities: []string{"boot", "uart_console", "led_control"},
		ValidationTargets:   []string{"uart_smoke"},
	}

	artifact, err := GenerateArtifact(context.Background(), req)
	if err == nil {
		t.Fatalf("expected grounded-fact failure, got artifact %+v", artifact)
	}
	if !strings.Contains(err.Error(), "insufficient grounded facts") {
		t.Fatalf("unexpected error: %v", err)
	}
}

func TestGenerateArtifact_UnknownPeripheralStillProducesStableContract(t *testing.T) {
	req := Request{
		Kind:          "peripheral_model_ingest",
		ComponentName: "QSC9 Random Sensor",
		Requirements:  "SPI interface required. Temperature and status registers must be readable after reset.",
	}

	artifact, err := GenerateArtifact(context.Background(), req)
	if err != nil {
		t.Fatalf("GenerateArtifact failed: %v", err)
	}
	if artifact.ArtifactType != "strict_ir_draft" {
		t.Fatalf("unexpected artifact type: %s", artifact.ArtifactType)
	}
	if artifact.ContractResult == nil {
		t.Fatal("expected contract_result")
	}
	if artifact.ContractResult.RequestKind != "peripheral_model_ingest" {
		t.Fatalf("unexpected request kind: %s", artifact.ContractResult.RequestKind)
	}
	if len(artifact.ModelDraft.Registers) < 2 {
		t.Fatalf("expected fallback register inference, got %+v", artifact.ModelDraft.Registers)
	}
	if len(artifact.ModelDraft.StrictIRDraft.Peripherals) == 0 {
		t.Fatal("expected strict IR peripheral entries")
	}
}

func TestGenerateArtifact_ExtractsBoardFactsFromLocalDocs(t *testing.T) {
	dir := t.TempDir()
	datasheetPath := filepath.Join(dir, "sparkfun-x1-datasheet.pdf")
	boardDocPath := filepath.Join(dir, "sparkfun-x1-board.pdf")
	schematicPath := filepath.Join(dir, "sparkfun-x1-schematic.pdf")
	referencePath := filepath.Join(dir, "sparkfun-x1-reference-manual.pdf")

	if err := os.WriteFile(datasheetPath, []byte(`
MCU STM32F411RE
FLASH 512KB
RAM 128KB
RCC 0x40023800
GPIOA 0x40020000
GPIOB 0x40020400
GPIOC 0x40020800
USART2 0x40004400 IRQ 38
TX GPIOA 2
RX GPIOA 3
`), 0o644); err != nil {
		t.Fatalf("WriteFile datasheet failed: %v", err)
	}
	if err := os.WriteFile(boardDocPath, []byte(`
led_status GPIOC 13 active_high
button_user GPIOA 0 active_low
`), 0o644); err != nil {
		t.Fatalf("WriteFile board doc failed: %v", err)
	}
	if err := os.WriteFile(schematicPath, []byte(`
board SparkFun X1 RevA
`), 0o644); err != nil {
		t.Fatalf("WriteFile schematic failed: %v", err)
	}
	if err := os.WriteFile(referencePath, []byte(`
reference manual STM32F411RE
`), 0o644); err != nil {
		t.Fatalf("WriteFile reference failed: %v", err)
	}

	req := Request{
		Kind:          "board_onboarding",
		ComponentName: "SparkFun X1 board bring-up",
		DatasheetURL:  datasheetPath,
		DocumentationURLs: []string{
			boardDocPath,
			schematicPath,
			referencePath,
		},
		Board: &BoardSpec{
			Vendor:        "SparkFun",
			MarketingName: "X1",
			BoardID:       "sparkfun-x1-reva",
			MCU:           "STM32F411RE",
		},
		DesiredCapabilities: []string{"boot", "uart_console", "led_control", "button_input"},
		ValidationTargets:   []string{"uart_smoke", "io_smoke"},
	}

	artifact, err := GenerateArtifact(context.Background(), req)
	if err != nil {
		t.Fatalf("GenerateArtifact failed: %v", err)
	}
	if artifact.BoardDraft == nil {
		t.Fatal("expected board draft")
	}
	if artifact.BoardDraft.ChipGuess != "stm32f411re" {
		t.Fatalf("unexpected chip guess: %s", artifact.BoardDraft.ChipGuess)
	}
	if artifact.BoardFacts == nil || len(artifact.BoardFacts.ExtractedFacts) < 4 {
		t.Fatalf("expected extracted board facts, got %+v", artifact.BoardFacts)
	}

	paths := map[string]string{}
	for _, file := range artifact.RepoBundle.Files {
		paths[file.Path] = file.Content
	}
	chipYAML := paths["core/configs/chips/stm32f411re.yaml"]
	if !strings.Contains(chipYAML, "size: \"512KB\"") || !strings.Contains(chipYAML, "size: \"128KB\"") {
		t.Fatalf("expected extracted flash/ram sizes, got: %s", chipYAML)
	}
	if !strings.Contains(chipYAML, "0x40023800") || !strings.Contains(chipYAML, "0x40004400") || !strings.Contains(chipYAML, "irq: 38") {
		t.Fatalf("expected extracted peripheral facts, got: %s", chipYAML)
	}
	systemYAML := paths["core/configs/systems/sparkfun_x1_reva.yaml"]
	if !strings.Contains(systemYAML, "led_status") || !strings.Contains(systemYAML, "button_user") {
		t.Fatalf("expected extracted LED/button mappings, got: %s", systemYAML)
	}
	openQuestions := strings.Join(artifact.BoardDraft.OpenQuestions, "\n")
	if strings.Contains(openQuestions, "Which UART instance") || strings.Contains(openQuestions, "Which exact LED GPIO pins") {
		t.Fatalf("expected doc extraction to resolve basic UART/LED questions, got: %s", openQuestions)
	}
}

func TestGenerateArtifact_ExtractsBoardFactsFromRemoteDocs(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/datasheet.txt":
			_, _ = w.Write([]byte("MCU STM32F411RE\nFLASH 512KB\nRAM 128KB\nRCC 0x40023800\nGPIOA 0x40020000\nGPIOB 0x40020400\nGPIOC 0x40020800\nUSART2 0x40004400 IRQ 38\nTX GPIOA 2\nRX GPIOA 3\n"))
		case "/board.txt":
			_, _ = w.Write([]byte("led_status GPIOC 13 active_high\nbutton_user GPIOA 0 active_low\n"))
		case "/schematic.txt":
			_, _ = w.Write([]byte("board SparkFun X1 RevA\n"))
		case "/reference.txt":
			_, _ = w.Write([]byte("reference manual STM32F411RE\n"))
		default:
			http.NotFound(w, r)
		}
	}))
	defer server.Close()

	req := Request{
		Kind:          "board_onboarding",
		ComponentName: "SparkFun X1 board bring-up",
		DatasheetURL:  server.URL + "/datasheet.txt",
		DocumentationURLs: []string{
			server.URL + "/board.txt",
			server.URL + "/schematic.txt",
			server.URL + "/reference.txt",
		},
		Board: &BoardSpec{
			Vendor:        "SparkFun",
			MarketingName: "X1",
			BoardID:       "sparkfun-x1-reva",
			MCU:           "STM32F411RE",
		},
		DesiredCapabilities: []string{"boot", "uart_console", "led_control", "button_input"},
		ValidationTargets:   []string{"uart_smoke", "io_smoke"},
	}

	artifact, err := GenerateArtifact(context.Background(), req)
	if err != nil {
		t.Fatalf("GenerateArtifact failed: %v", err)
	}
	if artifact.BoardDraft == nil || artifact.BoardDraft.ChipGuess != "stm32f411re" {
		t.Fatalf("expected remote docs to ground chip identity, got %+v", artifact.BoardDraft)
	}
	if artifact.BoardFacts == nil || len(artifact.BoardFacts.ExtractedFacts) < 4 {
		t.Fatalf("expected extracted board facts from remote docs, got %+v", artifact.BoardFacts)
	}
}

func TestGenerateArtifact_NRF52840DocsAreRecognized(t *testing.T) {
	dir := t.TempDir()
	datasheetPath := filepath.Join(dir, "nrf52840-datasheet.txt")
	boardPath := filepath.Join(dir, "nrf52840-board.txt")
	referencePath := filepath.Join(dir, "nrf52840-reference.txt")
	if err := os.WriteFile(datasheetPath, []byte("MCU NRF52840\nFLASH 1024KB\nRAM 256KB\nUARTE0 0x40002000 IRQ 2\nP0 0x50000000\n"), 0o644); err != nil {
		t.Fatalf("WriteFile datasheet failed: %v", err)
	}
	if err := os.WriteFile(boardPath, []byte("led_user P0 13 active_high\nbutton_user P0 11 active_low\n"), 0o644); err != nil {
		t.Fatalf("WriteFile board failed: %v", err)
	}
	if err := os.WriteFile(referencePath, []byte("reference manual NRF52840\n"), 0o644); err != nil {
		t.Fatalf("WriteFile reference failed: %v", err)
	}
	req := Request{
		Kind:          "board_onboarding",
		ComponentName: "nRF52840 DK onboarding",
		DatasheetURL:  datasheetPath,
		DocumentationURLs: []string{
			boardPath,
			referencePath,
		},
		Board: &BoardSpec{
			Vendor:        "Nordic",
			MarketingName: "PCA10056 DK",
			BoardID:       "nrf52840_dk",
			MCU:           "NRF52840",
		},
		DesiredCapabilities: []string{"boot", "uart_console", "led_control", "button_input"},
		ValidationTargets:   []string{"uart_smoke"},
	}

	artifact, err := GenerateArtifact(context.Background(), req)
	if err != nil {
		t.Fatalf("GenerateArtifact failed: %v", err)
	}
	if artifact.BoardDraft == nil || artifact.BoardDraft.ChipGuess != "nrf52840" {
		t.Fatalf("expected nrf chip guess, got %+v", artifact.BoardDraft)
	}
	if artifact.BoardFacts == nil || len(artifact.BoardFacts.ExtractedFacts) < 5 {
		t.Fatalf("expected extracted board facts, got %+v", artifact.BoardFacts)
	}
}

func TestGenerateArtifact_SAMD21DocsAreRecognized(t *testing.T) {
	dir := t.TempDir()
	datasheetPath := filepath.Join(dir, "samd21-datasheet.txt")
	boardPath := filepath.Join(dir, "samd21-board.txt")
	referencePath := filepath.Join(dir, "samd21-reference.txt")
	if err := os.WriteFile(datasheetPath, []byte("MCU SAMD21J18A\nFLASH 256KB\nRAM 32KB\nPORTA 0x41004400\nSERCOM3 USART 0x42000C00 IRQ 12\n"), 0o644); err != nil {
		t.Fatalf("WriteFile datasheet failed: %v", err)
	}
	if err := os.WriteFile(boardPath, []byte("led_user PA17 active_high\nbutton_user PA28 active_low\n"), 0o644); err != nil {
		t.Fatalf("WriteFile board failed: %v", err)
	}
	if err := os.WriteFile(referencePath, []byte("reference manual SAMD21J18A\n"), 0o644); err != nil {
		t.Fatalf("WriteFile reference failed: %v", err)
	}
	req := Request{
		Kind:          "board_onboarding",
		ComponentName: "ATSAMD21 Xplained onboarding",
		DatasheetURL:  datasheetPath,
		DocumentationURLs: []string{
			boardPath,
			referencePath,
		},
		Board: &BoardSpec{
			Vendor:        "Microchip",
			MarketingName: "ATSAMD21 Xplained Pro",
			BoardID:       "atsamd21_xplained",
			MCU:           "SAMD21J18A",
		},
		DesiredCapabilities: []string{"boot", "uart_console", "led_control", "button_input"},
		ValidationTargets:   []string{"uart_smoke"},
	}

	artifact, err := GenerateArtifact(context.Background(), req)
	if err != nil {
		t.Fatalf("GenerateArtifact failed: %v", err)
	}
	if artifact.BoardDraft == nil || artifact.BoardDraft.ChipGuess != "samd21j18a" {
		t.Fatalf("expected samd chip guess, got %+v", artifact.BoardDraft)
	}
	paths := map[string]string{}
	for _, file := range artifact.RepoBundle.Files {
		paths[file.Path] = file.Content
	}
	buildRS := paths["core/examples/atsamd21_xplained/board_firmware/build.rs"]
	if !strings.Contains(buildRS, "generated placeholder") {
		t.Fatalf("expected placeholder link.x generation, got: %s", buildRS)
	}
}

func TestInferChipGuessFromCorpus_AdditionalFamilies(t *testing.T) {
	cases := map[string]string{
		"MCU GD32F103CB": "gd32f103cb",
		"MCU CH32V003":   "ch32v003",
		"MCU SAME54P20A": "same54p20a",
		"MCU EFR32BG22":  "efr32bg22",
		"MCU FE310":      "fe310",
		"MCU ESP32C3":    "esp32c3",
		"MCU RA6M5":      "ra6m5",
	}
	for corpus, want := range cases {
		if got := inferChipGuessFromCorpus(corpus); got != want {
			t.Fatalf("inferChipGuessFromCorpus(%q) = %q, want %q", corpus, got, want)
		}
	}
}

func TestGenerateArtifact_EFR32SharedGPIOBaseMaterializesBoardPorts(t *testing.T) {
	dir := t.TempDir()
	datasheetPath := filepath.Join(dir, "efr32bg22-datasheet.txt")
	boardPath := filepath.Join(dir, "efr32bg22-board.txt")
	referencePath := filepath.Join(dir, "efr32bg22-reference.txt")
	if err := os.WriteFile(datasheetPath, []byte("MCU EFR32BG22\nFLASH 512KB\nRAM 32KB\nGPIO 0x4000A400\nUSART0 0x40010000 IRQ 13\n"), 0o644); err != nil {
		t.Fatalf("WriteFile datasheet failed: %v", err)
	}
	if err := os.WriteFile(boardPath, []byte("led_user PA4 active_high\nbutton_user PB1 active_low\n"), 0o644); err != nil {
		t.Fatalf("WriteFile board failed: %v", err)
	}
	if err := os.WriteFile(referencePath, []byte("reference manual EFR32BG22\n"), 0o644); err != nil {
		t.Fatalf("WriteFile reference failed: %v", err)
	}
	req := Request{
		Kind:          "board_onboarding",
		ComponentName: "EFR32BG22 DK onboarding",
		DatasheetURL:  datasheetPath,
		DocumentationURLs: []string{
			boardPath,
			referencePath,
		},
		Board: &BoardSpec{
			Vendor:        "Silicon Labs",
			MarketingName: "BRD4184",
			BoardID:       "efr32bg22_dk",
			MCU:           "EFR32BG22",
		},
		DesiredCapabilities: []string{"boot", "uart_console", "led_control", "button_input"},
		ValidationTargets:   []string{"uart_smoke"},
	}
	artifact, err := GenerateArtifact(context.Background(), req)
	if err != nil {
		t.Fatalf("GenerateArtifact failed: %v", err)
	}
	paths := map[string]string{}
	for _, file := range artifact.RepoBundle.Files {
		paths[file.Path] = file.Content
	}
	chipYAML := paths["core/configs/chips/efr32bg22.yaml"]
	systemYAML := paths["core/configs/systems/efr32bg22_dk.yaml"]
	if !strings.Contains(chipYAML, "id: \"gpioa\"") || strings.Contains(chipYAML, "id: \"gpiob\"") {
		t.Fatalf("expected one shared gpio controller, got: %s", chipYAML)
	}
	if !strings.Contains(systemYAML, "peripheral: \"gpioa\"") {
		t.Fatalf("expected shared gpio controller references, got: %s", systemYAML)
	}
}

func TestGenerateArtifact_RP2040SingleTokenGPIOIsRecognized(t *testing.T) {
	dir := t.TempDir()
	datasheetPath := filepath.Join(dir, "rp2040-datasheet.txt")
	boardPath := filepath.Join(dir, "rp2040-board.txt")
	referencePath := filepath.Join(dir, "rp2040-reference.txt")
	if err := os.WriteFile(datasheetPath, []byte("MCU RP2040\nFLASH 2048KB\nRAM 264KB\nUART0 0x40034000 IRQ 20\nSIO 0xD0000000\n"), 0o644); err != nil {
		t.Fatalf("WriteFile datasheet failed: %v", err)
	}
	if err := os.WriteFile(boardPath, []byte("led_user GPIO25 active_high\nbutton_user GPIO14 active_high\n"), 0o644); err != nil {
		t.Fatalf("WriteFile board failed: %v", err)
	}
	if err := os.WriteFile(referencePath, []byte("reference manual RP2040\n"), 0o644); err != nil {
		t.Fatalf("WriteFile reference failed: %v", err)
	}
	req := Request{
		Kind:          "board_onboarding",
		ComponentName: "RP2040 Pico onboarding",
		DatasheetURL:  datasheetPath,
		DocumentationURLs: []string{
			boardPath,
			referencePath,
		},
		Board: &BoardSpec{
			Vendor:        "Raspberry Pi",
			MarketingName: "Pico",
			BoardID:       "rp2040_pico",
			MCU:           "RP2040",
		},
		DesiredCapabilities: []string{"boot", "uart_console", "led_control", "button_input"},
		ValidationTargets:   []string{"uart_smoke"},
	}

	artifact, err := GenerateArtifact(context.Background(), req)
	if err != nil {
		t.Fatalf("GenerateArtifact failed: %v", err)
	}
	paths := map[string]string{}
	for _, file := range artifact.RepoBundle.Files {
		paths[file.Path] = file.Content
	}
	systemYAML := paths["core/configs/systems/rp2040_pico.yaml"]
	chipYAML := paths["core/configs/chips/rp2040.yaml"]
	if !strings.Contains(systemYAML, "led_user") || !strings.Contains(systemYAML, "button_user") {
		t.Fatalf("expected GPIO25/GPIO14 extraction, got: %s", systemYAML)
	}
	if !strings.Contains(chipYAML, "arch: \"arm\"") || !strings.Contains(chipYAML, "base: 0x10000000") {
		t.Fatalf("expected RP2040 flash base, got: %s", chipYAML)
	}
}

func TestGenerateArtifact_RiscVBoardsUseRiscVSmokeBundle(t *testing.T) {
	dir := t.TempDir()
	datasheetPath := filepath.Join(dir, "rv32i-datasheet.txt")
	boardPath := filepath.Join(dir, "rv32i-board.txt")
	referencePath := filepath.Join(dir, "rv32i-reference.txt")
	if err := os.WriteFile(datasheetPath, []byte("MCU GENERIC-RV32I\nFLASH 256KB\nRAM 64KB\nUART0 0x10013000 IRQ 3\nGPIO 0x10012000\n"), 0o644); err != nil {
		t.Fatalf("WriteFile datasheet failed: %v", err)
	}
	if err := os.WriteFile(boardPath, []byte("led_user GPIO0 5 active_high\nbutton_user GPIO0 6 active_high\n"), 0o644); err != nil {
		t.Fatalf("WriteFile board failed: %v", err)
	}
	if err := os.WriteFile(referencePath, []byte("reference manual GENERIC-RV32I\n"), 0o644); err != nil {
		t.Fatalf("WriteFile reference failed: %v", err)
	}
	req := Request{
		Kind:          "board_onboarding",
		ComponentName: "GENERIC-RV32I fixture onboarding",
		DatasheetURL:  datasheetPath,
		DocumentationURLs: []string{
			boardPath,
			referencePath,
		},
		Board: &BoardSpec{
			Vendor:        "LabWired",
			MarketingName: "RV32I Fixture",
			BoardID:       "generic_rv32i_fixture",
			MCU:           "GENERIC-RV32I",
		},
		DesiredCapabilities: []string{"boot", "uart_console", "led_control", "button_input"},
		ValidationTargets:   []string{"uart_smoke"},
	}

	artifact, err := GenerateArtifact(context.Background(), req)
	if err != nil {
		t.Fatalf("GenerateArtifact failed: %v", err)
	}
	paths := map[string]string{}
	for _, file := range artifact.RepoBundle.Files {
		paths[file.Path] = file.Content
	}
	chipYAML := paths["core/configs/chips/generic-rv32i.yaml"]
	cargoToml := paths["core/examples/generic_rv32i_fixture/board_firmware/Cargo.toml"]
	mainRS := paths["core/examples/generic_rv32i_fixture/board_firmware/src/main.rs"]
	minimalLD := paths["core/examples/generic_rv32i_fixture/board_firmware/minimal.ld"]
	if !strings.Contains(chipYAML, "arch: \"riscv\"") || !strings.Contains(chipYAML, "base: 0x80000000") {
		t.Fatalf("expected RISC-V chip emission, got: %s", chipYAML)
	}
	if !strings.Contains(chipYAML, "profile: \"stm32f1\"") || strings.Contains(chipYAML, "profile: \"stm32v2\"") {
		t.Fatalf("expected generic RISC-V UART layout, got: %s", chipYAML)
	}
	if !strings.Contains(cargoToml, "riscv-rt") {
		t.Fatalf("expected RISC-V smoke dependencies, got: %s", cargoToml)
	}
	if !strings.Contains(mainRS, "use riscv_rt::entry;") || !strings.Contains(mainRS, "(0x10013000 + 0x04) as *mut u8") {
		t.Fatalf("expected RISC-V smoke main, got: %s", mainRS)
	}
	if !strings.Contains(minimalLD, "RISC-V uses riscv-rt") {
		t.Fatalf("expected RISC-V linker placeholder, got: %s", minimalLD)
	}
}

func TestGenerateArtifact_WBA52BoardUsesFamilyAwareOutputs(t *testing.T) {
	dir := t.TempDir()
	datasheetPath := filepath.Join(dir, "stm32wba52cg-datasheet.pdf")
	boardDocPath := filepath.Join(dir, "nucleo-wba52cg-board.pdf")
	referencePath := filepath.Join(dir, "stm32wba52-reference.pdf")
	examplePath := filepath.Join(dir, "stm32cubewba-example.txt")

	if err := os.WriteFile(datasheetPath, []byte(`
MCU STM32WBA52CG
FLASH 1024KB
RAM 128KB
RCC 0x46020C00
GPIOA 0x42020000
GPIOB 0x42020400
GPIOC 0x42020800
GPIOH 0x42021C00
LPUART1 0x46002400 IRQ 45
TX GPIOA 2
RX GPIOA 3
`), 0o644); err != nil {
		t.Fatalf("WriteFile datasheet failed: %v", err)
	}
	if err := os.WriteFile(boardDocPath, []byte(`
led_blue GPIOB 4 active_high
led_green GPIOA 9 active_high
led_red GPIOB 8 active_high
button_b1 GPIOC 13 active_high
button_b2 GPIOB 6 active_high
button_b3 GPIOB 7 active_high
`), 0o644); err != nil {
		t.Fatalf("WriteFile board doc failed: %v", err)
	}
	if err := os.WriteFile(referencePath, []byte("STM32WBA reference manual\n"), 0o644); err != nil {
		t.Fatalf("WriteFile reference failed: %v", err)
	}
	if err := os.WriteFile(examplePath, []byte("STM32CubeWBA BLE_p2pServer\n"), 0o644); err != nil {
		t.Fatalf("WriteFile example failed: %v", err)
	}

	req := Request{
		Kind:          "board_onboarding",
		ComponentName: "NUCLEO-WBA52CG board onboarding proof",
		DatasheetURL:  datasheetPath,
		DocumentationURLs: []string{
			boardDocPath,
			referencePath,
			examplePath,
		},
		Board: &BoardSpec{
			Vendor:        "STMicroelectronics",
			MarketingName: "NUCLEO-WBA52CG",
			BoardID:       "nucleo_wba52cg",
			MCU:           "STM32WBA52CG",
		},
		DesiredCapabilities: []string{"boot", "uart_console", "led_control", "button_input"},
		ValidationTargets:   []string{"uart_smoke", "io_smoke", "unsupported_instruction_audit"},
		Workload:            &WorkloadSpec{Type: "generated_smoke_firmware", Example: "STM32CubeWBA BLE_p2pServer"},
		Constraints:         &ConstraintSpec{MustWriteRepoAssets: true, MustRunE2EValidation: true},
	}

	artifact, err := GenerateArtifact(context.Background(), req)
	if err != nil {
		t.Fatalf("GenerateArtifact failed: %v", err)
	}
	if artifact.BoardDraft == nil || artifact.BoardDraft.ChipGuess != "stm32wba52" {
		t.Fatalf("unexpected WBA board draft: %+v", artifact.BoardDraft)
	}
	if artifact.BoardFacts == nil || len(artifact.BoardFacts.DerivedFacts) == 0 {
		t.Fatalf("expected board facts for WBA board, got %+v", artifact.BoardFacts)
	}
	paths := map[string]string{}
	for _, file := range artifact.RepoBundle.Files {
		paths[file.Path] = file.Content
	}
	chipYAML := paths["core/configs/chips/stm32wba52.yaml"]
	if !strings.Contains(chipYAML, "0x46020C00") || !strings.Contains(chipYAML, "0x46002400") {
		t.Fatalf("expected WBA RCC/LPUART addresses, got: %s", chipYAML)
	}
	if strings.Contains(chipYAML, "gpiod") || !strings.Contains(chipYAML, "gpioh") {
		t.Fatalf("expected dynamic GPIO set for WBA, got: %s", chipYAML)
	}
	readme := paths["core/examples/nucleo_wba52cg/README.md"]
	if !strings.Contains(readme, "STM32CubeWBA") || strings.Contains(readme, "STM32CubeWB BLE_LLD_Pressbutton") {
		t.Fatalf("expected WBA vendor example guidance, got: %s", readme)
	}
	uartSmoke := paths["core/examples/nucleo_wba52cg/uart-smoke.yaml"]
	if !strings.Contains(uartSmoke, "thumbv8m.main-none-eabi") {
		t.Fatalf("expected thumbv8m smoke target, got: %s", uartSmoke)
	}
	memoryX := paths["core/examples/nucleo_wba52cg/board_firmware/memory.x"]
	if !strings.Contains(memoryX, "LENGTH = 1024K") || strings.Contains(memoryX, "1024KB") {
		t.Fatalf("expected linker-safe memory sizes, got: %s", memoryX)
	}
	minimalLD := paths["core/examples/nucleo_wba52cg/board_firmware/minimal.ld"]
	if !strings.Contains(minimalLD, "0x20020000") {
		t.Fatalf("expected WBA stack top, got: %s", minimalLD)
	}
}

func TestGenerateArtifact_ExtractsBoardFactsFromPDFUsingPdftotext(t *testing.T) {
	dir := t.TempDir()
	fakePdftotext := filepath.Join(dir, "pdftotext")
	fakePDF := filepath.Join(dir, "board-doc.pdf")
	datasheetPath := filepath.Join(dir, "mcu-datasheet.txt")
	referencePath := filepath.Join(dir, "reference.txt")

	if err := os.WriteFile(fakePdftotext, []byte(`#!/bin/sh
cat <<'EOF'
led_status GPIOC 7 active_high
button_user GPIOA 0 active_low
EOF
`), 0o755); err != nil {
		t.Fatalf("WriteFile fake pdftotext failed: %v", err)
	}
	if err := os.WriteFile(fakePDF, []byte("%PDF-1.4 fake"), 0o644); err != nil {
		t.Fatalf("WriteFile fake pdf failed: %v", err)
	}
	if err := os.WriteFile(datasheetPath, []byte(`
MCU STM32F411RE
FLASH 512KB
RAM 128KB
RCC 0x40023800
GPIOA 0x40020000
GPIOC 0x40020800
USART2 0x40004400 IRQ 38
TX GPIOA 2
RX GPIOA 3
`), 0o644); err != nil {
		t.Fatalf("WriteFile datasheet failed: %v", err)
	}
	if err := os.WriteFile(referencePath, []byte("reference manual STM32F411RE\n"), 0o644); err != nil {
		t.Fatalf("WriteFile reference failed: %v", err)
	}
	t.Setenv("PDFTOTEXT_PATH", fakePdftotext)

	req := Request{
		Kind:          "board_onboarding",
		ComponentName: "PDF extraction board",
		DatasheetURL:  datasheetPath,
		DocumentationURLs: []string{
			fakePDF,
			referencePath,
		},
		Board: &BoardSpec{
			Vendor:        "Acme",
			MarketingName: "PDF Board",
			BoardID:       "pdf-board",
			MCU:           "STM32F411RE",
		},
		DesiredCapabilities: []string{"boot", "uart_console", "led_control", "button_input"},
		ValidationTargets:   []string{"uart_smoke", "io_smoke"},
	}

	artifact, err := GenerateArtifact(context.Background(), req)
	if err != nil {
		t.Fatalf("GenerateArtifact failed: %v", err)
	}
	if artifact.BoardFacts == nil || len(artifact.BoardFacts.ExtractedFacts) == 0 {
		t.Fatalf("expected extracted facts, got %+v", artifact.BoardFacts)
	}
	paths := map[string]string{}
	for _, file := range artifact.RepoBundle.Files {
		paths[file.Path] = file.Content
	}
	systemYAML := paths["core/configs/systems/pdf_board.yaml"]
	if !strings.Contains(systemYAML, "led_status") || !strings.Contains(systemYAML, "button_user") {
		t.Fatalf("expected PDF-extracted board IO, got: %s", systemYAML)
	}
}

func TestGenerateArtifact_G474BoardUsesFamilyAwareVendorPackageAndStackTop(t *testing.T) {
	dir := t.TempDir()
	datasheetPath := filepath.Join(dir, "stm32g474re-datasheet.txt")
	boardDocPath := filepath.Join(dir, "nucleo-g474re-board.txt")
	referencePath := filepath.Join(dir, "stm32g474-reference.txt")
	examplePath := filepath.Join(dir, "stm32cubeg4-example.txt")

	if err := os.WriteFile(datasheetPath, []byte(`
MCU STM32G474RE
FLASH 512KB
RAM 128KB
RCC 0x40021000
GPIOA 0x48000000
GPIOB 0x48000400
GPIOC 0x48000800
GPIOD 0x48000C00
USART2 0x40004400 IRQ 38
TX GPIOA 2
RX GPIOA 3
`), 0o644); err != nil {
		t.Fatalf("WriteFile datasheet failed: %v", err)
	}
	if err := os.WriteFile(boardDocPath, []byte(`
led_user GPIOA 5 active_high
button_b1 GPIOC 13 active_high
`), 0o644); err != nil {
		t.Fatalf("WriteFile board doc failed: %v", err)
	}
	if err := os.WriteFile(referencePath, []byte("reference manual STM32G474RE\n"), 0o644); err != nil {
		t.Fatalf("WriteFile reference failed: %v", err)
	}
	if err := os.WriteFile(examplePath, []byte("STM32CubeG4 UART_Printf\n"), 0o644); err != nil {
		t.Fatalf("WriteFile example failed: %v", err)
	}

	req := Request{
		Kind:          "board_onboarding",
		ComponentName: "NUCLEO-G474RE board onboarding proof",
		DatasheetURL:  datasheetPath,
		DocumentationURLs: []string{
			boardDocPath,
			referencePath,
			examplePath,
		},
		Board: &BoardSpec{
			Vendor:        "STMicroelectronics",
			MarketingName: "NUCLEO-G474RE",
			BoardID:       "nucleo_g474re",
			MCU:           "STM32G474RE",
		},
		DesiredCapabilities: []string{"boot", "uart_console", "led_control", "button_input"},
		ValidationTargets:   []string{"uart_smoke", "io_smoke", "unsupported_instruction_audit"},
		Workload:            &WorkloadSpec{Type: "generated_smoke_firmware", Example: "STM32CubeG4 UART_Printf"},
		Constraints:         &ConstraintSpec{MustWriteRepoAssets: true, MustRunE2EValidation: true},
	}

	artifact, err := GenerateArtifact(context.Background(), req)
	if err != nil {
		t.Fatalf("GenerateArtifact failed: %v", err)
	}
	paths := map[string]string{}
	for _, file := range artifact.RepoBundle.Files {
		paths[file.Path] = file.Content
	}
	readme := paths["core/examples/nucleo_g474re/README.md"]
	if !strings.Contains(readme, "Reference package: STM32CubeG4") {
		t.Fatalf("expected STM32CubeG4 vendor package, got: %s", readme)
	}
	if strings.Contains(readme, "BLE example selection reference only") {
		t.Fatalf("did not expect BLE scope line for G474, got: %s", readme)
	}
	if !strings.Contains(readme, "Vendor example selection reference only") {
		t.Fatalf("expected generic vendor-example scope line, got: %s", readme)
	}
	if strings.Contains(readme, "Confirm whether BLE scope is documentation-only or full simulator behavior") {
		t.Fatalf("did not expect BLE confirmation line for G474, got: %s", readme)
	}
	minimalLD := paths["core/examples/nucleo_g474re/board_firmware/minimal.ld"]
	if !strings.Contains(minimalLD, "0x20020000") {
		t.Fatalf("expected 128KB stack top for G474, got: %s", minimalLD)
	}
	if artifact.ContractResult == nil {
		t.Fatal("expected contract_result")
	}
	if len(artifact.ContractResult.DeferredCapabilities) != 0 {
		t.Fatalf("did not expect deferred wireless capabilities for G474, got: %+v", artifact.ContractResult.DeferredCapabilities)
	}
}

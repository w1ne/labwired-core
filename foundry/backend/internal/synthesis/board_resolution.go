package synthesis

import (
	"bytes"
	"context"
	"fmt"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strings"
	"time"
)

func boardOpenQuestions(req Request, resolution boardResolution) []string {
	questions := []string{
		"Which UART instance is routed to the on-board ST-LINK virtual COM port?",
		"Which exact LED GPIO pins and active levels are used on this board revision?",
	}
	if resolution.HasProfile && len(resolution.Profile.LEDs) > 0 && resolution.Profile.UARTID != "" {
		questions = []string{}
	}
	if resolution.ChipGuess == "" {
		questions = append(questions, "Which exact MCU package is populated on the target board?")
	}
	if !strings.Contains(strings.ToLower(req.Requirements), "ble") {
		return questions
	}
	return append(questions, "Is BLE scope limited to example selection and documentation, or is CPU2/radio behavior expected in simulation?")
}

func resolveSourceDocs(req Request) []SourceDoc {
	docs := []SourceDoc{}
	seen := map[string]bool{}
	add := func(doc SourceDoc) {
		doc.URL = strings.TrimSpace(doc.URL)
		if doc.URL == "" || seen[doc.URL] {
			return
		}
		seen[doc.URL] = true
		docs = append(docs, doc)
	}

	if req.DatasheetURL != "" {
		add(SourceDoc{
			Kind:     "datasheet",
			Title:    "Caller-supplied datasheet",
			URL:      req.DatasheetURL,
			Required: true,
			Origin:   "request",
		})
	}
	for _, url := range req.DocumentationURLs {
		add(SourceDoc{
			Kind:     "supporting_doc",
			Title:    "Caller-supplied supporting document",
			URL:      url,
			Required: true,
			Origin:   "request",
		})
	}

	lower := strings.ToLower(req.ComponentName + " " + req.Requirements)
	if req.Board != nil {
		lower += " " + strings.ToLower(req.Board.MarketingName+" "+req.Board.BoardID+" "+req.Board.MCU)
	}
	if strings.Contains(lower, "mb1355") || strings.Contains(lower, "wb55") || strings.Contains(lower, "nucleo-wb55rg") {
		add(SourceDoc{
			Kind:     "board_user_manual",
			Title:    "UM2819 STM32WB Nucleo-64 board (MB1355)",
			URL:      "https://www.st.com/resource/en/user_manual/um2819-stm32wb-nucleo64-board-mb1355-stmicroelectronics.pdf",
			Required: true,
			Origin:   "autodiscovered",
		})
		add(SourceDoc{
			Kind:     "board_schematic",
			Title:    "MB1355-WB55RG board schematic",
			URL:      "https://www.st.com/resource/en/schematic_pack/mb1355-wb55rg-d01_schematic.pdf",
			Required: true,
			Origin:   "autodiscovered",
		})
		add(SourceDoc{
			Kind:     "mcu_datasheet",
			Title:    "STM32WB55RG datasheet",
			URL:      "https://www.st.com/resource/en/datasheet/stm32wb55rg.pdf",
			Required: true,
			Origin:   "autodiscovered",
		})
		add(SourceDoc{
			Kind:     "reference_manual",
			Title:    "RM0434 STM32WB series reference manual",
			URL:      "https://www.st.com/resource/en/reference_manual/rm0434-stm32wb55xx-stm32wb35xx-advanced-armbased-32bit-mcus-stmicroelectronics.pdf",
			Required: true,
			Origin:   "autodiscovered",
		})
		add(SourceDoc{
			Kind:     "vendor_examples",
			Title:    "STM32CubeWB firmware package",
			URL:      "https://github.com/STMicroelectronics/STM32CubeWB",
			Required: true,
			Origin:   "autodiscovered",
		})
		add(SourceDoc{
			Kind:     "vendor_example",
			Title:    "STM32CubeWB BLE_p2pServer",
			URL:      "https://github.com/STMicroelectronics/STM32CubeWB/tree/master/Projects/P-NUCLEO-WB55.Nucleo/Applications/BLE/BLE_p2pServer",
			Required: true,
			Origin:   "autodiscovered",
		})
		add(SourceDoc{
			Kind:     "vendor_example",
			Title:    "STM32CubeWB BLE_LLD_Pressbutton",
			URL:      "https://github.com/STMicroelectronics/STM32CubeWB/tree/master/Projects/P-NUCLEO-WB55.Nucleo/Applications/BLE/BLE_LLD_Pressbutton",
			Required: false,
			Origin:   "autodiscovered",
		})
	}
	if strings.Contains(lower, "wba52") || strings.Contains(lower, "stm32wba") || strings.Contains(lower, "nucleo-wba52cg") {
		add(SourceDoc{
			Kind:     "board_user_manual",
			Title:    "UM3290 STM32WBA Nucleo board user manual",
			URL:      "https://www.st.com/resource/en/user_manual/um3290-stm32wba5x-nucleo-board-mb1801-stmicroelectronics.pdf",
			Required: true,
			Origin:   "autodiscovered",
		})
		add(SourceDoc{
			Kind:     "mcu_datasheet",
			Title:    "STM32WBA52CG datasheet",
			URL:      "https://www.st.com/resource/en/datasheet/stm32wba52cg.pdf",
			Required: true,
			Origin:   "autodiscovered",
		})
		add(SourceDoc{
			Kind:     "reference_manual",
			Title:    "RM0493 STM32WBA series reference manual",
			URL:      "https://www.st.com/resource/en/reference_manual/rm0493-stm32wba52xx-and-stm32wba54xx-advanced-armbased-32bit-mcus-stmicroelectronics.pdf",
			Required: true,
			Origin:   "autodiscovered",
		})
		add(SourceDoc{
			Kind:     "vendor_examples",
			Title:    "STM32CubeWBA firmware package",
			URL:      "https://github.com/STMicroelectronics/STM32CubeWBA",
			Required: true,
			Origin:   "autodiscovered",
		})
		add(SourceDoc{
			Kind:     "vendor_example",
			Title:    "STM32CubeWBA BLE_p2pServer",
			URL:      "https://github.com/STMicroelectronics/STM32CubeWBA/tree/main/Projects/NUCLEO-WBA52CG/Applications/BLE/BLE_p2pServer",
			Required: false,
			Origin:   "autodiscovered",
		})
	}

	return docs
}

func renderSourceDocs(docs []SourceDoc) string {
	if len(docs) == 0 {
		return "- No source documents resolved."
	}
	lines := make([]string, 0, len(docs))
	for _, doc := range docs {
		label := "recommended"
		if doc.Required {
			label = "required"
		}
		lines = append(lines, fmt.Sprintf("- `%s` %s: %s", doc.Kind, label, doc.URL))
	}
	return strings.Join(lines, "\n")
}

func resolveBoard(req Request, docs []SourceDoc) boardResolution {
	draft := &BoardDraft{
		BoardID: inferBoardID(req),
	}
	if profile, ok := resolvedBoardProfile(draft); ok {
		resolution := boardResolution{
			Profile:    profile,
			HasProfile: true,
			ChipGuess:  inferChipGuess(req),
		}
		resolution.BoardFacts = builtinProfileFacts(profile)
		resolution.MissingFacts = requiredFactGaps(req, resolution)
		return resolution
	}

	extracted := extractBoardProfileFromDocs(req, docs)
	if extracted.HasProfile {
		extracted.MissingFacts = requiredFactGaps(req, extracted)
		return extracted
	}

	resolution := boardResolution{
		ChipGuess: inferChipGuess(req),
	}
	resolution.MissingFacts = requiredFactGaps(req, resolution)
	return resolution
}

func resolvedBoardProfile(draft *BoardDraft) (boardProfile, bool) {
	if draft == nil {
		return boardProfile{}, false
	}
	switch draft.BoardID {
	case "mb1355c":
		return boardProfile{
			ChipName:          "stm32wb55",
			Family:            "stm32wb",
			FlashSize:         "512KB",
			RAMSize:           "256KB",
			RCCBase:           "0x58000000",
			GPIOABase:         "0x48000000",
			GPIOBBase:         "0x48000400",
			GPIOCBase:         "0x48000800",
			GPIODBase:         "0x48000C00",
			UARTID:            "usart1",
			UARTBase:          "0x40013800",
			UARTIRQ:           "37",
			UARTTXPort:        "GPIOB",
			UARTTXPin:         "6",
			UARTRXPort:        "GPIOB",
			UARTRXPin:         "7",
			RustTarget:        "thumbv7em-none-eabi",
			StackTop:          "0x20040000",
			VendorExamplesPkg: "STM32CubeWB",
			PreferredExamples: []ReferenceCandidate{
				{Name: "STM32CubeWB BLE_p2pServer", Reason: "Good proof target for NUCLEO-WB55RG board IO plus BLE-facing application flow."},
				{Name: "STM32CubeWB BLE_LLD_Pressbutton", Reason: "Smaller board-level LED/button proof when radio fidelity is not the first milestone."},
			},
			LEDs: []boardGPIO{
				{ID: "led_blue", Port: "GPIOB", Pin: "5", Active: "high"},
				{ID: "led_green", Port: "GPIOB", Pin: "0", Active: "high"},
				{ID: "led_red", Port: "GPIOB", Pin: "1", Active: "high"},
			},
			Buttons: []boardGPIO{
				{ID: "button_sw1", Port: "GPIOC", Pin: "4", Active: "low"},
				{ID: "button_sw2", Port: "GPIOD", Pin: "0", Active: "low"},
				{ID: "button_sw3", Port: "GPIOD", Pin: "1", Active: "low"},
			},
		}, true
	case "nucleo_wba52cg":
		return boardProfile{
			ChipName:          "stm32wba52",
			Family:            "stm32wba",
			FlashSize:         "1024KB",
			RAMSize:           "128KB",
			RCCBase:           "0x46020C00",
			GPIOABase:         "0x42020000",
			GPIOBBase:         "0x42020400",
			GPIOCBase:         "0x42020800",
			GPIOHBase:         "0x42021C00",
			UARTID:            "lpuart1",
			UARTBase:          "0x46002400",
			UARTIRQ:           "45",
			UARTTXPort:        "GPIOA",
			UARTTXPin:         "2",
			UARTRXPort:        "GPIOA",
			UARTRXPin:         "3",
			RustTarget:        "thumbv8m.main-none-eabi",
			StackTop:          "0x20020000",
			VendorExamplesPkg: "STM32CubeWBA",
			PreferredExamples: []ReferenceCandidate{
				{Name: "STM32CubeWBA BLE_p2pServer", Reason: "Board-level BLE starter aligned with the NUCLEO-WBA52CG app-core bring-up flow."},
			},
			LEDs: []boardGPIO{
				{ID: "led_blue", Port: "GPIOB", Pin: "4", Active: "high"},
				{ID: "led_green", Port: "GPIOA", Pin: "9", Active: "high"},
				{ID: "led_red", Port: "GPIOB", Pin: "8", Active: "high"},
			},
			Buttons: []boardGPIO{
				{ID: "button_b1", Port: "GPIOC", Pin: "13", Active: "high"},
				{ID: "button_b2", Port: "GPIOB", Pin: "6", Active: "high"},
				{ID: "button_b3", Port: "GPIOB", Pin: "7", Active: "high"},
			},
		}, true
	default:
		return boardProfile{}, false
	}
}

func renderBoardIOList(items []boardGPIO, hasProfile bool, fallbackID string, fallbackPeripheral string, kind string) string {
	if !hasProfile || len(items) == 0 {
		return fmt.Sprintf("  - id: \"%s\"\n    kind: \"%s\"\n    peripheral: \"%s\"\n    pin: 0\n    signal: \"%s\"\n    active_high: true\n", fallbackID, kind, fallbackPeripheral, boardSignal(kind))
	}
	lines := make([]string, 0, len(items)*6)
	for _, item := range items {
		lines = append(lines,
			fmt.Sprintf("  - id: \"%s\"", item.ID),
			fmt.Sprintf("    kind: \"%s\"", kind),
			fmt.Sprintf("    peripheral: \"%s\"", strings.ToLower(item.Port)),
			fmt.Sprintf("    pin: %s", item.Pin),
			fmt.Sprintf("    signal: \"%s\"", boardSignal(kind)),
			fmt.Sprintf("    active_high: %t", item.Active == "high"),
		)
	}
	return strings.Join(lines, "\n") + "\n"
}

func boardSignal(kind string) string {
	if kind == "button" {
		return "input"
	}
	return "output"
}

func extractBoardProfileFromDocs(req Request, docs []SourceDoc) boardResolution {
	docTexts := loadDocTexts(docs)
	corpus := docExtractionCorpus(req, docs)
	if strings.TrimSpace(corpus) == "" {
		return boardResolution{}
	}

	profile := boardProfile{}
	facts := BoardFacts{}
	chipGuess := inferChipGuess(Request{
		ComponentName: req.ComponentName + " " + corpus,
		Requirements:  req.Requirements + " " + corpus,
		Board:         req.Board,
	})
	if chipGuess == "" {
		chipGuess = inferChipGuessFromCorpus(corpus)
	}
	addValueFact(&facts.ExtractedFacts, "chip_guess", chipGuess, "doc_extract", "", "", confidenceForValue(chipGuess != ""))

	flashMatch := findFirstDocMatch(docTexts, `(?i)\bflash\b[^0-9]{0,24}(\d+\s*(?:kb|mb))`)
	profile.FlashSize = normalizeSize(flashMatch.Value)
	addDocFact(&facts.ExtractedFacts, "flash_size", profile.FlashSize, flashMatch)
	ramMatch := findFirstDocMatch(docTexts, `(?i)\b(?:ram|sram)\b[^0-9]{0,24}(\d+\s*(?:kb|mb))`)
	profile.RAMSize = normalizeSize(ramMatch.Value)
	addDocFact(&facts.ExtractedFacts, "ram_size", profile.RAMSize, ramMatch)
	rccMatch := findPeripheralBaseWithEvidence(docTexts, "rcc")
	profile.RCCBase = normalizeHex(rccMatch.Value)
	addDocFact(&facts.ExtractedFacts, "rcc_base", profile.RCCBase, rccMatch)
	gpioAMatch := findPeripheralBaseWithEvidence(docTexts, "gpioa")
	profile.GPIOABase = normalizeHex(gpioAMatch.Value)
	addDocFact(&facts.ExtractedFacts, "gpioa_base", profile.GPIOABase, gpioAMatch)
	gpioBMatch := findPeripheralBaseWithEvidence(docTexts, "gpiob")
	profile.GPIOBBase = normalizeHex(gpioBMatch.Value)
	addDocFact(&facts.ExtractedFacts, "gpiob_base", profile.GPIOBBase, gpioBMatch)
	gpioCMatch := findPeripheralBaseWithEvidence(docTexts, "gpioc")
	profile.GPIOCBase = normalizeHex(gpioCMatch.Value)
	addDocFact(&facts.ExtractedFacts, "gpioc_base", profile.GPIOCBase, gpioCMatch)
	gpioDMatch := findPeripheralBaseWithEvidence(docTexts, "gpiod")
	profile.GPIODBase = normalizeHex(gpioDMatch.Value)
	addDocFact(&facts.ExtractedFacts, "gpiod_base", profile.GPIODBase, gpioDMatch)
	gpioHMatch := findPeripheralBaseWithEvidence(docTexts, "gpioh")
	profile.GPIOHBase = normalizeHex(gpioHMatch.Value)
	addDocFact(&facts.ExtractedFacts, "gpioh_base", profile.GPIOHBase, gpioHMatch)
	uartIDMatch := findFirstDocMatch(docTexts, `(?i)\b(usart\d+|uart\d+)\b`)
	profile.UARTID = strings.ToLower(uartIDMatch.Value)
	if profile.UARTID == "" {
		uartIDMatch = findFirstDocMatch(docTexts, `(?i)\b(lpuart\d+)\b`)
		profile.UARTID = strings.ToLower(uartIDMatch.Value)
	}
	addDocFact(&facts.ExtractedFacts, "uart_id", profile.UARTID, uartIDMatch)
	uartBaseMatch := findPeripheralBaseWithEvidence(docTexts, profile.UARTID)
	profile.UARTBase = normalizeHex(uartBaseMatch.Value)
	addDocFact(&facts.ExtractedFacts, "uart_base", profile.UARTBase, uartBaseMatch)
	uartIRQMatch := findUARTIRQWithEvidence(docTexts, profile.UARTID)
	profile.UARTIRQ = uartIRQMatch.Value
	addDocFact(&facts.ExtractedFacts, "uart_irq", profile.UARTIRQ, uartIRQMatch)
	profile.UARTTXPort, profile.UARTTXPin = findSignalPin(corpus, "tx")
	profile.UARTRXPort, profile.UARTRXPin = findSignalPin(corpus, "rx")
	profile.LEDs = extractBoardGPIOs(corpus, "led")
	for _, led := range profile.LEDs {
		addValueFact(&facts.ExtractedFacts, "led."+led.ID, fmt.Sprintf("%s%s active_%s", led.Port, led.Pin, led.Active), "doc_extract", "", led.ID, 0.9)
	}
	profile.Buttons = extractBoardGPIOs(corpus, "button")
	for _, button := range profile.Buttons {
		addValueFact(&facts.ExtractedFacts, "button."+button.ID, fmt.Sprintf("%s%s active_%s", button.Port, button.Pin, button.Active), "doc_extract", "", button.ID, 0.9)
	}
	applyDerivedProfileDefaults(&profile, chipGuess)
	appendDerivedDefaults(&facts.DerivedFacts, profile)

	hasProfile := profile.FlashSize != "" || profile.RAMSize != "" || len(profile.LEDs) > 0 || len(profile.Buttons) > 0 || profile.UARTID != ""
	if !hasProfile {
		return boardResolution{ChipGuess: chipGuess, BoardFacts: facts}
	}

	return boardResolution{
		Profile:        profile,
		HasProfile:     true,
		ChipGuess:      chipGuess,
		ResolvedByDocs: true,
		BoardFacts:     facts,
	}
}

func docExtractionCorpus(req Request, docs []SourceDoc) string {
	parts := []string{req.ComponentName, req.Requirements}
	if req.Board != nil {
		parts = append(parts, req.Board.Vendor, req.Board.MarketingName, req.Board.BoardID, req.Board.MCU)
	}
	for _, doc := range docs {
		parts = append(parts, doc.Title, doc.URL)
		if text := loadDocText(doc.URL); text != "" {
			parts = append(parts, text)
		}
	}
	return strings.Join(parts, "\n")
}

func loadDocText(ref string) string {
	path := strings.TrimSpace(ref)
	if path == "" {
		return ""
	}
	if parsed, err := url.Parse(path); err == nil && parsed.Scheme == "file" {
		path = parsed.Path
	}
	if !filepath.IsAbs(path) {
		return ""
	}
	if strings.EqualFold(filepath.Ext(path), ".pdf") {
		if text := extractPDFText(path); text != "" {
			return text
		}
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return ""
	}
	text := string(data)
	if !isMostlyText(text) {
		return ""
	}
	return text
}

func isMostlyText(value string) bool {
	if value == "" {
		return false
	}
	printable := 0
	for _, r := range value {
		if r == '\n' || r == '\r' || r == '\t' || (r >= 32 && r <= 126) {
			printable++
		}
	}
	return printable >= len(value)*3/4
}

func extractPDFText(path string) string {
	toolPath := strings.TrimSpace(os.Getenv("PDFTOTEXT_PATH"))
	if toolPath == "" {
		toolPath = "pdftotext"
	}
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, toolPath, "-layout", path, "-")
	output, err := cmd.Output()
	if err != nil {
		return ""
	}
	text := strings.TrimSpace(string(bytes.TrimSpace(output)))
	if text == "" || !isMostlyText(text) {
		return ""
	}
	return text
}

func findFirstMatch(corpus, pattern string) string {
	re := regexp.MustCompile(pattern)
	matches := re.FindStringSubmatch(corpus)
	if len(matches) < 2 {
		return ""
	}
	return matches[1]
}

func findPeripheralBase(corpus, peripheral string) string {
	peripheral = strings.TrimSpace(strings.ToLower(peripheral))
	if peripheral == "" {
		return ""
	}
	pattern := fmt.Sprintf(`(?i)\b%s\b[^0-9a-f]{0,24}(0x[0-9a-f]+)`, regexp.QuoteMeta(peripheral))
	return findFirstMatch(corpus, pattern)
}

func findUARTIRQ(corpus, uartID string) string {
	uartID = strings.TrimSpace(strings.ToLower(uartID))
	if uartID == "" {
		return ""
	}
	pattern := fmt.Sprintf(`(?i)\b%s\b.{0,80}?\birq\b[^0-9]{0,8}(\d+)`, regexp.QuoteMeta(uartID))
	return findFirstMatch(corpus, pattern)
}

func findSignalPin(corpus, signal string) (string, string) {
	pattern := fmt.Sprintf(`(?i)\b%s\b[^A-Z0-9]{0,16}(GPIO[A-Z])\s*([0-9]{1,2})`, regexp.QuoteMeta(signal))
	re := regexp.MustCompile(pattern)
	matches := re.FindStringSubmatch(corpus)
	if len(matches) < 3 {
		return "", ""
	}
	return strings.ToUpper(matches[1]), matches[2]
}

func extractBoardGPIOs(corpus, kind string) []boardGPIO {
	re := regexp.MustCompile(fmt.Sprintf(`(?im)^\s*(%s[\w-]*)\s+(GPIO[A-Z])\s*([0-9]{1,2})(?:\s+(active_(?:high|low)|high|low))?`, regexp.QuoteMeta(kind)))
	matches := re.FindAllStringSubmatch(corpus, -1)
	items := make([]boardGPIO, 0, len(matches))
	for _, match := range matches {
		if len(match) < 4 {
			continue
		}
		active := "high"
		if len(match) >= 5 && strings.TrimSpace(match[4]) != "" {
			value := strings.TrimSpace(strings.ToLower(match[4]))
			active = strings.TrimPrefix(value, "active_")
		}
		items = append(items, boardGPIO{
			ID:     SanitizeIdent(match[1]),
			Port:   strings.ToUpper(match[2]),
			Pin:    match[3],
			Active: active,
		})
	}
	return items
}

func normalizeSize(value string) string {
	value = strings.TrimSpace(strings.ToUpper(value))
	value = strings.ReplaceAll(value, " ", "")
	return value
}

func normalizeHex(value string) string {
	value = strings.TrimSpace(value)
	if value == "" {
		return ""
	}
	return "0x" + strings.TrimPrefix(strings.ToLower(value), "0x")
}

func inferChipGuessFromCorpus(corpus string) string {
	re := regexp.MustCompile(`(?i)\b(stm32[a-z0-9]+|nrf[0-9a-z]+|rp2040|samd[0-9a-z]+)\b`)
	match := re.FindStringSubmatch(corpus)
	if len(match) < 2 {
		return ""
	}
	return SanitizeIdent(strings.ToLower(match[1]))
}

func applyDerivedProfileDefaults(profile *boardProfile, chipGuess string) {
	if profile == nil {
		return
	}
	profile.ChipName = chipGuess
	switch {
	case strings.HasPrefix(chipGuess, "stm32wba"):
		profile.Family = "stm32wba"
		if profile.RustTarget == "" {
			profile.RustTarget = "thumbv8m.main-none-eabi"
		}
		if profile.StackTop == "" && profile.RAMSize == "128KB" {
			profile.StackTop = "0x20020000"
		}
		if profile.VendorExamplesPkg == "" {
			profile.VendorExamplesPkg = "STM32CubeWBA"
		}
	case strings.HasPrefix(chipGuess, "stm32wb"):
		profile.Family = "stm32wb"
		if profile.RustTarget == "" {
			profile.RustTarget = "thumbv7em-none-eabi"
		}
		if profile.StackTop == "" && profile.RAMSize == "256KB" {
			profile.StackTop = "0x20040000"
		}
		if profile.VendorExamplesPkg == "" {
			profile.VendorExamplesPkg = "STM32CubeWB"
		}
	case strings.HasPrefix(chipGuess, "stm32f"):
		profile.Family = "stm32f"
		if profile.RustTarget == "" {
			profile.RustTarget = "thumbv7em-none-eabi"
		}
		if profile.StackTop == "" && profile.RAMSize == "128KB" {
			profile.StackTop = "0x20020000"
		}
	}
}

func requiredFactGaps(req Request, resolution boardResolution) []string {
	requested := requestedBoardCapabilities(req)
	missing := []string{}
	hasBoot := resolution.ChipGuess != "" && resolution.Profile.FlashSize != "" && resolution.Profile.RAMSize != ""
	hasUART := resolution.Profile.UARTID != "" && resolution.Profile.UARTBase != ""
	hasLED := len(resolution.Profile.LEDs) > 0
	hasButton := len(resolution.Profile.Buttons) > 0
	for _, capability := range requested {
		switch strings.TrimSpace(strings.ToLower(capability)) {
		case "boot":
			if !hasBoot {
				missing = append(missing, "boot requires chip identity plus flash and ram facts")
			}
		case "uart_console":
			if !hasUART {
				missing = append(missing, "uart_console requires uart instance and base address")
			}
		case "led_control":
			if !hasLED {
				missing = append(missing, "led_control requires at least one LED GPIO mapping")
			}
		case "button_input":
			if !hasButton {
				missing = append(missing, "button_input requires at least one button GPIO mapping")
			}
		}
	}
	return dedupeStrings(missing)
}

func dedupeStrings(items []string) []string {
	seen := map[string]bool{}
	out := make([]string, 0, len(items))
	for _, item := range items {
		item = strings.TrimSpace(item)
		if item == "" || seen[item] {
			continue
		}
		seen[item] = true
		out = append(out, item)
	}
	return out
}

func builtinProfileFacts(profile boardProfile) BoardFacts {
	derived := []FactEvidence{}
	addValueFact(&derived, "flash_size", profile.FlashSize, "builtin_profile", "", "", 1.0)
	addValueFact(&derived, "ram_size", profile.RAMSize, "builtin_profile", "", "", 1.0)
	addValueFact(&derived, "rcc_base", profile.RCCBase, "builtin_profile", "", "", 1.0)
	addValueFact(&derived, "uart_id", profile.UARTID, "builtin_profile", "", "", 1.0)
	addValueFact(&derived, "uart_base", profile.UARTBase, "builtin_profile", "", "", 1.0)
	addValueFact(&derived, "uart_irq", profile.UARTIRQ, "builtin_profile", "", "", 1.0)
	for _, gpio := range boardGPIOPeripheralEntries(profile, true) {
		addValueFact(&derived, gpio.ID+"_base", gpio.BaseAddress, "builtin_profile", "", "", 1.0)
	}
	for _, led := range profile.LEDs {
		addValueFact(&derived, "led."+led.ID, fmt.Sprintf("%s%s active_%s", led.Port, led.Pin, led.Active), "builtin_profile", "", "", 1.0)
	}
	for _, button := range profile.Buttons {
		addValueFact(&derived, "button."+button.ID, fmt.Sprintf("%s%s active_%s", button.Port, button.Pin, button.Active), "builtin_profile", "", "", 1.0)
	}
	return BoardFacts{DerivedFacts: derived}
}

type docText struct {
	URL   string
	Title string
	Text  string
}

type docMatch struct {
	Value    string
	URL      string
	Title    string
	Match    string
	Found    bool
	Multiple bool
}

func loadDocTexts(docs []SourceDoc) []docText {
	texts := make([]docText, 0, len(docs))
	for _, doc := range docs {
		if text := loadDocText(doc.URL); text != "" {
			texts = append(texts, docText{URL: doc.URL, Title: doc.Title, Text: text})
		}
	}
	return texts
}

func findFirstDocMatch(docs []docText, pattern string) docMatch {
	re := regexp.MustCompile(pattern)
	found := []docMatch{}
	for _, doc := range docs {
		matches := re.FindAllStringSubmatch(doc.Text, -1)
		for _, match := range matches {
			if len(match) < 2 {
				continue
			}
			found = append(found, docMatch{
				Value: match[1],
				URL:   doc.URL,
				Title: doc.Title,
				Match: match[0],
				Found: true,
			})
		}
	}
	if len(found) == 0 {
		return docMatch{}
	}
	first := found[0]
	first.Multiple = len(found) > 1
	return first
}

func findPeripheralBaseWithEvidence(docs []docText, peripheral string) docMatch {
	peripheral = strings.TrimSpace(strings.ToLower(peripheral))
	if peripheral == "" {
		return docMatch{}
	}
	return findFirstDocMatch(docs, fmt.Sprintf(`(?i)\b%s\b[^0-9a-f]{0,24}(0x[0-9a-f]+)`, regexp.QuoteMeta(peripheral)))
}

func findUARTIRQWithEvidence(docs []docText, uartID string) docMatch {
	uartID = strings.TrimSpace(strings.ToLower(uartID))
	if uartID == "" {
		return docMatch{}
	}
	return findFirstDocMatch(docs, fmt.Sprintf(`(?i)\b%s\b.{0,80}?\birq\b[^0-9]{0,8}(\d+)`, regexp.QuoteMeta(uartID)))
}

func addDocFact(facts *[]FactEvidence, name string, value string, match docMatch) {
	addValueFact(facts, name, value, "doc_extract", match.URL, match.Match, confidenceForMatch(match))
}

func addValueFact(facts *[]FactEvidence, name string, value string, origin string, sourceDoc string, matchText string, confidence float64) {
	if facts == nil || strings.TrimSpace(value) == "" {
		return
	}
	*facts = append(*facts, FactEvidence{
		Name:       name,
		Value:      value,
		Origin:     origin,
		SourceDoc:  strings.TrimSpace(sourceDoc),
		MatchText:  strings.TrimSpace(matchText),
		Confidence: confidence,
	})
}

func confidenceForMatch(match docMatch) float64 {
	if !match.Found {
		return 0
	}
	if match.Multiple {
		return 0.65
	}
	return 0.95
}

func confidenceForValue(ok bool) float64 {
	if ok {
		return 0.8
	}
	return 0
}

func appendDerivedDefaults(facts *[]FactEvidence, profile boardProfile) {
	addValueFact(facts, "family", profile.Family, "derived", "", "", 0.85)
	addValueFact(facts, "rust_target", profile.RustTarget, "derived", "", "", 0.85)
	addValueFact(facts, "stack_top", profile.StackTop, "derived", "", "", 0.85)
	addValueFact(facts, "vendor_examples_package", profile.VendorExamplesPkg, "derived", "", "", 0.85)
}

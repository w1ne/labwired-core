package synthesis

import (
	"bytes"
	"context"
	"fmt"
	"io"
	"net/http"
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
	if !boardHasWirelessScope(req, resolution.Profile, resolution.ChipGuess) {
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

func renderBoardIOList(items []boardGPIO, profile boardProfile, hasProfile bool, fallbackID string, fallbackPeripheral string, kind string) string {
	if !hasProfile || len(items) == 0 {
		return fmt.Sprintf("  - id: \"%s\"\n    kind: \"%s\"\n    peripheral: \"%s\"\n    pin: 0\n    signal: \"%s\"\n    active_high: true\n", fallbackID, kind, fallbackPeripheral, boardSignal(kind))
	}
	available := map[string]bool{}
	for _, entry := range boardGPIOPeripheralEntries(profile, hasProfile) {
		available[entry.ID] = true
	}
	lines := make([]string, 0, len(items)*6)
	for _, item := range items {
		peripheral := strings.ToLower(item.Port)
		if !available[peripheral] {
			peripheral = fallbackPeripheral
		}
		lines = append(lines,
			fmt.Sprintf("  - id: \"%s\"", item.ID),
			fmt.Sprintf("    kind: \"%s\"", kind),
			fmt.Sprintf("    peripheral: \"%s\"", peripheral),
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
	gpioAMatch := findPeripheralBaseAliasesWithEvidence(docTexts, "gpioa")
	profile.GPIOABase = normalizeHex(gpioAMatch.Value)
	addDocFact(&facts.ExtractedFacts, "gpioa_base", profile.GPIOABase, gpioAMatch)
	gpioBMatch := findPeripheralBaseAliasesWithEvidence(docTexts, "gpiob")
	profile.GPIOBBase = normalizeHex(gpioBMatch.Value)
	addDocFact(&facts.ExtractedFacts, "gpiob_base", profile.GPIOBBase, gpioBMatch)
	gpioCMatch := findPeripheralBaseAliasesWithEvidence(docTexts, "gpioc")
	profile.GPIOCBase = normalizeHex(gpioCMatch.Value)
	addDocFact(&facts.ExtractedFacts, "gpioc_base", profile.GPIOCBase, gpioCMatch)
	gpioDMatch := findPeripheralBaseAliasesWithEvidence(docTexts, "gpiod")
	profile.GPIODBase = normalizeHex(gpioDMatch.Value)
	addDocFact(&facts.ExtractedFacts, "gpiod_base", profile.GPIODBase, gpioDMatch)
	gpioHMatch := findPeripheralBaseAliasesWithEvidence(docTexts, "gpioh")
	profile.GPIOHBase = normalizeHex(gpioHMatch.Value)
	addDocFact(&facts.ExtractedFacts, "gpioh_base", profile.GPIOHBase, gpioHMatch)
	uartIDMatch := findUARTIDWithEvidence(docTexts)
	profile.UARTID = strings.ToLower(uartIDMatch.Value)
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
	if parsed, err := url.Parse(path); err == nil {
		switch parsed.Scheme {
		case "file":
			path = parsed.Path
		case "http", "https":
			return fetchRemoteDocText(parsed.String())
		}
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

func fetchRemoteDocText(rawURL string) string {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	req, err := http.NewRequestWithContext(ctx, http.MethodGet, rawURL, nil)
	if err != nil {
		return ""
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return ""
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return ""
	}
	body, err := io.ReadAll(io.LimitReader(resp.Body, 8<<20))
	if err != nil || len(body) == 0 {
		return ""
	}

	contentType := strings.ToLower(strings.TrimSpace(resp.Header.Get("Content-Type")))
	if strings.Contains(contentType, "pdf") || strings.EqualFold(filepath.Ext(strings.ToLower(rawURL)), ".pdf") {
		return extractPDFBytes(body)
	}
	text := strings.TrimSpace(string(bytes.TrimSpace(body)))
	if text == "" || !isMostlyText(text) {
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
	return normalizeExtractedPDFText(output)
}

func extractPDFBytes(data []byte) string {
	tmpFile, err := os.CreateTemp("", "labwired-doc-*.pdf")
	if err != nil {
		return ""
	}
	tmpPath := tmpFile.Name()
	defer os.Remove(tmpPath)
	if _, err := tmpFile.Write(data); err != nil {
		_ = tmpFile.Close()
		return ""
	}
	if err := tmpFile.Close(); err != nil {
		return ""
	}
	return extractPDFText(tmpPath)
}

func normalizeExtractedPDFText(output []byte) string {
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
	pattern := fmt.Sprintf(`(?i)\b%s\b[^\n]{0,64}?(0x[0-9a-f]+)`, regexp.QuoteMeta(peripheral))
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
	patterns := []string{
		fmt.Sprintf(`(?i)\b%s\b[^A-Z0-9]{0,16}(GPIO[A-Z])\s*([0-9]{1,2})`, regexp.QuoteMeta(signal)),
		fmt.Sprintf(`(?i)\b%s\b[^A-Z0-9]{0,16}(P[A-Z])\s*([0-9]{1,2})`, regexp.QuoteMeta(signal)),
		fmt.Sprintf(`(?i)\b%s\b[^A-Z0-9]{0,16}(P[0-9])\s*([0-9]{1,2})`, regexp.QuoteMeta(signal)),
		fmt.Sprintf(`(?i)\b%s\b[^A-Z0-9]{0,16}(P[0-9])([0-9]{2})`, regexp.QuoteMeta(signal)),
	}
	for _, pattern := range patterns {
		re := regexp.MustCompile(pattern)
		matches := re.FindStringSubmatch(corpus)
		if len(matches) < 3 {
			continue
		}
		port, pin := normalizeBoardPortPin(matches[1], matches[2])
		if port != "" && pin != "" {
			return port, pin
		}
	}
	return "", ""
}

func extractBoardGPIOs(corpus, kind string) []boardGPIO {
	patterns := []string{
		fmt.Sprintf(`(?im)^\s*(%s[\w-]*)\s+(GPIO[A-Z])\s*([0-9]{1,2})(?:\s+(active_(?:high|low)|high|low))?`, regexp.QuoteMeta(kind)),
		fmt.Sprintf(`(?im)^\s*(%s[\w-]*)\s+(GPIO)\s*([0-9]{1,2})(?:\s+(active_(?:high|low)|high|low))?`, regexp.QuoteMeta(kind)),
		fmt.Sprintf(`(?im)^\s*(%s[\w-]*)\s+(GPIO[0-9]+)\s+([0-9]{1,2})(?:\s+(active_(?:high|low)|high|low))?`, regexp.QuoteMeta(kind)),
		fmt.Sprintf(`(?im)^\s*(%s[\w-]*)\s+(GPIO)([0-9]{1,2})(?:\s+(active_(?:high|low)|high|low))?`, regexp.QuoteMeta(kind)),
		fmt.Sprintf(`(?im)^\s*(%s[\w-]*)\s+(P[A-Z])\s*([0-9]{1,2})(?:\s+(active_(?:high|low)|high|low))?`, regexp.QuoteMeta(kind)),
		fmt.Sprintf(`(?im)^\s*(%s[\w-]*)\s+(P[0-9])\s*([0-9]{1,2})(?:\s+(active_(?:high|low)|high|low))?`, regexp.QuoteMeta(kind)),
		fmt.Sprintf(`(?im)^\s*(%s[\w-]*)\s+(P[0-9])([0-9]{2})(?:\s+(active_(?:high|low)|high|low))?`, regexp.QuoteMeta(kind)),
	}
	items := []boardGPIO{}
	seen := map[string]bool{}
	for _, pattern := range patterns {
		re := regexp.MustCompile(pattern)
		matches := re.FindAllStringSubmatch(corpus, -1)
		for _, match := range matches {
			if len(match) < 4 {
				continue
			}
			port, pin := normalizeBoardPortPin(match[2], match[3])
			if port == "" || pin == "" {
				continue
			}
			active := "high"
			if len(match) >= 5 && strings.TrimSpace(match[4]) != "" {
				value := strings.TrimSpace(strings.ToLower(match[4]))
				active = strings.TrimPrefix(value, "active_")
			}
			id := SanitizeIdent(match[1])
			key := id + ":" + port + ":" + pin
			if seen[key] {
				continue
			}
			seen[key] = true
			items = append(items, boardGPIO{
				ID:     id,
				Port:   port,
				Pin:    pin,
				Active: active,
			})
		}
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
	re := regexp.MustCompile(`(?i)\b(stm32[a-z0-9]+|gd32[a-z0-9]+|ch32[a-z0-9]+|nrf[0-9a-z]+|generic-rv32i|rp2040|samd[0-9a-z]+|same[0-9a-z]+|efr32[0-9a-z]+|fe310|esp32c3|ra6m5)\b`)
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
		profile.Arch = "arm"
		if profile.FlashBase == "" {
			profile.FlashBase = "0x08000000"
		}
		if profile.RAMBase == "" {
			profile.RAMBase = "0x20000000"
		}
		if profile.RustTarget == "" {
			profile.RustTarget = "thumbv8m.main-none-eabi"
		}
		if profile.StackTop == "" {
			profile.StackTop = stackTopForRAM(profile.RAMSize)
		}
		if profile.VendorExamplesPkg == "" {
			profile.VendorExamplesPkg = "STM32CubeWBA"
		}
	case strings.HasPrefix(chipGuess, "stm32wb"):
		profile.Family = "stm32wb"
		profile.Arch = "arm"
		if profile.FlashBase == "" {
			profile.FlashBase = "0x08000000"
		}
		if profile.RAMBase == "" {
			profile.RAMBase = "0x20000000"
		}
		if profile.RustTarget == "" {
			profile.RustTarget = "thumbv7em-none-eabi"
		}
		if profile.StackTop == "" {
			profile.StackTop = stackTopForRAM(profile.RAMSize)
		}
		if profile.VendorExamplesPkg == "" {
			profile.VendorExamplesPkg = "STM32CubeWB"
		}
	case strings.HasPrefix(chipGuess, "stm32g"):
		profile.Family = "stm32g"
		profile.Arch = "arm"
		if profile.FlashBase == "" {
			profile.FlashBase = "0x08000000"
		}
		if profile.RAMBase == "" {
			profile.RAMBase = "0x20000000"
		}
		if profile.RustTarget == "" {
			profile.RustTarget = "thumbv7em-none-eabi"
		}
		if profile.StackTop == "" {
			profile.StackTop = stackTopForRAM(profile.RAMSize)
		}
		if profile.VendorExamplesPkg == "" {
			profile.VendorExamplesPkg = "STM32CubeG4"
		}
	case strings.HasPrefix(chipGuess, "stm32l"):
		profile.Family = "stm32l"
		profile.Arch = "arm"
		if profile.FlashBase == "" {
			profile.FlashBase = "0x08000000"
		}
		if profile.RAMBase == "" {
			profile.RAMBase = "0x20000000"
		}
		if profile.RustTarget == "" {
			profile.RustTarget = "thumbv7em-none-eabi"
		}
		if profile.StackTop == "" {
			profile.StackTop = stackTopForRAM(profile.RAMSize)
		}
		if profile.VendorExamplesPkg == "" {
			profile.VendorExamplesPkg = "STM32CubeL4"
		}
	case strings.HasPrefix(chipGuess, "stm32f"):
		profile.Family = "stm32f"
		profile.Arch = "arm"
		if profile.FlashBase == "" {
			profile.FlashBase = "0x08000000"
		}
		if profile.RAMBase == "" {
			profile.RAMBase = "0x20000000"
		}
		if profile.RustTarget == "" {
			profile.RustTarget = "thumbv7em-none-eabi"
		}
		if profile.StackTop == "" {
			profile.StackTop = stackTopForRAM(profile.RAMSize)
		}
	case strings.HasPrefix(chipGuess, "gd32f"):
		profile.Family = "gd32f"
		profile.Arch = "arm"
		if profile.FlashBase == "" {
			profile.FlashBase = "0x08000000"
		}
		if profile.RAMBase == "" {
			profile.RAMBase = "0x20000000"
		}
		if profile.RustTarget == "" {
			profile.RustTarget = "thumbv7m-none-eabi"
		}
		if profile.StackTop == "" {
			profile.StackTop = stackTopForRAM(profile.RAMSize)
		}
	case strings.HasPrefix(chipGuess, "nrf52"):
		profile.Family = "nrf52"
		profile.Arch = "arm"
		if profile.FlashBase == "" {
			profile.FlashBase = "0x00000000"
		}
		if profile.RAMBase == "" {
			profile.RAMBase = "0x20000000"
		}
		if profile.RustTarget == "" {
			profile.RustTarget = "thumbv7em-none-eabi"
		}
		if profile.StackTop == "" {
			profile.StackTop = stackTopForRAM(profile.RAMSize)
		}
	case chipGuess == "rp2040" || strings.HasPrefix(chipGuess, "samd21"):
		profile.Family = chipGuess
		profile.Arch = "arm"
		if profile.FlashBase == "" {
			if chipGuess == "rp2040" {
				profile.FlashBase = "0x10000000"
			} else {
				profile.FlashBase = "0x00000000"
			}
		}
		if profile.RAMBase == "" {
			profile.RAMBase = "0x20000000"
		}
		if chipGuess == "rp2040" && profile.GPIOABase == "" {
			profile.GPIOABase = "0xD0000000"
		}
		if profile.RustTarget == "" {
			profile.RustTarget = "thumbv6m-none-eabi"
		}
		if profile.StackTop == "" {
			profile.StackTop = stackTopForRAM(profile.RAMSize)
		}
	case strings.HasPrefix(chipGuess, "same54"):
		profile.Family = "same54"
		profile.Arch = "arm"
		if profile.FlashBase == "" {
			profile.FlashBase = "0x00000000"
		}
		if profile.RAMBase == "" {
			profile.RAMBase = "0x20000000"
		}
		if profile.RustTarget == "" {
			profile.RustTarget = "thumbv7em-none-eabi"
		}
		if profile.StackTop == "" {
			profile.StackTop = stackTopForRAM(profile.RAMSize)
		}
	case strings.HasPrefix(chipGuess, "efr32"):
		profile.Family = "efr32"
		profile.Arch = "arm"
		if profile.FlashBase == "" {
			profile.FlashBase = "0x00000000"
		}
		if profile.RAMBase == "" {
			profile.RAMBase = "0x20000000"
		}
		if profile.RustTarget == "" {
			profile.RustTarget = "thumbv8m.main-none-eabi"
		}
		if profile.StackTop == "" {
			profile.StackTop = stackTopForRAM(profile.RAMSize)
		}
	case strings.HasPrefix(chipGuess, "ra6m5"):
		profile.Family = "ra6m5"
		profile.Arch = "arm"
		if profile.FlashBase == "" {
			profile.FlashBase = "0x00000000"
		}
		if profile.RAMBase == "" {
			profile.RAMBase = "0x20000000"
		}
		if profile.RustTarget == "" {
			profile.RustTarget = "thumbv8m.main-none-eabi"
		}
		if profile.StackTop == "" {
			profile.StackTop = stackTopForBaseAndRAM(profile.RAMBase, profile.RAMSize)
		}
	case chipGuess == "generic-rv32i":
		profile.Family = "generic-rv32i"
		profile.Arch = "riscv"
		if profile.FlashBase == "" {
			profile.FlashBase = "0x80000000"
		}
		if profile.RAMBase == "" {
			profile.RAMBase = "0x80040000"
		}
		if profile.RustTarget == "" {
			profile.RustTarget = "riscv32i-unknown-none-elf"
		}
		if profile.StackTop == "" {
			profile.StackTop = stackTopForBaseAndRAM(profile.RAMBase, profile.RAMSize)
		}
	case strings.HasPrefix(chipGuess, "ch32v"):
		profile.Family = "ch32v"
		profile.Arch = "riscv"
		if profile.FlashBase == "" {
			profile.FlashBase = "0x08000000"
		}
		if profile.RAMBase == "" {
			profile.RAMBase = "0x20000000"
		}
		if profile.RustTarget == "" {
			profile.RustTarget = "riscv32i-unknown-none-elf"
		}
		if profile.StackTop == "" {
			profile.StackTop = stackTopForBaseAndRAM(profile.RAMBase, profile.RAMSize)
		}
	case chipGuess == "fe310":
		profile.Family = "fe310"
		profile.Arch = "riscv"
		if profile.FlashBase == "" {
			profile.FlashBase = "0x20000000"
		}
		if profile.RAMBase == "" {
			profile.RAMBase = "0x80000000"
		}
		if profile.RustTarget == "" {
			profile.RustTarget = "riscv32i-unknown-none-elf"
		}
		if profile.StackTop == "" {
			profile.StackTop = stackTopForBaseAndRAM(profile.RAMBase, profile.RAMSize)
		}
	case chipGuess == "esp32c3":
		profile.Family = "esp32c3"
		profile.Arch = "riscv"
		if profile.FlashBase == "" {
			profile.FlashBase = "0x42000000"
		}
		if profile.RAMBase == "" {
			profile.RAMBase = "0x3FC80000"
		}
		if profile.RustTarget == "" {
			profile.RustTarget = "riscv32i-unknown-none-elf"
		}
		if profile.StackTop == "" {
			profile.StackTop = stackTopForBaseAndRAM(profile.RAMBase, profile.RAMSize)
		}
	}
	if profile.Arch == "" {
		profile.Arch = "arm"
	}
	if profile.FlashBase == "" {
		profile.FlashBase = "0x08000000"
	}
	if profile.RAMBase == "" {
		profile.RAMBase = "0x20000000"
	}
}

func stackTopForRAM(ramSize string) string {
	return stackTopForBaseAndRAM("0x20000000", ramSize)
}

func stackTopForBaseAndRAM(ramBase string, ramSize string) string {
	base, ok := parseHex(ramBase)
	if !ok {
		return ""
	}
	ramSize = strings.TrimSpace(strings.ToUpper(ramSize))
	if ramSize == "" {
		return ""
	}
	multiplier := 1
	switch {
	case strings.HasSuffix(ramSize, "KB"):
		ramSize = strings.TrimSuffix(ramSize, "KB")
		multiplier = 1024
	case strings.HasSuffix(ramSize, "MB"):
		ramSize = strings.TrimSuffix(ramSize, "MB")
		multiplier = 1024 * 1024
	case strings.HasSuffix(ramSize, "K"):
		ramSize = strings.TrimSuffix(ramSize, "K")
		multiplier = 1024
	case strings.HasSuffix(ramSize, "M"):
		ramSize = strings.TrimSuffix(ramSize, "M")
		multiplier = 1024 * 1024
	default:
		return ""
	}
	ramSize = strings.TrimSpace(ramSize)
	var qty int
	for _, ch := range ramSize {
		if ch < '0' || ch > '9' {
			return ""
		}
		qty = qty*10 + int(ch-'0')
	}
	if qty <= 0 {
		return ""
	}
	return fmt.Sprintf("0x%08X", base+qty*multiplier)
}

func parseHex(value string) (int, bool) {
	value = strings.TrimSpace(strings.TrimPrefix(strings.ToLower(value), "0x"))
	if value == "" {
		return 0, false
	}
	total := 0
	for _, ch := range value {
		total <<= 4
		switch {
		case ch >= '0' && ch <= '9':
			total += int(ch - '0')
		case ch >= 'a' && ch <= 'f':
			total += int(ch-'a') + 10
		default:
			return 0, false
		}
	}
	return total, true
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
	return findFirstDocMatch(docs, fmt.Sprintf(`(?i)\b%s\b[^\n]{0,64}?(0x[0-9a-f]+)`, regexp.QuoteMeta(peripheral)))
}

func findPeripheralBaseAliasesWithEvidence(docs []docText, peripheral string) docMatch {
	for _, alias := range peripheralAliases(peripheral) {
		if match := findPeripheralBaseWithEvidence(docs, alias); match.Found {
			return match
		}
	}
	return docMatch{}
}

func findUARTIRQWithEvidence(docs []docText, uartID string) docMatch {
	uartID = strings.TrimSpace(strings.ToLower(uartID))
	if uartID == "" {
		return docMatch{}
	}
	return findFirstDocMatch(docs, fmt.Sprintf(`(?i)\b%s\b.{0,80}?\birq\b[^0-9]{0,8}(\d+)`, regexp.QuoteMeta(uartID)))
}

func findUARTIDWithEvidence(docs []docText) docMatch {
	patterns := []string{
		`(?i)\b(usart\d+|uart\d+)\b`,
		`(?i)\b(lpuart\d+)\b`,
		`(?i)\b(uarte\d+)\b`,
		`(?i)\b(sercom\d+)\s+(?:usart|uart)\b`,
		`(?i)\b(sci\d+)\b`,
	}
	for _, pattern := range patterns {
		if match := findFirstDocMatch(docs, pattern); match.Found {
			return match
		}
	}
	return docMatch{}
}

func peripheralAliases(peripheral string) []string {
	peripheral = strings.TrimSpace(strings.ToLower(peripheral))
	switch peripheral {
	case "gpioa":
		return []string{"gpioa", "gpio", "porta", "port0", "pa", "p0", "gpio0"}
	case "gpiob":
		return []string{"gpiob", "portb", "port1", "pb", "p1", "gpio1"}
	case "gpioc":
		return []string{"gpioc", "portc", "port2", "pc", "p2", "gpio2"}
	case "gpiod":
		return []string{"gpiod", "portd", "port3", "pd", "p3", "gpio3"}
	case "gpioh":
		return []string{"gpioh", "porth", "ph", "p7", "gpio7"}
	default:
		return []string{peripheral}
	}
}

func normalizeBoardPortPin(rawPort string, rawPin string) (string, string) {
	port := strings.ToUpper(strings.TrimSpace(rawPort))
	pin := strings.TrimLeft(strings.TrimSpace(rawPin), "0")
	if pin == "" {
		pin = "0"
	}
	switch {
	case port == "GPIO":
		return "GPIO", pin
	case strings.HasPrefix(port, "GPIO") && len(port) > 4 && port[4] >= '0' && port[4] <= '9':
		return "GPIO", pin
	case strings.HasPrefix(port, "GPIO"):
		return port, pin
	case len(port) == 2 && port[0] == 'P' && port[1] >= 'A' && port[1] <= 'Z':
		return "GPIO" + port[1:], pin
	case len(port) == 2 && port[0] == 'P' && port[1] >= '0' && port[1] <= '9':
		return "GPIO" + string(rune('A'+(port[1]-'0'))), pin
	default:
		return "", ""
	}
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
	addValueFact(facts, "arch", profile.Arch, "derived", "", "", 0.85)
	addValueFact(facts, "flash_base", profile.FlashBase, "derived", "", "", 0.85)
	addValueFact(facts, "ram_base", profile.RAMBase, "derived", "", "", 0.85)
	addValueFact(facts, "family", profile.Family, "derived", "", "", 0.85)
	addValueFact(facts, "rust_target", profile.RustTarget, "derived", "", "", 0.85)
	addValueFact(facts, "stack_top", profile.StackTop, "derived", "", "", 0.85)
	addValueFact(facts, "vendor_examples_package", profile.VendorExamplesPkg, "derived", "", "", 0.85)
}

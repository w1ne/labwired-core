package synthesis

import (
	"context"
	"fmt"
	"os"
	"regexp"
	"strings"
	"time"
)

type Request struct {
	Kind                string
	ComponentName       string
	Requirements        string
	DatasheetURL        string
	DocumentationURLs   []string
	Board               *BoardSpec
	DesiredCapabilities []string
	ValidationTargets   []string
	Workload            *WorkloadSpec
	Constraints         *ConstraintSpec
}

type BoardSpec struct {
	Vendor        string `json:"vendor,omitempty"`
	MarketingName string `json:"marketing_name,omitempty"`
	BoardID       string `json:"board_id,omitempty"`
	MCU           string `json:"mcu,omitempty"`
}

type WorkloadSpec struct {
	Type         string `json:"type,omitempty"`
	FirmwarePath string `json:"firmware_path,omitempty"`
	Example      string `json:"example,omitempty"`
}

type ConstraintSpec struct {
	BLEScope             string `json:"ble_scope,omitempty"`
	MustWriteRepoAssets  bool   `json:"must_write_repo_assets,omitempty"`
	MustRunE2EValidation bool   `json:"must_run_e2e_validation,omitempty"`
}

type Artifact struct {
	ArtifactType   string          `json:"artifact_type"`
	Name           string          `json:"name"`
	Description    string          `json:"description"`
	Source         string          `json:"source"`
	GeneratedAt    string          `json:"generated_at"`
	Inputs         ArtifactInputs  `json:"inputs"`
	ContractResult *ContractResult `json:"contract_result,omitempty"`
	Summary        string          `json:"summary,omitempty"`
	SourceDocs     []SourceDoc     `json:"source_docs,omitempty"`
	BoardFacts     *BoardFacts     `json:"board_facts,omitempty"`
	RepoBundle     *RepoBundle     `json:"repo_bundle,omitempty"`
	BoardDraft     *BoardDraft     `json:"board_draft,omitempty"`
	ModelDraft     *ModelDraft     `json:"model_draft,omitempty"`
}

type ContractResult struct {
	RequestKind           string   `json:"request_kind"`
	RequestedCapabilities []string `json:"requested_capabilities,omitempty"`
	ValidatedCapabilities []string `json:"validated_capabilities,omitempty"`
	DeferredCapabilities  []string `json:"deferred_capabilities,omitempty"`
	MissingCapabilities   []string `json:"missing_capabilities,omitempty"`
	ValidationTargets     []string `json:"validation_targets,omitempty"`
	EvidenceArtifacts     []string `json:"evidence_artifacts,omitempty"`
	PromotionMode         string   `json:"promotion_mode,omitempty"`
}

type ArtifactInputs struct {
	Kind                string   `json:"kind,omitempty"`
	Requirements        string   `json:"requirements"`
	DatasheetURL        string   `json:"datasheet_url,omitempty"`
	DocumentationURLs   []string `json:"documentation_urls,omitempty"`
	DesiredCapabilities []string `json:"desired_capabilities,omitempty"`
	ValidationTargets   []string `json:"validation_targets,omitempty"`
}

type SourceDoc struct {
	Kind     string `json:"kind"`
	Title    string `json:"title"`
	URL      string `json:"url"`
	Required bool   `json:"required"`
	Origin   string `json:"origin,omitempty"`
}

type BoardDraft struct {
	BoardID               string               `json:"board_id"`
	ChipGuess             string               `json:"chip_guess,omitempty"`
	RequestedCapabilities []string             `json:"requested_capabilities,omitempty"`
	ValidatedCapabilities []string             `json:"validated_capabilities,omitempty"`
	ValidationTargets     []string             `json:"validation_targets,omitempty"`
	BringupScope          []string             `json:"bringup_scope"`
	DeferredScope         []string             `json:"deferred_scope,omitempty"`
	RepoArtifacts         []DraftArtifact      `json:"repo_artifacts"`
	RecommendedExamples   []ReferenceCandidate `json:"recommended_examples,omitempty"`
	SourceRequirements    []string             `json:"source_requirements"`
	ValidationPlan        []string             `json:"validation_plan"`
	OpenQuestions         []string             `json:"open_questions,omitempty"`
}

type BoardFacts struct {
	ExtractedFacts      []FactEvidence `json:"extracted_facts,omitempty"`
	DerivedFacts        []FactEvidence `json:"derived_facts,omitempty"`
	FallbackAssumptions []FactEvidence `json:"fallback_assumptions,omitempty"`
}

type FactEvidence struct {
	Name       string  `json:"name"`
	Value      string  `json:"value"`
	Origin     string  `json:"origin"`
	SourceDoc  string  `json:"source_doc,omitempty"`
	MatchText  string  `json:"match_text,omitempty"`
	Confidence float64 `json:"confidence,omitempty"`
}

type RepoBundle struct {
	Files []GeneratedFile `json:"files"`
}

type GeneratedFile struct {
	Path        string `json:"path"`
	Description string `json:"description"`
	Content     string `json:"content"`
}

type boardProfile struct {
	ChipName          string
	Family            string
	Arch              string
	FlashBase         string
	FlashSize         string
	RAMBase           string
	RAMSize           string
	RCCBase           string
	GPIOABase         string
	GPIOBBase         string
	GPIOCBase         string
	GPIODBase         string
	GPIOHBase         string
	UARTID            string
	UARTBase          string
	UARTIRQ           string
	UARTTXPort        string
	UARTTXPin         string
	UARTRXPort        string
	UARTRXPin         string
	RustTarget        string
	StackTop          string
	VendorExamplesPkg string
	PreferredExamples []ReferenceCandidate
	LEDs              []boardGPIO
	Buttons           []boardGPIO
}

type boardResolution struct {
	Profile        boardProfile
	HasProfile     bool
	ChipGuess      string
	ResolvedByDocs bool
	BoardFacts     BoardFacts
	MissingFacts   []string
}

type boardGPIO struct {
	ID     string
	Port   string
	Pin    string
	Active string
}

type DraftArtifact struct {
	Path    string `json:"path"`
	Purpose string `json:"purpose"`
}

type ReferenceCandidate struct {
	Name   string `json:"name"`
	Reason string `json:"reason"`
}

type ModelDraft struct {
	BusHints           []string  `json:"bus_hints,omitempty"`
	Registers          []RegHint `json:"registers"`
	ValidationPlan     []string  `json:"validation_plan"`
	OpenQuestions      []string  `json:"open_questions,omitempty"`
	StrictIRDraft      *StrictIR `json:"strict_ir_draft,omitempty"`
	BehavioralCoverage []string  `json:"behavioral_coverage,omitempty"`
}

type RegHint struct {
	Name        string `json:"name"`
	Offset      string `json:"offset"`
	Access      string `json:"access"`
	ResetValue  string `json:"reset_value,omitempty"`
	Description string `json:"description,omitempty"`
}

func GenerateArtifact(ctx context.Context, req Request) (*Artifact, error) {
	req.Kind = strings.TrimSpace(req.Kind)
	req.ComponentName = strings.TrimSpace(req.ComponentName)
	req.Requirements = strings.TrimSpace(req.Requirements)
	req.DatasheetURL = strings.TrimSpace(req.DatasheetURL)
	for i := range req.DocumentationURLs {
		req.DocumentationURLs[i] = strings.TrimSpace(req.DocumentationURLs[i])
	}
	for i := range req.DesiredCapabilities {
		req.DesiredCapabilities[i] = strings.TrimSpace(req.DesiredCapabilities[i])
	}
	for i := range req.ValidationTargets {
		req.ValidationTargets[i] = strings.TrimSpace(req.ValidationTargets[i])
	}
	if req.Board != nil {
		req.Board.Vendor = strings.TrimSpace(req.Board.Vendor)
		req.Board.MarketingName = strings.TrimSpace(req.Board.MarketingName)
		req.Board.BoardID = strings.TrimSpace(req.Board.BoardID)
		req.Board.MCU = strings.TrimSpace(req.Board.MCU)
	}
	if req.Workload != nil {
		req.Workload.Type = strings.TrimSpace(req.Workload.Type)
		req.Workload.FirmwarePath = strings.TrimSpace(req.Workload.FirmwarePath)
		req.Workload.Example = strings.TrimSpace(req.Workload.Example)
	}
	if req.Constraints != nil {
		req.Constraints.BLEScope = strings.TrimSpace(req.Constraints.BLEScope)
	}

	artifact := &Artifact{
		Name:        req.ComponentName,
		Source:      "foundry-synthesis",
		GeneratedAt: time.Now().UTC().Format(time.RFC3339),
		Inputs: ArtifactInputs{
			Kind:                req.Kind,
			Requirements:        req.Requirements,
			DatasheetURL:        req.DatasheetURL,
			DocumentationURLs:   append([]string(nil), req.DocumentationURLs...),
			DesiredCapabilities: append([]string(nil), req.DesiredCapabilities...),
			ValidationTargets:   append([]string(nil), req.ValidationTargets...),
		},
	}
	artifact.SourceDocs = resolveSourceDocs(req)

	if isBoardRequest(req) {
		boardResolution := resolveBoard(req, artifact.SourceDocs)
		if len(boardResolution.MissingFacts) > 0 {
			return nil, fmt.Errorf("insufficient grounded facts for requested capabilities: %s", strings.Join(boardResolution.MissingFacts, "; "))
		}
		artifact.ArtifactType = "board_onboarding_draft"
		artifact.Description = fmt.Sprintf("Board onboarding draft for %s.", req.ComponentName)
		artifact.BoardFacts = &boardResolution.BoardFacts
		artifact.BoardDraft = buildBoardDraft(req, boardResolution)
		artifact.RepoBundle = buildBoardRepoBundle(req, artifact.BoardDraft, artifact.SourceDocs)
		artifact.ContractResult = buildBoardContractResult(req, artifact.BoardDraft)
	} else {
		artifact.ArtifactType = "strict_ir_draft"
		artifact.Description = fmt.Sprintf("Peripheral/model draft for %s.", req.ComponentName)
		artifact.ModelDraft = buildModelDraft(req)
		artifact.ContractResult = buildPeripheralContractResult(req)
	}

	if summary, err := maybeGenerateSummary(ctx, req, artifact.ArtifactType); err == nil && strings.TrimSpace(summary) != "" {
		artifact.Summary = strings.TrimSpace(summary)
	}

	return artifact, nil
}

func ValidateArtifact(artifact *Artifact) (int, error) {
	if artifact == nil {
		return 0, fmt.Errorf("artifact is nil")
	}
	if strings.TrimSpace(artifact.Name) == "" {
		return 0, fmt.Errorf("artifact name is empty")
	}
	switch artifact.ArtifactType {
	case "board_onboarding_draft":
		if artifact.BoardDraft == nil {
			return 0, fmt.Errorf("board draft missing")
		}
		if artifact.BoardFacts == nil {
			return 0, fmt.Errorf("board facts missing")
		}
		if len(artifact.BoardDraft.RepoArtifacts) < 3 {
			return 0, fmt.Errorf("board draft must include repo artifacts")
		}
		if artifact.RepoBundle == nil || len(artifact.RepoBundle.Files) < 5 {
			return 0, fmt.Errorf("board draft must include generated repo bundle files")
		}
		if len(artifact.SourceDocs) < 3 {
			return 0, fmt.Errorf("board draft must include source docs")
		}
		if len(artifact.BoardDraft.ValidationPlan) == 0 {
			return 0, fmt.Errorf("board draft must include validation plan")
		}
		if artifact.ContractResult == nil {
			return 0, fmt.Errorf("board draft must include contract result")
		}
		return 5, nil
	case "strict_ir_draft":
		if artifact.ModelDraft == nil {
			return 0, fmt.Errorf("model draft missing")
		}
		if len(artifact.ModelDraft.Registers) == 0 {
			return 0, fmt.Errorf("model draft must include at least one register hint")
		}
		if artifact.ModelDraft.StrictIRDraft == nil || len(artifact.ModelDraft.StrictIRDraft.Peripherals) == 0 {
			return 0, fmt.Errorf("strict IR draft missing")
		}
		if artifact.ContractResult == nil {
			return 0, fmt.Errorf("model draft must include contract result")
		}
		return 3, nil
	default:
		return 0, fmt.Errorf("unsupported artifact type %q", artifact.ArtifactType)
	}
}

func buildModelDraft(req Request) *ModelDraft {
	busHints := inferBusHints(req.Requirements)
	registers := inferRegisterHints(req.Requirements)
	ir := strictIRDraft(req.ComponentName, busHints, registers)

	return &ModelDraft{
		BusHints:           busHints,
		Registers:          registers,
		StrictIRDraft:      ir,
		BehavioralCoverage: []string{"reset values", "basic register accessibility", "simple identity/status behavior"},
		ValidationPlan: []string{
			"Verify required ID/status registers against the request contract.",
			"Validate read/write access classes and reset values in the generated draft.",
			"Run targeted simulator verification before promoting to a reusable catalog model.",
		},
		OpenQuestions: []string{
			"Additional register pages may be required from the datasheet for full fidelity.",
			"Timing hooks and interrupt semantics still need datasheet-grounded verification.",
		},
	}
}

func pick(value string, ok bool, fallback string) string {
	if ok && value != "" {
		return value
	}
	return fallback
}

func chipFileName(chipGuess, boardID string) string {
	if chipGuess != "" {
		return chipGuess
	}
	return boardID + "_chip"
}

func inferBusHints(requirements string) []string {
	lower := strings.ToLower(requirements)
	hints := []string{}
	if strings.Contains(lower, "i2c") {
		hints = append(hints, "i2c")
	}
	if strings.Contains(lower, "spi") {
		hints = append(hints, "spi")
	}
	if strings.Contains(lower, "uart") {
		hints = append(hints, "uart")
	}
	if len(hints) == 0 {
		hints = append(hints, "memory-mapped")
	}
	return hints
}

func inferRegisterHints(requirements string) []RegHint {
	lower := strings.ToLower(requirements)
	idPattern := regexp.MustCompile(`(?i)register\s+(0x[0-9a-f]+).*?(device id|chip id|id).*?(0x[0-9a-f]+)`)
	if matches := idPattern.FindStringSubmatch(requirements); len(matches) == 4 {
		return []RegHint{
			{
				Name:        "ID",
				Offset:      strings.ToLower(matches[1]),
				Access:      "ReadOnly",
				ResetValue:  strings.ToLower(matches[3]),
				Description: strings.TrimSpace(matches[2]) + " register",
			},
		}
	}

	registers := []RegHint{}
	if strings.Contains(lower, "temperature") {
		registers = append(registers, RegHint{
			Name:        "TEMPERATURE",
			Offset:      "0x00",
			Access:      "ReadOnly",
			Description: "Primary sensor output register.",
		})
	}
	if strings.Contains(lower, "status") {
		registers = append(registers, RegHint{
			Name:        "STATUS",
			Offset:      "0x01",
			Access:      "ReadOnly",
			Description: "Status flags required by the request.",
		})
	}
	if len(registers) == 0 {
		registers = append(registers, RegHint{
			Name:        "CONTROL",
			Offset:      "0x00",
			Access:      "ReadWrite",
			Description: "Synthesized control register placeholder derived from requirements.",
		})
	}
	return registers
}

func strictIRDraft(componentName string, busHints []string, registers []RegHint) *StrictIR {
	peripheralID := SanitizeIdent(componentName)
	irRegs := make([]IRRegister, 0, len(registers))
	for _, reg := range registers {
		irRegs = append(irRegs, IRRegister{
			Name:        SanitizeIdent(reg.Name),
			Offset:      ParseHex(reg.Offset),
			Size:        8,
			Access:      MapAccess(reg.Access),
			ResetValue:  ParseHex(reg.ResetValue),
			Fields:      []IRField{},
			Description: reg.Description,
		})
	}

	description := "Synthesized strict IR draft."
	if len(busHints) > 0 {
		description = fmt.Sprintf("Synthesized strict IR draft with %s bus assumptions.", strings.Join(busHints, ", "))
	}

	return &StrictIR{
		Name:        peripheralID,
		Arch:        "Arm",
		Description: description,
		Peripherals: map[string]IRPeripheral{
			peripheralID: {
				Name:        peripheralID,
				BaseAddress: 0x40000000,
				Description: description,
				Registers:   irRegs,
				Interrupts:  []IRInterrupt{},
				Timing:      []IRTimingHook{},
			},
		},
		InterruptMapping: map[string]int{},
	}
}

func maybeGenerateSummary(ctx context.Context, req Request, artifactType string) (string, error) {
	if strings.TrimSpace(strings.ToLower(req.ComponentName)) == "" {
		return "", fmt.Errorf("missing component name")
	}
	if strings.TrimSpace(strings.ToLower(req.Requirements)) == "" {
		return "", fmt.Errorf("missing requirements")
	}
	if strings.TrimSpace(os.Getenv("XAI_API_KEY")) == "" {
		return "", fmt.Errorf("xai key not configured")
	}

	client := NewLLMClient()
	systemPrompt := "You produce short, concrete synthesis summaries for hardware onboarding artifacts. Stay factual and under 80 words."
	userPrompt := fmt.Sprintf(
		"Artifact type: %s\nComponent: %s\nRequirements: %s\nDatasheet URL: %s\nReturn one concise paragraph summarizing what this synthesis artifact is for and what it intentionally does not guarantee.",
		artifactType,
		req.ComponentName,
		req.Requirements,
		req.DatasheetURL,
	)
	return client.Complete(ctx, systemPrompt, userPrompt)
}

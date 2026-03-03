package synthesis

type IRField struct {
	Name        string `json:"name"`
	BitOffset   int    `json:"bit_offset"`
	BitWidth    int    `json:"bit_width"`
	Access      string `json:"access,omitempty"`
	Description string `json:"description"`
}

type IRRegister struct {
	Name        string    `json:"name"`
	Offset      int64     `json:"offset"`
	Size        int       `json:"size"`
	Access      string    `json:"access"`
	ResetValue  int64     `json:"reset_value"`
	Fields      []IRField `json:"fields"`
	SideEffects any       `json:"side_effects,omitempty"`
	Description string    `json:"description"`
}

type IRTimingHook struct {
	ID           string `json:"id"`
	Trigger      any    `json:"trigger"`
	DelayCycles  int    `json:"delay_cycles"`
	Action       any    `json:"action"`
	Interrupt    string `json:"interrupt,omitempty"`
	Reasoning    string `json:"reasoning,omitempty"`
	Evidence     string `json:"evidence,omitempty"`
}

type IRInterrupt struct {
	Name  string `json:"name"`
	Value int    `json:"value"`
}

type IRPeripheral struct {
	Name        string         `json:"name"`
	BaseAddress int64          `json:"base_address"`
	Description string         `json:"description"`
	Registers   []IRRegister   `json:"registers"`
	Interrupts  []IRInterrupt  `json:"interrupts"`
	Timing      []IRTimingHook `json:"timing"`
}

type StrictIR struct {
	Name             string                  `json:"name"`
	Arch             string                  `json:"arch"`
	Description      string                  `json:"description"`
	Peripherals      map[string]IRPeripheral `json:"peripherals"`
	InterruptMapping map[string]int          `json:"interrupt_mapping"`
}

// YAML input structures
type YamlField struct {
	Name        string `yaml:"name"`
	BitRange    []int  `yaml:"bit_range"`
	Description string `yaml:"description"`
}

type YamlRegister struct {
	Name        string      `yaml:"name"`
	Offset      any         `yaml:"offset"`
	ResetValue  any         `yaml:"reset_value"`
	Access      string      `yaml:"access"`
	Fields      []YamlField `yaml:"fields"`
	Description string      `yaml:"description"`
}

type YamlPeripheral struct {
	Name        string         `yaml:"name"`
	Registers   []YamlRegister `yaml:"registers"`
	SideEffects []any          `yaml:"side_effects"`
}

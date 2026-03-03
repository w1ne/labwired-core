package catalog

type Asset struct {
	ID          string `json:"id"`
	Name        string `json:"name"`
	Description string `json:"description"`
	PassRate    int    `json:"pass_rate"`
	Registers   int    `json:"registers"`
	IrURL       string `json:"ir_url"`
}

type Manager struct {
	assets map[string]Asset
}

func NewManager() *Manager {
	m := &Manager{
		assets: make(map[string]Asset),
	}
	m.seed()
	return m
}

func (m *Manager) seed() {
	m.assets["ADXL345"] = Asset{
		ID:          "ADXL345",
		Name:        "Analog Devices ADXL345",
		Description: "3-Axis Digital Accelerometer with I2C/SPI interface.",
		PassRate:    100,
		Registers:   29,
		IrURL:       "/artifacts/catalog/adxl345.json",
	}
	m.assets["BME280"] = Asset{
		ID:          "BME280",
		Name:        "Bosch BME280",
		Description: "Digital Humidity, Pressure and Temperature Sensor.",
		PassRate:    100,
		Registers:   18,
		IrURL:       "/artifacts/catalog/bme280.json",
	}
	m.assets["MCP2515"] = Asset{
		ID:          "MCP2515",
		Name:        "Microchip MCP2515",
		Description: "Stand-Alone CAN Controller with SPI Interface.",
		PassRate:    98,
		Registers:   52,
		IrURL:       "/artifacts/catalog/mcp2515.json",
	}
}

func (m *Manager) List() []Asset {
	res := make([]Asset, 0, len(m.assets))
	for _, a := range m.assets {
		res = append(res, a)
	}
	return res
}

func (m *Manager) Get(id string) (Asset, bool) {
	a, ok := m.assets[id]
	return a, ok
}

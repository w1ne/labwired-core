package catalog

import (
	"fmt"
	"io/fs"
	"log"
	"os"
	"path/filepath"
	"strings"

	"github.com/labwired/foundry-backend/internal/db"
	"gopkg.in/yaml.v3"
)

type Manager struct {
	store *db.Store
}

var knownOfficialBoardURLs = map[string]string{
	"board/arduino-nano-33-ble":   "https://docs.arduino.cc/hardware/nano-33-ble/",
	"board/arduino-uno-r4-minima": "https://docs.arduino.cc/hardware/uno-r4-minima/",
	"board/nucleo-f401re":         "https://www.st.com/en/evaluation-tools/nucleo-f401re.html",
	"board/nucleo-h563zi-demo":    "https://www.st.com/en/evaluation-tools/nucleo-h563zi.html",
	"chip/rp2040":                 "https://www.raspberrypi.com/documentation/microcontrollers/rp2040.html",
}

// knownBoardImageURLs maps catalog asset IDs to stable manufacturer product images.
// Only includes images from official manufacturer CDNs or documentation sites.
var knownBoardImageURLs = map[string]string{
	// ST Nucleo boards
	"board/nucleo-f401re":      "https://www.st.com/bin/ecommerce/api/image.PF260320.en.feature-description-include-personalized-no-cpn-large.jpg",
	"board/nucleo-h563zi":      "https://www.st.com/bin/ecommerce/api/image.PF272352.en.feature-description-include-personalized-no-cpn-large.jpg",
	"board/nucleo-h563zi-demo": "https://www.st.com/bin/ecommerce/api/image.PF272352.en.feature-description-include-personalized-no-cpn-large.jpg",
	// Raspberry Pi
	"chip/rp2040":    "https://www.raspberrypi.com/app/uploads/2020/12/rp2040-top-1-300x300.png",
	"board/rpi-pico": "https://www.raspberrypi.com/app/uploads/2020/12/pico-board-top-315x237.png",
	// Arduino
	"board/arduino-nano-33-ble":     "https://docs.arduino.cc/static/media/arduino-nano-33-ble.svg",
	"board/arduino-uno-r4-minima":   "https://docs.arduino.cc/static/media/arduino-uno-r4-minima.svg",
	"board/arduino-uno-r4-wifi":     "https://docs.arduino.cc/static/media/arduino-uno-r4-wifi.svg",
	"board/arduino-nano-33-ble-rev2": "https://docs.arduino.cc/static/media/arduino-nano-33-ble.svg",
	// Nordic nRF52
	"chip/nrf52840": "https://docs.nordicsemi.com/bundle/ncs-latest/page/nrf/images/nrf52840.png",
	"chip/nrf52832": "https://docs.nordicsemi.com/bundle/ncs-latest/page/nrf/images/nrf52832.png",
	// Espressif
	"chip/esp32c3": "https://www.espressif.com/sites/default/files/modules/ESP32-C3-MINI-1_v1.0.png",
	"chip/esp32":   "https://www.espressif.com/sites/default/files/modules/ESP32_v3.2.png",
}

func imageURLForAssetID(assetID string) string {
	if v, ok := knownBoardImageURLs[assetID]; ok {
		return v
	}
	return ""
}

func NewManager(store *db.Store) *Manager {
	return &Manager{
		store: store,
	}
}

func slugifyCatalogPart(v string) string {
	s := strings.ToLower(strings.TrimSpace(v))
	s = strings.ReplaceAll(s, "_", "-")
	s = strings.ReplaceAll(s, " ", "-")
	s = strings.Trim(s, "-")
	if s == "" {
		return "unknown"
	}
	return s
}

func standardizeCatalogName(raw string) string {
	raw = strings.TrimSpace(raw)
	if raw == "" {
		return ""
	}
	replaced := strings.NewReplacer("_", " ", "-", " ").Replace(raw)
	parts := strings.Fields(replaced)
	if len(parts) == 0 {
		return ""
	}
	acronyms := map[string]struct{}{
		"acrn": {}, "adc": {}, "arm": {}, "ble": {}, "can": {}, "cpu": {}, "dma": {}, "eth": {},
		"fpga": {}, "gpio": {}, "i2c": {}, "i2s": {}, "irq": {}, "lte": {}, "mcu": {}, "nvic": {},
		"pwm": {}, "qspi": {}, "rcc": {}, "riscv": {}, "sdio": {}, "soc": {}, "spi": {}, "uart": {},
		"usb": {}, "wifi": {},
	}
	for i, p := range parts {
		lower := strings.ToLower(p)
		if _, ok := acronyms[lower]; ok || strings.IndexFunc(p, func(r rune) bool { return r >= '0' && r <= '9' }) >= 0 {
			parts[i] = strings.ToUpper(p)
			continue
		}
		if len(p) <= 3 {
			parts[i] = strings.ToUpper(p)
			continue
		}
		parts[i] = strings.ToUpper(p[:1]) + strings.ToLower(p[1:])
	}
	return strings.Join(parts, " ")
}

func catalogIDFromCorePath(relPath string, fallbackName string) string {
	relPath = filepath.ToSlash(strings.TrimSpace(relPath))
	stem := strings.TrimSuffix(filepath.Base(relPath), filepath.Ext(relPath))
	label := stem
	if strings.TrimSpace(fallbackName) != "" {
		label = fallbackName
	}
	if label == "" {
		label = "unknown"
	}
	parts := strings.Split(relPath, "/")
	root := ""
	if len(parts) > 0 {
		root = parts[0]
	}
	switch root {
	case "onboarding", "boards", "systems":
		return "board/" + slugifyCatalogPart(label)
	case "chips":
		return "chip/" + slugifyCatalogPart(label)
	case "peripherals":
		return "peripheral/" + slugifyCatalogPart(label)
	default:
		return "catalog/" + slugifyCatalogPart(label)
	}
}

func catalogIDFromHardwareItem(item db.HardwareItem) string {
	name := strings.TrimSpace(item.Name)
	if name == "" {
		name = strings.TrimSpace(item.ID)
	}
	if name == "" {
		name = strings.TrimSuffix(filepath.Base(strings.TrimSpace(item.ReplPath)), filepath.Ext(strings.TrimSpace(item.ReplPath)))
	}
	prefix := "board"
	if strings.EqualFold(strings.TrimSpace(item.Type), "cpu") {
		prefix = "cpu"
	}
	return prefix + "/" + slugifyCatalogPart(name)
}

func sourceURLForCoreConfig(relPath string) string {
	relPath = filepath.ToSlash(strings.TrimSpace(relPath))
	if relPath == "" {
		return ""
	}
	return "https://github.com/w1ne/labwired-core/blob/main/configs/" + relPath
}
func sourceURLForHardwareItem(item db.HardwareItem) string {
	ref := filepath.ToSlash(strings.TrimSpace(item.ReplPath))
	if ref == "" {
		return ""
	}
	if strings.HasPrefix(ref, "platforms/") {
		return "https://github.com/renode/renode/blob/master/" + ref
	}
	if strings.HasPrefix(ref, "core/configs/") {
		return "https://github.com/w1ne/labwired-core/blob/main/configs/" + strings.TrimPrefix(ref, "core/configs/")
	}
	if strings.HasPrefix(ref, "onboarding/") || strings.HasPrefix(ref, "chips/") || strings.HasPrefix(ref, "systems/") || strings.HasPrefix(ref, "peripherals/") {
		return "https://github.com/w1ne/labwired-core/blob/main/configs/" + ref
	}
	return ""
}

func officialURLForAssetID(assetID string) string {
	if v, ok := knownOfficialBoardURLs[assetID]; ok {
		return v
	}
	return ""
}

func validationURLFromModel(runURL, artifactsURL string) string {
	runURL = strings.TrimSpace(runURL)
	artifactsURL = strings.TrimSpace(artifactsURL)
	if artifactsURL != "" {
		return artifactsURL
	}
	if runURL != "" {
		return runURL
	}
	return ""
}

func splitCatalogID(id string) (string, string, bool) {
	parts := strings.SplitN(strings.TrimSpace(id), "/", 2)
	if len(parts) != 2 {
		return "", "", false
	}
	kind := strings.TrimSpace(parts[0])
	slug := strings.TrimSpace(parts[1])
	if kind == "" || slug == "" {
		return "", "", false
	}
	return kind, slug, true
}

func isCanonicalCoreConfigRef(ref string) bool {
	ref = filepath.ToSlash(strings.TrimSpace(ref))
	return strings.HasPrefix(ref, "onboarding/")
}

// dedupeBoardChipAliases keeps board rows as canonical and hides chip rows with the same slug.
func dedupeBoardChipAliases(assets []db.CatalogAsset) []db.CatalogAsset {
	boardSlugs := make(map[string]struct{}, len(assets))
	chipBySlug := make(map[string]db.CatalogAsset, len(assets))
	for _, a := range assets {
		kind, slug, ok := splitCatalogID(a.ID)
		if !ok {
			continue
		}
		if kind == "board" {
			boardSlugs[slug] = struct{}{}
			continue
		}
		if kind == "chip" {
			chipBySlug[slug] = a
		}
	}

	out := make([]db.CatalogAsset, 0, len(assets))
	for _, a := range assets {
		kind, slug, ok := splitCatalogID(a.ID)
		if ok && kind == "board" {
			if chip, hasChip := chipBySlug[slug]; hasChip {
				// Keep board as canonical row but backfill missing metadata from chip alias.
				if strings.TrimSpace(a.Architecture) == "" && strings.TrimSpace(chip.Architecture) != "" {
					a.Architecture = chip.Architecture
				}
				if strings.TrimSpace(a.Family) == "" && strings.TrimSpace(chip.Family) != "" {
					a.Family = chip.Family
				}
			}
		}
		if ok && kind == "chip" {
			if _, hasBoard := boardSlugs[slug]; hasBoard {
				continue
			}
		}
		out = append(out, a)
	}
	return out
}

// SyncFromDisk scans the provided directory for YAML models and upserts them to the DB.
func (m *Manager) SyncFromDisk(configsDir string) error {
	log.Printf("[catalog] syncing models from disk: %s", configsDir)

	err := filepath.WalkDir(configsDir, func(path string, d fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if d.IsDir() {
			return nil
		}
		if !strings.HasSuffix(path, ".yaml") && !strings.HasSuffix(path, ".yml") {
			return nil
		}

		data, err := os.ReadFile(path)
		if err != nil {
			log.Printf("[catalog] failed to read model %s: %v", path, err)
			return nil
		}

		var model struct {
			Name           string `yaml:"name"`
			Description    string `yaml:"description"`
			URL            string `yaml:"url"`
			Image          string `yaml:"image"`
			Family         string `yaml:"family"`
			CodeExample    string `yaml:"code_example"`
			SampleTrace    string `yaml:"sample_trace"`
			RegistersCount *int   `yaml:"registers_count"`
			PassRate       *int   `yaml:"pass_rate"`
			Verified       *bool  `yaml:"verified"`
			Validation     struct {
				Method       string `yaml:"method"`
				RunURL       string `yaml:"run_url"`
				ArtifactsURL string `yaml:"artifacts_url"`
			} `yaml:"validation"`
		}
		if err := yaml.Unmarshal(data, &model); err != nil {
			log.Printf("[catalog] failed to parse model %s: %v", path, err)
			return nil
		}

		arch := ""
		if strings.Contains(model.Description, "Architecture: ") {
			parts := strings.Split(model.Description, "Architecture: ")
			if len(parts) > 1 {
				arch = strings.TrimSpace(parts[1])
			}
		}

		relPath, relErr := filepath.Rel(configsDir, path)
		if relErr != nil {
			relPath = d.Name()
		}
		relPath = filepath.ToSlash(relPath)
		name := standardizeCatalogName(model.Name)
		if name == "" {
			name = standardizeCatalogName(strings.TrimSuffix(d.Name(), filepath.Ext(d.Name())))
		}
		id := catalogIDFromCorePath(relPath, name)

		passRate := 0
		if model.PassRate != nil {
			passRate = *model.PassRate
		}

		verified := false
		if model.Verified != nil {
			verified = *model.Verified
		}

		registers := countRegistersInYAMLModel(data)
		if model.RegistersCount != nil {
			registers = *model.RegistersCount
		}

		asset := db.CatalogAsset{
			ID:            id,
			Name:          name,
			Description:   model.Description,
			Family:        model.Family,
			Architecture:  arch,
			CodeExample:   model.CodeExample,
			PassRate:      passRate,
			Registers:     registers,
			IrURL:         "",
			Verified:      verified,
			SourceType:    "core-config",
			SourceRef:     relPath,
			SourceURL:     sourceURLForCoreConfig(relPath),
			OfficialURL:   strings.TrimSpace(model.URL),
			ValidationURL: validationURLFromModel(model.Validation.RunURL, model.Validation.ArtifactsURL),
			ImageURL:      strings.TrimSpace(model.Image),
		}
		if asset.OfficialURL == "" {
			asset.OfficialURL = officialURLForAssetID(id)
		}
		if asset.ImageURL == "" {
			asset.ImageURL = imageURLForAssetID(id)
		}

		if model.Description == "" {
			if strings.Contains(path, "/chips/") {
				asset.Description = fmt.Sprintf("Hardware model for %s chip.", name)
			} else if strings.Contains(path, "/peripherals/") {
				asset.Description = fmt.Sprintf("Peripheral model for %s.", name)
			} else {
				asset.Description = fmt.Sprintf("Simulation profile for %s.", name)
			}
		}

		if existing, ok, err := m.store.GetCatalogAsset(asset.ID); err != nil {
			log.Printf("[catalog] failed to inspect existing asset %s: %v", asset.ID, err)
		} else if ok {
			existingIsCanonical := isCanonicalCoreConfigRef(existing.SourceRef)
			assetIsCanonical := isCanonicalCoreConfigRef(asset.SourceRef)
			preferExisting := existingIsCanonical && !assetIsCanonical

			if strings.TrimSpace(asset.Description) == "" || preferExisting {
				asset.Description = existing.Description
			}
			if strings.TrimSpace(asset.Family) == "" || preferExisting {
				asset.Family = existing.Family
			}
			if strings.TrimSpace(asset.Architecture) == "" || preferExisting {
				asset.Architecture = existing.Architecture
			}
			if strings.TrimSpace(asset.CodeExample) == "" || preferExisting {
				asset.CodeExample = existing.CodeExample
			}
			if asset.Registers == 0 || preferExisting {
				asset.Registers = existing.Registers
			}
			if asset.PassRate == 0 || preferExisting {
				asset.PassRate = existing.PassRate
			}
			if !asset.Verified || preferExisting {
				asset.Verified = asset.Verified || existing.Verified
			}
			if strings.TrimSpace(asset.IrURL) == "" {
				asset.IrURL = existing.IrURL
			}
			if strings.TrimSpace(asset.ValidationURL) == "" || preferExisting {
				asset.ValidationURL = existing.ValidationURL
			}
			if strings.TrimSpace(asset.OfficialURL) == "" || preferExisting {
				asset.OfficialURL = existing.OfficialURL
			}
			if strings.TrimSpace(asset.SourceURL) == "" || preferExisting {
				asset.SourceURL = existing.SourceURL
			}
			if strings.TrimSpace(asset.SourceRef) == "" || preferExisting {
				asset.SourceRef = existing.SourceRef
			}
			if strings.TrimSpace(asset.ImageURL) == "" {
				asset.ImageURL = existing.ImageURL
			}
		}

		if err := m.store.UpsertCatalogAsset(asset); err != nil {
			log.Printf("[catalog] failed to upsert asset %s: %v", id, err)
		}

		return nil
	})

	return err
}

// RebuildGitBackedCatalog replaces git-derived catalog rows with a fresh import.
func (m *Manager) RebuildGitBackedCatalog(configsDir string, indexItems []db.HardwareItem) error {
	if err := m.store.DeleteCatalogAssetsBySourceTypes([]string{"core-config", "platform-catalog"}); err != nil {
		return err
	}
	if err := m.SyncFromDisk(configsDir); err != nil {
		return err
	}
	if len(indexItems) > 0 {
		if err := m.SyncFromHardwareIndex(indexItems); err != nil {
			return err
		}
	}
	return nil
}

// PromoteToCatalog saves a synthesized model to persistent storage and adds it to the catalog.
func (m *Manager) PromoteToCatalog(asset db.CatalogAsset, modelData []byte, dataDir string) error {
	// 1. Ensure persistent directory exists
	catalogDir := filepath.Join(dataDir, "catalog")
	if err := os.MkdirAll(catalogDir, 0755); err != nil {
		return fmt.Errorf("failed to create catalog directory: %w", err)
	}

	// 2. Save model file
	fileName := fmt.Sprintf("%s.json", asset.ID)
	filePath := filepath.Join(catalogDir, fileName)
	if err := os.WriteFile(filePath, modelData, 0644); err != nil {
		return fmt.Errorf("failed to save model file: %w", err)
	}

	// 3. Update asset with local URL
	asset.IrURL = fmt.Sprintf("/data/catalog/%s", fileName)
	asset.Verified = true
	if asset.SourceType == "" {
		asset.SourceType = "synthesized"
	}
	if asset.SourceRef == "" {
		asset.SourceRef = filePath
	}

	// 4. Upsert to DB
	return m.store.UpsertCatalogAsset(asset)
}

func (m *Manager) List() []db.CatalogAsset {
	assets, err := m.store.ListCatalogAssets()
	if err != nil {
		log.Printf("[catalog] failed to list assets: %v", err)
		return []db.CatalogAsset{}
	}
	return dedupeBoardChipAliases(assets)
}

func (m *Manager) Get(id string) (db.CatalogAsset, bool) {
	asset, ok, err := m.store.GetCatalogAsset(id)
	if err != nil {
		log.Printf("[catalog] failed to get asset %s: %v", id, err)
		return db.CatalogAsset{}, false
	}
	return asset, ok
}

// SyncFromHardwareIndex upserts external hardware index rows into the unified catalog.
// This keeps /v1/catalog and /v1/hardware aligned even for non-core board entries.
func (m *Manager) SyncFromHardwareIndex(items []db.HardwareItem) error {
	for _, item := range items {
		name := standardizeCatalogName(item.Name)
		if name == "" {
			name = standardizeCatalogName(item.ID)
		}
		if name == "" {
			continue
		}

		verified := item.Tier <= 1
		passRate := 0
		if verified {
			passRate = 100
		}

		asset := db.CatalogAsset{
			ID:            catalogIDFromHardwareItem(item),
			Name:          name,
			Description:   fmt.Sprintf("Simulation profile for %s.", name),
			Family:        "",
			Architecture:  "",
			CodeExample:   "",
			PassRate:      passRate,
			Registers:     0,
			IrURL:         "",
			Verified:      verified,
			SourceType:    "platform-catalog",
			SourceRef:     strings.TrimSpace(item.ReplPath),
			SourceURL:     sourceURLForHardwareItem(item),
			ValidationURL: "",
		}
		if asset.ID == "" {
			continue
		}
		if asset.SourceRef == "" {
			asset.SourceRef = asset.ID
		}
		asset.OfficialURL = officialURLForAssetID(asset.ID)
		asset.ImageURL = imageURLForAssetID(asset.ID)

		// Preserve richer metadata imported from core configs when the hardware index
		// references the same logical board/chip ID.
		if existing, ok, err := m.store.GetCatalogAsset(asset.ID); err != nil {
			log.Printf("[catalog] failed to inspect existing asset %s: %v", asset.ID, err)
		} else if ok {
			if strings.TrimSpace(existing.Description) != "" {
				asset.Description = existing.Description
			}
			if strings.TrimSpace(existing.Family) != "" {
				asset.Family = existing.Family
			}
			if strings.TrimSpace(existing.Architecture) != "" {
				asset.Architecture = existing.Architecture
			}
			if strings.TrimSpace(existing.CodeExample) != "" {
				asset.CodeExample = existing.CodeExample
			}
			if existing.Registers > 0 {
				asset.Registers = existing.Registers
			}
			if strings.TrimSpace(existing.IrURL) != "" {
				asset.IrURL = existing.IrURL
			}
			if strings.TrimSpace(existing.SourceURL) != "" && asset.SourceURL == "" {
				asset.SourceURL = existing.SourceURL
			}
			if strings.TrimSpace(existing.ValidationURL) != "" {
				asset.ValidationURL = existing.ValidationURL
			}
			if strings.TrimSpace(existing.OfficialURL) != "" {
				asset.OfficialURL = existing.OfficialURL
			}
			if strings.TrimSpace(existing.ImageURL) != "" {
				asset.ImageURL = existing.ImageURL
			}
		}

		if err := m.store.UpsertCatalogAsset(asset); err != nil {
			log.Printf("[catalog] failed to upsert indexed asset %s: %v", asset.ID, err)
		}
	}
	return nil
}

func countRegistersInYAMLModel(data []byte) int {
	var parsed any
	if err := yaml.Unmarshal(data, &parsed); err != nil {
		return 0
	}
	return countRegistersInNode(parsed)
}

func countRegistersInNode(v any) int {
	switch node := v.(type) {
	case map[string]any:
		total := 0
		for k, child := range node {
			if k == "registers" {
				if regs, ok := child.([]any); ok {
					total += len(regs)
					continue
				}
			}
			total += countRegistersInNode(child)
		}
		return total
	case map[any]any:
		total := 0
		for k, child := range node {
			if ks, ok := k.(string); ok && ks == "registers" {
				if regs, ok := child.([]any); ok {
					total += len(regs)
					continue
				}
			}
			total += countRegistersInNode(child)
		}
		return total
	case []any:
		total := 0
		for _, child := range node {
			total += countRegistersInNode(child)
		}
		return total
	default:
		return 0
	}
}

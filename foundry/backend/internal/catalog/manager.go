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

func NewManager(store *db.Store) *Manager {
	return &Manager{
		store: store,
	}
}

// SyncFromDisk scans the provided directory for YAML models and upserts them to the DB.
func (m *Manager) SyncFromDisk(configsDir string) error {
	log.Printf("[catalog] syncing models from disk: %s", configsDir)

	err := filepath.WalkDir(configsDir, func(path string, d fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if d.IsDir() || (!strings.HasSuffix(path, ".yaml") && !strings.HasSuffix(path, ".yml")) {
			return nil
		}

		data, err := os.ReadFile(path)
		if err != nil {
			log.Printf("[catalog] failed to read model %s: %v", path, err)
			return nil
		}

		var model struct {
			Name        string `yaml:"name"`
			Description string `yaml:"description"`
		}
		if err := yaml.Unmarshal(data, &model); err != nil {
			log.Printf("[catalog] failed to parse model %s: %v", path, err)
			return nil
		}

		id := strings.TrimSuffix(d.Name(), filepath.Ext(d.Name()))
		name := model.Name
		if name == "" {
			name = id
		}

		asset := db.CatalogAsset{
			ID:          id,
			Name:        name,
			Description: model.Description,
			PassRate:    100, // Default for pre-verified repo models
			Registers:   0,   // Could be parsed if needed
			IrURL:       "",  // TBD: how to resolve URLs for GitHub models
		}

		if model.Description == "" {
			if strings.Contains(path, "/chips/") {
				asset.Description = fmt.Sprintf("Hardware model for %s chip.", name)
			} else if strings.Contains(path, "/peripherals/") {
				asset.Description = fmt.Sprintf("Peripheral model for %s.", name)
			} else {
				asset.Description = fmt.Sprintf("LabWired hardware model: %s", name)
			}
		}

		if err := m.store.UpsertCatalogAsset(asset); err != nil {
			log.Printf("[catalog] failed to upsert asset %s: %v", id, err)
		}

		return nil
	})

	return err
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

	// 4. Upsert to DB
	return m.store.UpsertCatalogAsset(asset)
}

func (m *Manager) List() []db.CatalogAsset {
	assets, err := m.store.ListCatalogAssets()
	if err != nil {
		log.Printf("[catalog] failed to list assets: %v", err)
		return []db.CatalogAsset{}
	}
	return assets
}

func (m *Manager) Get(id string) (db.CatalogAsset, bool) {
	asset, ok, err := m.store.GetCatalogAsset(id)
	if err != nil {
		log.Printf("[catalog] failed to get asset %s: %v", id, err)
		return db.CatalogAsset{}, false
	}
	return asset, ok
}

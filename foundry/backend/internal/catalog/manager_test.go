package catalog

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/labwired/foundry-backend/internal/db"
)

func newTestStore(t *testing.T) *db.Store {
	t.Helper()
	store, err := db.NewStore(filepath.Join(t.TempDir(), "catalog_test.db"))
	if err != nil {
		t.Fatalf("NewStore failed: %v", err)
	}
	t.Cleanup(func() { _ = store.Close() })
	return store
}

func TestSyncFromDisk_UsesUniquePathIDsAndProvenance(t *testing.T) {
	store := newTestStore(t)
	mgr := NewManager(store)
	root := t.TempDir()

	boardDir := filepath.Join(root, "boards")
	periphDir := filepath.Join(root, "peripherals")
	if err := os.MkdirAll(boardDir, 0o755); err != nil {
		t.Fatalf("MkdirAll board dir failed: %v", err)
	}
	if err := os.MkdirAll(periphDir, 0o755); err != nil {
		t.Fatalf("MkdirAll periph dir failed: %v", err)
	}

	boardYAML := []byte("name: DemoBoard\nregisters_count: 42\npass_rate: 100\nverified: true\nregisters:\n  - { name: A }\n  - { name: B }\n")
	periphYAML := []byte("name: DemoPeriph\nregisters:\n  - { name: C }\n")
	if err := os.WriteFile(filepath.Join(boardDir, "dup.yaml"), boardYAML, 0o644); err != nil {
		t.Fatalf("write board yaml failed: %v", err)
	}
	if err := os.WriteFile(filepath.Join(periphDir, "dup.yaml"), periphYAML, 0o644); err != nil {
		t.Fatalf("write periph yaml failed: %v", err)
	}

	if err := mgr.SyncFromDisk(root); err != nil {
		t.Fatalf("SyncFromDisk failed: %v", err)
	}

	assets := mgr.List()
	if len(assets) != 2 {
		t.Fatalf("expected 2 assets, got %d", len(assets))
	}

	seen := map[string]db.CatalogAsset{}
	for _, a := range assets {
		seen[a.ID] = a
	}

	boardAsset, ok := seen["boards/dup"]
	if !ok {
		t.Fatalf("expected boards/dup asset")
	}
	if boardAsset.Registers != 42 {
		t.Fatalf("expected board registers=42 (override), got %d", boardAsset.Registers)
	}
	if !boardAsset.Verified {
		t.Fatalf("expected disk-synced asset to be verified")
	}
	if boardAsset.PassRate != 100 {
		t.Fatalf("expected pass rate 100, got %d", boardAsset.PassRate)
	}
	if boardAsset.SourceType != "core-config" {
		t.Fatalf("expected source_type core-config, got %q", boardAsset.SourceType)
	}
	if boardAsset.SourceRef != "boards/dup.yaml" {
		t.Fatalf("unexpected source_ref: %q", boardAsset.SourceRef)
	}

	periphAsset, ok := seen["peripherals/dup"]
	if !ok {
		t.Fatalf("expected peripherals/dup asset")
	}
	if periphAsset.Registers != 1 {
		t.Fatalf("expected peripheral registers=1, got %d", periphAsset.Registers)
	}
}

func TestPromoteToCatalog_MarksVerifiedAndSetsSource(t *testing.T) {
	store := newTestStore(t)
	mgr := NewManager(store)

	err := mgr.PromoteToCatalog(
		db.CatalogAsset{
			ID:         "asset-1",
			Name:       "Asset One",
			SourceType: "",
		},
		[]byte(`{"ok":true}`),
		t.TempDir(),
	)
	if err != nil {
		t.Fatalf("PromoteToCatalog failed: %v", err)
	}

	asset, ok := mgr.Get("asset-1")
	if !ok {
		t.Fatalf("expected promoted asset to exist")
	}
	if !asset.Verified {
		t.Fatalf("expected promoted asset to be marked verified")
	}
	if asset.SourceType != "synthesized" {
		t.Fatalf("expected source_type synthesized, got %q", asset.SourceType)
	}
	if asset.SourceRef == "" {
		t.Fatalf("expected source_ref to be set")
	}
	if asset.IrURL == "" {
		t.Fatalf("expected ir_url to be set")
	}
}

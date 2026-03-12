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

	boardAsset, ok := seen["board/demoboard"]
	if !ok {
		t.Fatalf("expected board/demoboard asset")
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

	periphAsset, ok := seen["peripheral/demoperiph"]
	if !ok {
		t.Fatalf("expected peripheral/demoperiph asset")
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

func TestSyncFromHardwareIndex_ImportsExternalBoardsIntoCatalog(t *testing.T) {
	store := newTestStore(t)
	mgr := NewManager(store)

	items := []db.HardwareItem{
		{
			ID:       "board-ext-a",
			Name:     "ext-a",
			Type:     "board",
			ReplPath: "platforms/boards/ext-a.repl",
			Tier:     1,
		},
		{
			ID:       "board-ext-b",
			Name:     "ext-b",
			Type:     "board",
			ReplPath: "platforms/boards/ext-b.repl",
			Tier:     2,
		},
	}

	if err := mgr.SyncFromHardwareIndex(items); err != nil {
		t.Fatalf("SyncFromHardwareIndex failed: %v", err)
	}

	a, ok := mgr.Get("board/ext-a")
	if !ok {
		t.Fatalf("expected board/ext-a to exist")
	}
	if a.SourceType != "platform-catalog" {
		t.Fatalf("expected source_type platform-catalog, got %q", a.SourceType)
	}
	if !a.Verified || a.PassRate != 100 {
		t.Fatalf("expected tier-1 board to map to verified=true pass_rate=100, got verified=%v pass_rate=%d", a.Verified, a.PassRate)
	}

	b, ok := mgr.Get("board/ext-b")
	if !ok {
		t.Fatalf("expected board/ext-b to exist")
	}
	if b.Verified || b.PassRate != 0 {
		t.Fatalf("expected tier-2 board to map to verified=false pass_rate=0, got verified=%v pass_rate=%d", b.Verified, b.PassRate)
	}
	if b.SourceRef != "platforms/boards/ext-b.repl" {
		t.Fatalf("unexpected source_ref: %q", b.SourceRef)
	}
}

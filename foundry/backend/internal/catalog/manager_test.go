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

	boardYAML := []byte("name: DemoBoard\nregisters_count: 42\npass_rate: 100\nverified: true\nvalidation:\n  run_url: https://github.com/example/repo/actions/runs/123\n  artifacts_url: https://github.com/example/repo/actions/runs/123#artifacts\nregisters:\n  - { name: A }\n  - { name: B }\n")
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
	if boardAsset.SourceURL == "" {
		t.Fatalf("expected source_url to be populated")
	}
	if boardAsset.ValidationURL != "https://github.com/example/repo/actions/runs/123#artifacts" {
		t.Fatalf("unexpected validation_url: %q", boardAsset.ValidationURL)
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
	if b.SourceURL != "https://github.com/renode/renode/blob/master/platforms/boards/ext-b.repl" {
		t.Fatalf("unexpected source_url: %q", b.SourceURL)
	}
	if b.ValidationURL != "" {
		t.Fatalf("expected no validation_url for external index import, got %q", b.ValidationURL)
	}
}

func TestSyncFromHardwareIndex_PreservesExistingCoreMetadata(t *testing.T) {
	store := newTestStore(t)
	mgr := NewManager(store)

	if err := store.UpsertCatalogAsset(db.CatalogAsset{
		ID:            "board/ext-a",
		Name:          "Ext A",
		Description:   "Imported from core configs. Architecture: ARM Cortex-M4F.",
		Family:        "ExtFamily",
		Architecture:  "ARM Cortex-M4F",
		CodeExample:   "int main(void) {}",
		Registers:     77,
		PassRate:      65,
		Verified:      false,
		SourceType:    "core-config",
		SourceRef:     "onboarding/ext-a.yaml",
		SourceURL:     "https://example.com/source",
		OfficialURL:   "https://example.com/official",
		ValidationURL: "https://example.com/validation",
	}); err != nil {
		t.Fatalf("seed core-config asset failed: %v", err)
	}

	items := []db.HardwareItem{
		{
			ID:       "board-ext-a",
			Name:     "ext-a",
			Type:     "board",
			ReplPath: "platforms/boards/ext-a.repl",
			Tier:     1,
		},
	}

	if err := mgr.SyncFromHardwareIndex(items); err != nil {
		t.Fatalf("SyncFromHardwareIndex failed: %v", err)
	}

	a, ok := mgr.Get("board/ext-a")
	if !ok {
		t.Fatalf("expected board/ext-a to exist")
	}
	if a.Architecture != "ARM Cortex-M4F" {
		t.Fatalf("expected architecture to be preserved, got %q", a.Architecture)
	}
	if a.ValidationURL != "https://example.com/validation" {
		t.Fatalf("expected validation url to be preserved, got %q", a.ValidationURL)
	}
	if a.Description != "Imported from core configs. Architecture: ARM Cortex-M4F." {
		t.Fatalf("expected description to be preserved, got %q", a.Description)
	}
	if a.Registers != 77 {
		t.Fatalf("expected registers to be preserved, got %d", a.Registers)
	}
	if !a.Verified || a.PassRate != 100 {
		t.Fatalf("expected tier-1 hardware index to upgrade verification, got verified=%v pass_rate=%d", a.Verified, a.PassRate)
	}
	if a.SourceType != "platform-catalog" {
		t.Fatalf("expected source_type platform-catalog after merge, got %q", a.SourceType)
	}
}

func TestSyncFromDiskThenHardwareIndex_PreservesActualCoreMetadata(t *testing.T) {
	store := newTestStore(t)
	mgr := NewManager(store)

	root := t.TempDir()
	onboardingDir := filepath.Join(root, "onboarding")
	if err := os.MkdirAll(onboardingDir, 0o755); err != nil {
		t.Fatalf("MkdirAll onboarding dir failed: %v", err)
	}

	yamlData := []byte("name: a20\ndescription: 'Allwinner A20 Dual-Core ARM Cortex-A7 System-on-Chip. Architecture: ARMv7-A. Used in Cubieboard2 and Olinuxino.'\nfamily: Allwinner\nverified: true\npass_rate: 100\nvalidation:\n  method: local-simulation\n  reason: simulation-ok\n")
	if err := os.WriteFile(filepath.Join(onboardingDir, "a20.yaml"), yamlData, 0o644); err != nil {
		t.Fatalf("write onboarding yaml failed: %v", err)
	}

	if err := mgr.SyncFromDisk(root); err != nil {
		t.Fatalf("SyncFromDisk failed: %v", err)
	}

	if err := mgr.SyncFromHardwareIndex([]db.HardwareItem{
		{
			ID:       "board-a20",
			Name:     "a20",
			Type:     "board",
			ReplPath: "core/configs/onboarding/a20.yaml",
			Tier:     1,
		},
	}); err != nil {
		t.Fatalf("SyncFromHardwareIndex failed: %v", err)
	}

	a, ok := mgr.Get("board/a20")
	if !ok {
		t.Fatalf("expected board/a20 to exist")
	}
	if a.Description != "Allwinner A20 Dual-Core ARM Cortex-A7 System-on-Chip. Architecture: ARMv7-A. Used in Cubieboard2 and Olinuxino." {
		t.Fatalf("expected description to be preserved, got %q", a.Description)
	}
	if a.Architecture != "ARMv7-A. Used in Cubieboard2 and Olinuxino." {
		t.Fatalf("expected architecture to be preserved, got %q", a.Architecture)
	}
}

func TestSyncFromDisk_PrefersCanonicalOnboardingManifestForDuplicateBoardIDs(t *testing.T) {
	store := newTestStore(t)
	mgr := NewManager(store)

	root := t.TempDir()
	onboardingDir := filepath.Join(root, "onboarding")
	systemsDir := filepath.Join(root, "systems", "onboarding")
	if err := os.MkdirAll(onboardingDir, 0o755); err != nil {
		t.Fatalf("MkdirAll onboarding dir failed: %v", err)
	}
	if err := os.MkdirAll(systemsDir, 0o755); err != nil {
		t.Fatalf("MkdirAll systems dir failed: %v", err)
	}

	onboardingYAML := []byte("name: a20\ndescription: 'Allwinner A20 Dual-Core ARM Cortex-A7 System-on-Chip. Architecture: ARMv7-A.'\nurl: https://linux-sunxi.org/A20\nfamily: Allwinner\npass_rate: 100\nverified: true\nsample_trace: traces/a20.txt\nvalidation:\n  method: local-simulation\n")
	systemYAML := []byte("name: a20\ncpu: cortex-a7\n")
	if err := os.WriteFile(filepath.Join(onboardingDir, "a20.yaml"), onboardingYAML, 0o644); err != nil {
		t.Fatalf("write onboarding yaml failed: %v", err)
	}
	if err := os.WriteFile(filepath.Join(systemsDir, "a20.yaml"), systemYAML, 0o644); err != nil {
		t.Fatalf("write system yaml failed: %v", err)
	}

	if err := mgr.SyncFromDisk(root); err != nil {
		t.Fatalf("SyncFromDisk failed: %v", err)
	}

	a, ok := mgr.Get("board/a20")
	if !ok {
		t.Fatalf("expected board/a20 to exist")
	}
	if a.Description != "Allwinner A20 Dual-Core ARM Cortex-A7 System-on-Chip. Architecture: ARMv7-A." {
		t.Fatalf("expected onboarding description to win, got %q", a.Description)
	}
	if a.Architecture != "ARMv7-A." {
		t.Fatalf("expected onboarding architecture to win, got %q", a.Architecture)
	}
	if a.SourceRef != "onboarding/a20.yaml" {
		t.Fatalf("expected onboarding source_ref to win, got %q", a.SourceRef)
	}
	if a.OfficialURL != "https://linux-sunxi.org/A20" {
		t.Fatalf("expected onboarding url to populate official_url, got %q", a.OfficialURL)
	}
	if a.ValidationURL != "https://github.com/w1ne/labwired-core/blob/main/configs/onboarding/traces/a20.txt" {
		t.Fatalf("expected real trace fallback validation url, got %q", a.ValidationURL)
	}
}

func TestList_DedupesChipWhenSameBoardSlugExists(t *testing.T) {
	store := newTestStore(t)
	mgr := NewManager(store)

	if err := store.UpsertCatalogAsset(db.CatalogAsset{
		ID:           "board/a20",
		Name:         "A20",
		Description:  "Board row",
		SourceType:   "platform-catalog",
		Architecture: "",
	}); err != nil {
		t.Fatalf("upsert board failed: %v", err)
	}
	if err := store.UpsertCatalogAsset(db.CatalogAsset{
		ID:           "chip/a20",
		Name:         "A20",
		Description:  "Chip row",
		SourceType:   "core-config",
		Architecture: "ARM 64",
	}); err != nil {
		t.Fatalf("upsert chip failed: %v", err)
	}
	if err := store.UpsertCatalogAsset(db.CatalogAsset{
		ID:          "chip/rp2040",
		Name:        "RP2040",
		Description: "Unique chip row",
		SourceType:  "core-config",
	}); err != nil {
		t.Fatalf("upsert unique chip failed: %v", err)
	}

	assets := mgr.List()
	seen := map[string]bool{}
	for _, a := range assets {
		seen[a.ID] = true
	}

	if !seen["board/a20"] {
		t.Fatalf("expected board/a20 to remain")
	}
	if seen["chip/a20"] {
		t.Fatalf("expected chip/a20 to be hidden when board/a20 exists")
	}
	if !seen["chip/rp2040"] {
		t.Fatalf("expected unique chip/rp2040 to remain")
	}

	var board db.CatalogAsset
	for _, a := range assets {
		if a.ID == "board/a20" {
			board = a
			break
		}
	}
	if board.Architecture != "ARM 64" {
		t.Fatalf("expected board/a20 architecture to be inherited from chip alias, got %q", board.Architecture)
	}
}

package db

import (
	"path/filepath"
	"testing"
	"time"
)

func newTestStore(t *testing.T) *Store {
	t.Helper()
	dbPath := filepath.Join(t.TempDir(), "test.db")
	store, err := NewStore(dbPath)
	if err != nil {
		t.Fatalf("NewStore failed: %v", err)
	}
	t.Cleanup(func() {
		_ = store.Close()
	})
	return store
}

func TestApplyStripeCreditIfNew_AppliesOncePerEventID(t *testing.T) {
	store := newTestStore(t)
	workspaceID := "ws-test-1"

	if _, err := store.CreateKey(workspaceID, "lw_sk_live_test_apply_once"); err != nil {
		t.Fatalf("CreateKey failed: %v", err)
	}

	quotaBefore, err := store.GetMonthlyQuota(workspaceID)
	if err != nil {
		t.Fatalf("GetMonthlyQuota before failed: %v", err)
	}
	if quotaBefore != 1000 {
		t.Fatalf("unexpected initial quota: got=%d want=1000", quotaBefore)
	}

	applied, err := store.ApplyStripeCreditIfNew("evt_1", "cs_1", workspaceID, 1000)
	if err != nil {
		t.Fatalf("ApplyStripeCreditIfNew first call failed: %v", err)
	}
	if !applied {
		t.Fatalf("expected first event application to be applied")
	}

	quotaAfterFirst, err := store.GetMonthlyQuota(workspaceID)
	if err != nil {
		t.Fatalf("GetMonthlyQuota after first failed: %v", err)
	}
	if quotaAfterFirst != 2000 {
		t.Fatalf("unexpected quota after first apply: got=%d want=2000", quotaAfterFirst)
	}

	applied, err = store.ApplyStripeCreditIfNew("evt_1", "cs_1_retry", workspaceID, 5000)
	if err != nil {
		t.Fatalf("ApplyStripeCreditIfNew duplicate call failed: %v", err)
	}
	if applied {
		t.Fatalf("expected duplicate event application to be ignored")
	}

	quotaAfterDuplicate, err := store.GetMonthlyQuota(workspaceID)
	if err != nil {
		t.Fatalf("GetMonthlyQuota after duplicate failed: %v", err)
	}
	if quotaAfterDuplicate != 2000 {
		t.Fatalf("unexpected quota after duplicate apply: got=%d want=2000", quotaAfterDuplicate)
	}

	var eventRows int
	if err := store.db.QueryRow(`SELECT COUNT(*) FROM stripe_events WHERE event_id = ?`, "evt_1").Scan(&eventRows); err != nil {
		t.Fatalf("count stripe_events failed: %v", err)
	}
	if eventRows != 1 {
		t.Fatalf("unexpected stripe event row count: got=%d want=1", eventRows)
	}
}

func TestApplyStripeCreditIfNew_RollsBackWhenWorkspaceMissing(t *testing.T) {
	store := newTestStore(t)

	applied, err := store.ApplyStripeCreditIfNew("evt_missing_ws", "cs_missing_ws", "unknown-workspace", 1000)
	if err == nil {
		t.Fatalf("expected error when workspace is missing")
	}
	if applied {
		t.Fatalf("expected applied=false when workspace is missing")
	}

	var eventRows int
	if err := store.db.QueryRow(`SELECT COUNT(*) FROM stripe_events WHERE event_id = ?`, "evt_missing_ws").Scan(&eventRows); err != nil {
		t.Fatalf("count stripe_events failed: %v", err)
	}
	if eventRows != 0 {
		t.Fatalf("expected rolled back stripe event insert, got rows=%d", eventRows)
	}
}

func TestSeedAndListHardware(t *testing.T) {
	store := newTestStore(t)

	// Initial list should be empty
	items, err := store.ListHardware()
	if err != nil {
		t.Fatalf("ListHardware failed: %v", err)
	}
	if len(items) != 0 {
		t.Fatalf("expected empty hardware list, got %d items", len(items))
	}

	seed := []HardwareItem{
		{
			ID:       "stm32f4_discovery",
			Name:     "stm32f4_discovery",
			Type:     "board",
			ReplPath: "platforms/boards/stm32f4_discovery.repl",
			Tier:     1,
		},
		{
			ID:       "arduino_nano_33_ble",
			Name:     "arduino_nano_33_ble",
			Type:     "board",
			ReplPath: "platforms/boards/arduino_nano_33_ble.repl",
			Tier:     1,
		},
		{
			ID:       "stm32f4",
			Name:     "stm32f4",
			Type:     "cpu",
			ReplPath: "platforms/cpus/stm32f4.repl",
			Tier:     2,
		},
	}

	if err := store.SeedHardware(seed); err != nil {
		t.Fatalf("SeedHardware failed: %v", err)
	}

	items, err = store.ListHardware()
	if err != nil {
		t.Fatalf("ListHardware post-seed failed: %v", err)
	}
	if len(items) != 3 {
		t.Fatalf("expected 3 items, got %d", len(items))
	}

	// Verify sorting: Tier 1 Boards -> Tier 2 CPUs
	if items[0].ID != "arduino_nano_33_ble" {
		t.Errorf("expected arduino_nano_33_ble first (Sort by Tier, Type, Name), got %s", items[0].ID)
	}
	if items[1].ID != "stm32f4_discovery" {
		t.Errorf("expected stm32f4_discovery second, got %s", items[1].ID)
	}
	if items[2].ID != "stm32f4" {
		t.Errorf("expected stm32f4 third (Tier 2), got %s", items[2].ID)
	}

	// Test overwriting
	newSeed := []HardwareItem{
		{
			ID:       "nrf52840",
			Name:     "nrf52840",
			Type:     "cpu",
			ReplPath: "platforms/cpus/nrf52840.repl",
			Tier:     1,
		},
	}

	if err := store.SeedHardware(newSeed); err != nil {
		t.Fatalf("SeedHardware full replace failed: %v", err)
	}

	items, err = store.ListHardware()
	if err != nil {
		t.Fatalf("ListHardware post-replace failed: %v", err)
	}
	if len(items) != 1 {
		t.Fatalf("expected 1 item after replace, got %d", len(items))
	}
	if items[0].ID != "nrf52840" {
		t.Errorf("expected nrf52840, got %s", items[0].ID)
	}
}

func TestPruneTerminalRunsBefore_DeletesOnlyEligibleRows(t *testing.T) {
	store := newTestStore(t)

	mustRun := func(runID, workspaceID, status, artifactsPath, createdAt string) {
		t.Helper()
		_, err := store.db.Exec(
			`INSERT INTO simulation_runs (run_id, workspace_id, status, assertions_passed, assertions_total, artifacts_path, created_at)
			 VALUES (?, ?, ?, 0, 0, ?, ?)`,
			runID, workspaceID, status, artifactsPath, createdAt,
		)
		if err != nil {
			t.Fatalf("insert run %s failed: %v", runID, err)
		}
	}

	old := "2000-01-01 00:00:00"
	recent := time.Now().UTC().Format("2006-01-02 15:04:05")

	mustRun("old-terminal-empty", "ws", "pass", "", old)
	mustRun("old-terminal-with-artifacts", "ws", "fail", "/tmp/artifacts", old)
	mustRun("old-non-terminal", "ws", "running", "", old)
	mustRun("recent-terminal-empty", "ws", "error", "", recent)

	deleted, err := store.PruneTerminalRunsBefore(time.Date(2020, 1, 1, 0, 0, 0, 0, time.UTC))
	if err != nil {
		t.Fatalf("PruneTerminalRunsBefore failed: %v", err)
	}
	if deleted != 1 {
		t.Fatalf("expected 1 deleted row, got %d", deleted)
	}

	var remaining int
	if err := store.db.QueryRow(`SELECT COUNT(*) FROM simulation_runs`).Scan(&remaining); err != nil {
		t.Fatalf("count simulation_runs failed: %v", err)
	}
	if remaining != 3 {
		t.Fatalf("expected 3 remaining rows, got %d", remaining)
	}
}

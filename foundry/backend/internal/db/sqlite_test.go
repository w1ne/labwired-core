package db

import (
	"path/filepath"
	"testing"
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

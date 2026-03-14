package db

import (
	"database/sql"
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

func TestPruneIdempotencyRequestsBefore_DeletesOldRows(t *testing.T) {
	store := newTestStore(t)

	_, err := store.db.Exec(
		`INSERT INTO idempotency_requests (workspace_id, endpoint, idempotency_key, status_code, response_body, created_at)
		 VALUES
		   ('ws', '/v1/models/verify', 'old-key', 202, '{"run_id":"old"}', '2000-01-01 00:00:00'),
		   ('ws', '/v1/models/verify', 'new-key', 202, '{"run_id":"new"}', ?)`,
		time.Now().UTC().Format("2006-01-02 15:04:05"),
	)
	if err != nil {
		t.Fatalf("insert idempotency rows failed: %v", err)
	}

	deleted, err := store.PruneIdempotencyRequestsBefore(time.Date(2020, 1, 1, 0, 0, 0, 0, time.UTC))
	if err != nil {
		t.Fatalf("PruneIdempotencyRequestsBefore failed: %v", err)
	}
	if deleted != 1 {
		t.Fatalf("expected 1 deleted row, got %d", deleted)
	}

	var remaining int
	if err := store.db.QueryRow(`SELECT COUNT(*) FROM idempotency_requests`).Scan(&remaining); err != nil {
		t.Fatalf("count idempotency_requests failed: %v", err)
	}
	if remaining != 1 {
		t.Fatalf("expected 1 remaining row, got %d", remaining)
	}
}

func TestIncrementRateWindow_IncrementsWithinWindowAndResets(t *testing.T) {
	store := newTestStore(t)
	now := time.Date(2026, 3, 5, 10, 0, 10, 0, time.UTC)

	count1, reset1, err := store.IncrementRateWindow("api_key", "key-1", now, time.Minute)
	if err != nil {
		t.Fatalf("IncrementRateWindow first failed: %v", err)
	}
	if count1 != 1 {
		t.Fatalf("expected first count 1, got %d", count1)
	}
	if want := time.Date(2026, 3, 5, 10, 1, 0, 0, time.UTC); !reset1.Equal(want) {
		t.Fatalf("unexpected reset time: got %s want %s", reset1, want)
	}

	count2, _, err := store.IncrementRateWindow("api_key", "key-1", now.Add(20*time.Second), time.Minute)
	if err != nil {
		t.Fatalf("IncrementRateWindow second failed: %v", err)
	}
	if count2 != 2 {
		t.Fatalf("expected second count 2, got %d", count2)
	}

	count3, reset3, err := store.IncrementRateWindow("api_key", "key-1", now.Add(70*time.Second), time.Minute)
	if err != nil {
		t.Fatalf("IncrementRateWindow third failed: %v", err)
	}
	if count3 != 1 {
		t.Fatalf("expected reset window count 1, got %d", count3)
	}
	if want := time.Date(2026, 3, 5, 10, 2, 0, 0, time.UTC); !reset3.Equal(want) {
		t.Fatalf("unexpected reset time after roll: got %s want %s", reset3, want)
	}
}

func TestIncrementRateWindow_PrunesOldWindows(t *testing.T) {
	store := newTestStore(t)
	base := time.Date(2026, 3, 5, 10, 0, 0, 0, time.UTC)

	if _, _, err := store.IncrementRateWindow("workspace", "ws-1", base, time.Minute); err != nil {
		t.Fatalf("IncrementRateWindow base failed: %v", err)
	}
	if _, _, err := store.IncrementRateWindow("workspace", "ws-1", base.Add(3*time.Minute), time.Minute); err != nil {
		t.Fatalf("IncrementRateWindow future failed: %v", err)
	}

	var oldCount int
	if err := store.db.QueryRow(
		`SELECT COUNT(*) FROM request_rate_windows
		 WHERE scope = 'workspace' AND subject = 'ws-1' AND window_start = '2026-03-05 10:00:00'`,
	).Scan(&oldCount); err != nil {
		t.Fatalf("old window count query failed: %v", err)
	}
	if oldCount != 0 {
		t.Fatalf("expected old window to be pruned, rows=%d", oldCount)
	}
}

func TestListRecoverableRuns_ReturnsQueuedAndRunningOnly(t *testing.T) {
	store := newTestStore(t)

	insert := func(runID, workspaceID, status string) {
		t.Helper()
		if err := store.SaveRun(runID, workspaceID, status); err != nil {
			t.Fatalf("SaveRun(%s) failed: %v", runID, err)
		}
	}
	insert("run-q-1", "ws", "queued")
	insert("run-r-1", "ws", "running")
	insert("run-p-1", "ws", "pass")

	recoverable, err := store.ListRecoverableRuns()
	if err != nil {
		t.Fatalf("ListRecoverableRuns failed: %v", err)
	}
	if len(recoverable) != 2 {
		t.Fatalf("expected 2 recoverable runs, got %d", len(recoverable))
	}
	statusByID := map[string]string{}
	for _, r := range recoverable {
		statusByID[r.RunID] = r.Status
	}
	if statusByID["run-q-1"] != "queued" {
		t.Fatalf("missing queued run in recoverable set")
	}
	if statusByID["run-r-1"] != "running" {
		t.Fatalf("missing running run in recoverable set")
	}
	if _, ok := statusByID["run-p-1"]; ok {
		t.Fatalf("pass run should not be recoverable")
	}
}

func TestListQueuedRuns_ReturnsOnlyQueuedWithLimit(t *testing.T) {
	store := newTestStore(t)
	insert := func(runID, status string) {
		t.Helper()
		if err := store.SaveRun(runID, "ws-q", status); err != nil {
			t.Fatalf("SaveRun(%s) failed: %v", runID, err)
		}
	}

	insert("q1", "queued")
	insert("q2", "queued")
	insert("r1", "running")

	queued, err := store.ListQueuedRuns(1)
	if err != nil {
		t.Fatalf("ListQueuedRuns failed: %v", err)
	}
	if len(queued) != 1 {
		t.Fatalf("expected one queued row due to limit, got %d", len(queued))
	}
	if queued[0].Status != "queued" {
		t.Fatalf("expected queued status, got %s", queued[0].Status)
	}

	queued, err = store.ListQueuedRuns(10)
	if err != nil {
		t.Fatalf("ListQueuedRuns failed: %v", err)
	}
	if len(queued) != 2 {
		t.Fatalf("expected two queued rows, got %d", len(queued))
	}
}

func TestCountRunsForWorkspace_ExcludesNonBillableRuns(t *testing.T) {
	store := newTestStore(t)
	workspaceID := "ws-billable-count"

	if _, err := store.CreateKey(workspaceID, "lw_sk_live_billable_count"); err != nil {
		t.Fatalf("CreateKey failed: %v", err)
	}
	if err := store.SaveRun("run-billable-1", workspaceID, "queued"); err != nil {
		t.Fatalf("SaveRun run-billable-1 failed: %v", err)
	}
	if err := store.SaveRun("run-billable-2", workspaceID, "queued"); err != nil {
		t.Fatalf("SaveRun run-billable-2 failed: %v", err)
	}
	if err := store.SetRunBillable("run-billable-2", false); err != nil {
		t.Fatalf("SetRunBillable failed: %v", err)
	}

	used, err := store.CountRunsForWorkspace(workspaceID)
	if err != nil {
		t.Fatalf("CountRunsForWorkspace failed: %v", err)
	}
	if used != 1 {
		t.Fatalf("expected used=1 (excluding non-billable), got %d", used)
	}
}

func TestReserveRunForWorkspace_AllowsAfterNonBillableReservation(t *testing.T) {
	store := newTestStore(t)
	workspaceID := "ws-billable-reserve"

	if _, err := store.CreateKey(workspaceID, "lw_sk_live_billable_reserve"); err != nil {
		t.Fatalf("CreateKey failed: %v", err)
	}
	if _, err := store.db.Exec(`UPDATE api_keys SET monthly_quota = 1 WHERE workspace_id = ?`, workspaceID); err != nil {
		t.Fatalf("failed to set quota: %v", err)
	}

	if err := store.ReserveRunForWorkspace("run-reserve-1", workspaceID, "queued"); err != nil {
		t.Fatalf("first ReserveRunForWorkspace failed: %v", err)
	}
	if err := store.SetRunBillable("run-reserve-1", false); err != nil {
		t.Fatalf("SetRunBillable failed: %v", err)
	}
	if err := store.ReserveRunForWorkspace("run-reserve-2", workspaceID, "queued"); err != nil {
		t.Fatalf("second ReserveRunForWorkspace should succeed after non-billable mark: %v", err)
	}
}

func TestReserveRunForWorkspaceWithInflight_EnforcesActiveLimit(t *testing.T) {
	store := newTestStore(t)
	workspaceID := "ws-inflight-enforce"

	if _, err := store.CreateKey(workspaceID, "lw_sk_live_inflight_enforce"); err != nil {
		t.Fatalf("CreateKey failed: %v", err)
	}
	if _, err := store.db.Exec(`UPDATE api_keys SET monthly_quota = 10 WHERE workspace_id = ?`, workspaceID); err != nil {
		t.Fatalf("failed to set quota: %v", err)
	}

	if err := store.ReserveRunForWorkspaceWithInflight("run-active-1", workspaceID, "queued", 1); err != nil {
		t.Fatalf("first reserve failed: %v", err)
	}
	err := store.ReserveRunForWorkspaceWithInflight("run-active-2", workspaceID, "queued", 1)
	if err != ErrInflightLimit {
		t.Fatalf("expected ErrInflightLimit, got %v", err)
	}

	if err := store.UpdateRunStatus("run-active-1", "pass", 1, 1, ""); err != nil {
		t.Fatalf("UpdateRunStatus failed: %v", err)
	}
	if err := store.ReserveRunForWorkspaceWithInflight("run-active-2", workspaceID, "queued", 1); err != nil {
		t.Fatalf("reserve after terminal status failed: %v", err)
	}
}

func TestTryClaimQueuedRun_AllowsSingleWinner(t *testing.T) {
	store := newTestStore(t)
	if err := store.SaveRun("run-claim-1", "ws-claim", "queued"); err != nil {
		t.Fatalf("SaveRun failed: %v", err)
	}

	ok, err := store.TryClaimQueuedRun("run-claim-1", "worker-a", time.Now(), 3)
	if err != nil {
		t.Fatalf("TryClaimQueuedRun first failed: %v", err)
	}
	if !ok {
		t.Fatalf("expected first claim to win")
	}

	ok, err = store.TryClaimQueuedRun("run-claim-1", "worker-b", time.Now(), 3)
	if err != nil {
		t.Fatalf("TryClaimQueuedRun second failed: %v", err)
	}
	if ok {
		t.Fatalf("expected second claim to lose")
	}
}

func TestCompleteClaimedRun_ClearsClaimMetadata(t *testing.T) {
	store := newTestStore(t)
	if err := store.SaveRun("run-complete-1", "ws-complete", "queued"); err != nil {
		t.Fatalf("SaveRun failed: %v", err)
	}
	if ok, err := store.TryClaimQueuedRun("run-complete-1", "worker-a", time.Now(), 3); err != nil || !ok {
		t.Fatalf("TryClaimQueuedRun failed: ok=%v err=%v", ok, err)
	}

	if err := store.CompleteClaimedRun("run-complete-1", "pass", 2, 2, "/tmp/x", ""); err != nil {
		t.Fatalf("CompleteClaimedRun failed: %v", err)
	}

	row := store.db.QueryRow(`SELECT status, worker_id, claimed_at, last_error, assertions_passed, assertions_total FROM simulation_runs WHERE run_id = ?`, "run-complete-1")
	var (
		status, workerID, lastError string
		claimedAt                   sql.NullString
		passed, total               int
	)
	if err := row.Scan(&status, &workerID, &claimedAt, &lastError, &passed, &total); err != nil {
		t.Fatalf("scan failed: %v", err)
	}
	if status != "pass" {
		t.Fatalf("expected status pass, got %s", status)
	}
	if workerID != "" {
		t.Fatalf("expected worker_id cleared, got %q", workerID)
	}
	if claimedAt.Valid {
		t.Fatalf("expected claimed_at cleared, got %q", claimedAt.String)
	}
	if lastError != "" {
		t.Fatalf("expected empty last_error, got %q", lastError)
	}
	if passed != 2 || total != 2 {
		t.Fatalf("unexpected assertions counters: %d/%d", passed, total)
	}
}

func TestRequeueRunningRuns_MovesRunningToQueued(t *testing.T) {
	store := newTestStore(t)
	if err := store.SaveRun("run-requeue-1", "ws-requeue", "queued"); err != nil {
		t.Fatalf("SaveRun failed: %v", err)
	}
	if ok, err := store.TryClaimQueuedRun("run-requeue-1", "worker-a", time.Now(), 3); err != nil || !ok {
		t.Fatalf("TryClaimQueuedRun failed: ok=%v err=%v", ok, err)
	}

	n, err := store.RequeueRunningRuns("test-requeue")
	if err != nil {
		t.Fatalf("RequeueRunningRuns failed: %v", err)
	}
	if n != 1 {
		t.Fatalf("expected one row requeued, got %d", n)
	}

	row := store.db.QueryRow(`SELECT status, worker_id, claimed_at, last_error FROM simulation_runs WHERE run_id = ?`, "run-requeue-1")
	var (
		status, workerID, lastError string
		claimedAt                   sql.NullString
	)
	if err := row.Scan(&status, &workerID, &claimedAt, &lastError); err != nil {
		t.Fatalf("scan failed: %v", err)
	}
	if status != "queued" {
		t.Fatalf("expected status queued, got %s", status)
	}
	if workerID != "" {
		t.Fatalf("expected worker_id cleared, got %q", workerID)
	}
	if claimedAt.Valid {
		t.Fatalf("expected claimed_at cleared, got %q", claimedAt.String)
	}
	if lastError != "test-requeue" {
		t.Fatalf("unexpected last_error: %q", lastError)
	}
}

func TestHeartbeatClaimedRun_OnlyOwnerCanRefresh(t *testing.T) {
	store := newTestStore(t)
	if err := store.SaveRun("run-hb-1", "ws-hb", "queued"); err != nil {
		t.Fatalf("SaveRun failed: %v", err)
	}
	if ok, err := store.TryClaimQueuedRun("run-hb-1", "worker-a", time.Now(), 3); err != nil || !ok {
		t.Fatalf("TryClaimQueuedRun failed: ok=%v err=%v", ok, err)
	}

	ok, err := store.HeartbeatClaimedRun("run-hb-1", "worker-a", time.Now())
	if err != nil {
		t.Fatalf("HeartbeatClaimedRun owner failed: %v", err)
	}
	if !ok {
		t.Fatalf("expected owner heartbeat to succeed")
	}

	ok, err = store.HeartbeatClaimedRun("run-hb-1", "worker-b", time.Now())
	if err != nil {
		t.Fatalf("HeartbeatClaimedRun non-owner failed: %v", err)
	}
	if ok {
		t.Fatalf("expected non-owner heartbeat to fail")
	}
}

func TestRequeueStaleRunningRuns_RequeuesOnlyExpired(t *testing.T) {
	store := newTestStore(t)
	now := time.Now().UTC()

	if err := store.SaveRun("run-stale", "ws-stale", "queued"); err != nil {
		t.Fatalf("SaveRun stale failed: %v", err)
	}
	if err := store.SaveRun("run-fresh", "ws-fresh", "queued"); err != nil {
		t.Fatalf("SaveRun fresh failed: %v", err)
	}
	if ok, err := store.TryClaimQueuedRun("run-stale", "worker-a", now.Add(-2*time.Minute), 3); err != nil || !ok {
		t.Fatalf("TryClaimQueuedRun stale failed: ok=%v err=%v", ok, err)
	}
	if ok, err := store.TryClaimQueuedRun("run-fresh", "worker-b", now, 3); err != nil || !ok {
		t.Fatalf("TryClaimQueuedRun fresh failed: ok=%v err=%v", ok, err)
	}
	// Force an old lease timestamp for stale row.
	if _, err := store.db.Exec(
		`UPDATE simulation_runs SET claimed_at = ?, worker_id = 'worker-a' WHERE run_id = ?`,
		now.Add(-2*time.Minute).Format("2006-01-02 15:04:05"),
		"run-stale",
	); err != nil {
		t.Fatalf("set stale claimed_at failed: %v", err)
	}

	n, err := store.RequeueStaleRunningRuns(now.Add(-30*time.Second), "lease-expired")
	if err != nil {
		t.Fatalf("RequeueStaleRunningRuns failed: %v", err)
	}
	if n != 1 {
		t.Fatalf("expected one stale row requeued, got %d", n)
	}

	stale, err := store.GetRun("run-stale")
	if err != nil || stale == nil {
		t.Fatalf("GetRun stale failed: err=%v", err)
	}
	if stale.Status != "queued" {
		t.Fatalf("expected stale run queued, got %s", stale.Status)
	}

	fresh, err := store.GetRun("run-fresh")
	if err != nil || fresh == nil {
		t.Fatalf("GetRun fresh failed: err=%v", err)
	}
	if fresh.Status != "running" {
		t.Fatalf("expected fresh run to stay running, got %s", fresh.Status)
	}
}

func TestIdempotencyRequestLifecycle(t *testing.T) {
	store := newTestStore(t)
	workspaceID := "ws-idem"
	endpoint := "/v1/models/verify"
	key := "idem-123"

	isNew, existing, err := store.BeginIdempotencyRequest(workspaceID, endpoint, key)
	if err != nil {
		t.Fatalf("BeginIdempotencyRequest first failed: %v", err)
	}
	if !isNew {
		t.Fatalf("expected first idempotency begin to be new")
	}
	if existing == nil {
		t.Fatalf("expected existing record payload")
	}
	if existing.StatusCode != 0 {
		t.Fatalf("expected pending status code 0, got %d", existing.StatusCode)
	}

	isNew, existing, err = store.BeginIdempotencyRequest(workspaceID, endpoint, key)
	if err != nil {
		t.Fatalf("BeginIdempotencyRequest second failed: %v", err)
	}
	if isNew {
		t.Fatalf("expected duplicate idempotency begin to not be new")
	}
	if existing.StatusCode != 0 {
		t.Fatalf("expected duplicate existing pending status code 0, got %d", existing.StatusCode)
	}

	if err := store.CompleteIdempotencyRequest(workspaceID, endpoint, key, "run-1", 202, `{"run_id":"run-1"}`); err != nil {
		t.Fatalf("CompleteIdempotencyRequest failed: %v", err)
	}

	isNew, existing, err = store.BeginIdempotencyRequest(workspaceID, endpoint, key)
	if err != nil {
		t.Fatalf("BeginIdempotencyRequest third failed: %v", err)
	}
	if isNew {
		t.Fatalf("expected completed idempotency key to replay existing response")
	}
	if existing.StatusCode != 202 || existing.RunID != "run-1" {
		t.Fatalf("unexpected replay record: status=%d run_id=%s", existing.StatusCode, existing.RunID)
	}
}

func TestCancelPendingIdempotencyRequest_AllowsRetry(t *testing.T) {
	store := newTestStore(t)
	workspaceID := "ws-idem-cancel"
	endpoint := "/v1/synthesize"
	key := "idem-cancel-1"

	isNew, _, err := store.BeginIdempotencyRequest(workspaceID, endpoint, key)
	if err != nil {
		t.Fatalf("BeginIdempotencyRequest failed: %v", err)
	}
	if !isNew {
		t.Fatalf("expected first idempotency request to be new")
	}

	if err := store.CancelPendingIdempotencyRequest(workspaceID, endpoint, key); err != nil {
		t.Fatalf("CancelPendingIdempotencyRequest failed: %v", err)
	}

	isNew, _, err = store.BeginIdempotencyRequest(workspaceID, endpoint, key)
	if err != nil {
		t.Fatalf("BeginIdempotencyRequest retry failed: %v", err)
	}
	if !isNew {
		t.Fatalf("expected cancelled idempotency request to allow retry")
	}
}

func TestTryClaimQueuedRun_RespectsMaxAttempts(t *testing.T) {
	store := newTestStore(t)
	if err := store.SaveRun("run-attempt-cap", "ws-attempt", "queued"); err != nil {
		t.Fatalf("SaveRun failed: %v", err)
	}
	if _, err := store.db.Exec(`UPDATE simulation_runs SET attempt_count = 3 WHERE run_id = ?`, "run-attempt-cap"); err != nil {
		t.Fatalf("failed to set attempt_count: %v", err)
	}

	ok, err := store.TryClaimQueuedRun("run-attempt-cap", "worker-a", time.Now(), 3)
	if err != nil {
		t.Fatalf("TryClaimQueuedRun failed: %v", err)
	}
	if ok {
		t.Fatalf("expected claim to fail when attempt_count reached max")
	}
}

func TestFailExhaustedQueuedRuns_MarksQueuedAsError(t *testing.T) {
	store := newTestStore(t)
	if err := store.SaveRun("run-exhausted", "ws-attempt", "queued"); err != nil {
		t.Fatalf("SaveRun failed: %v", err)
	}
	if _, err := store.db.Exec(`UPDATE simulation_runs SET attempt_count = 4 WHERE run_id = ?`, "run-exhausted"); err != nil {
		t.Fatalf("failed to set attempt_count: %v", err)
	}

	n, err := store.FailExhaustedQueuedRuns(3, "max attempts exhausted")
	if err != nil {
		t.Fatalf("FailExhaustedQueuedRuns failed: %v", err)
	}
	if n != 1 {
		t.Fatalf("expected one row marked exhausted, got %d", n)
	}

	record, err := store.GetRun("run-exhausted")
	if err != nil || record == nil {
		t.Fatalf("GetRun failed: err=%v", err)
	}
	if record.Status != "error" {
		t.Fatalf("expected status error, got %s", record.Status)
	}
}

func TestGetAccountUsageAndRuns_DeduplicateSharedWorkspaceKeys(t *testing.T) {
	store := newTestStore(t)
	clerkUserID := "clerk-shared-workspace"

	key1, err := store.CreateKeyForClerkUser(clerkUserID, "lw_sk_live_shared_workspace_key_1", "ws-shared")
	if err != nil {
		t.Fatalf("CreateKeyForClerkUser first failed: %v", err)
	}
	key2, err := store.CreateKeyForClerkUser(clerkUserID, "lw_sk_live_shared_workspace_key_2", "ws-ignored")
	if err != nil {
		t.Fatalf("CreateKeyForClerkUser second failed: %v", err)
	}
	if key2.WorkspaceID != key1.WorkspaceID {
		t.Fatalf("expected reused workspace, got=%s want=%s", key2.WorkspaceID, key1.WorkspaceID)
	}

	if err := store.SaveRun("run-shared-1", key1.WorkspaceID, "pass"); err != nil {
		t.Fatalf("SaveRun failed: %v", err)
	}

	usage, err := store.GetAccountUsage(clerkUserID)
	if err != nil {
		t.Fatalf("GetAccountUsage failed: %v", err)
	}
	if usage.Quota != 1000 {
		t.Fatalf("expected quota 1000 once per workspace, got %d", usage.Quota)
	}
	if usage.RunsUsedThisMonth != 1 {
		t.Fatalf("expected one used run, got %d", usage.RunsUsedThisMonth)
	}

	runs, err := store.ListRunsForClerkUser(clerkUserID)
	if err != nil {
		t.Fatalf("ListRunsForClerkUser failed: %v", err)
	}
	if len(runs) != 1 {
		t.Fatalf("expected one deduplicated run, got %d", len(runs))
	}
	if runs[0].RunID != "run-shared-1" {
		t.Fatalf("unexpected run id: got=%s want=run-shared-1", runs[0].RunID)
	}
}

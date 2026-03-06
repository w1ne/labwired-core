package db

import (
	"database/sql"
	"errors"
	"fmt"
	"time"

	_ "modernc.org/sqlite"
)

// RunRecord is the full simulation_runs row returned for polling.
type RunRecord struct {
	RunID            string `json:"run_id"`
	WorkspaceID      string `json:"workspace_id"`
	Status           string `json:"status"`
	AssertionsPassed int    `json:"assertions_passed"`
	AssertionsTotal  int    `json:"assertions_total"`
	ArtifactsPath    string `json:"artifacts_path,omitempty"`
	CreatedAt        string `json:"created_at"`
}

// RunArtifactRecord represents a run with persisted artifacts and timestamp.
type RunArtifactRecord struct {
	RunID         string
	WorkspaceID   string
	ArtifactsPath string
	CreatedAt     string
}

// IdempotencyRecord stores replay-safe response data for idempotent requests.
type IdempotencyRecord struct {
	WorkspaceID  string
	Endpoint     string
	Key          string
	RunID        string
	StatusCode   int
	ResponseBody string
	CreatedAt    string
}

// HardwareItem represents a Renode-supported board or CPU in the database.
type HardwareItem struct {
	ID       string `json:"id"`
	Name     string `json:"name"`
	Type     string `json:"type"` // "board" or "cpu"
	ReplPath string `json:"repl_path"`
	Tier     int    `json:"tier"` // 1 (Top 20%) or 2 (Extended)
}

// CatalogAsset represents a verified hardware model in the catalog.
type CatalogAsset struct {
	ID          string `json:"id"`
	Name        string `json:"name"`
	Description string `json:"description"`
	PassRate    int    `json:"pass_rate"`
	Registers   int    `json:"registers"`
	IrURL       string `json:"ir_url"`
	Verified    bool   `json:"verified"`
	SourceType  string `json:"source_type"`
	SourceRef   string `json:"source_ref"`
}

type Store struct {
	db *sql.DB
}

var ErrQuotaExceeded = errors.New("quota exceeded")
var ErrInflightLimit = errors.New("inflight limit exceeded")

func NewStore(path string) (*Store, error) {
	db, err := sql.Open("sqlite", path)
	if err != nil {
		return nil, err
	}

	s := &Store{db: db}
	if err := s.migrate(); err != nil {
		return nil, err
	}

	return s, nil
}

// Ping checks the database connectivity.
func (s *Store) Ping() error {
	return s.db.Ping()
}

// Close releases database resources.
func (s *Store) Close() error {
	return s.db.Close()
}

func (s *Store) migrate() error {
	queries := []string{
		`PRAGMA journal_mode=WAL;`,
		`PRAGMA busy_timeout = 5000;`,
		`CREATE TABLE IF NOT EXISTS api_keys (
			id UUID PRIMARY KEY,
			key_hash TEXT NOT NULL,
			key_prefix TEXT NOT NULL DEFAULT '',
			workspace_id UUID NOT NULL,
			tier TEXT NOT NULL DEFAULT 'builder',
			monthly_quota INTEGER NOT NULL DEFAULT 1000,
			revoked BOOLEAN DEFAULT FALSE
		);`,
		`CREATE TABLE IF NOT EXISTS simulation_runs (
			run_id UUID PRIMARY KEY,
			workspace_id UUID NOT NULL,
			status TEXT NOT NULL,
			assertions_passed INTEGER DEFAULT 0,
			assertions_total INTEGER DEFAULT 0,
			billable INTEGER NOT NULL DEFAULT 1,
			attempt_count INTEGER NOT NULL DEFAULT 0,
			last_error TEXT NOT NULL DEFAULT '',
			claimed_at TIMESTAMP NULL,
			worker_id TEXT NOT NULL DEFAULT '',
			artifacts_path TEXT DEFAULT '',
			created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
		);`,
		`CREATE TABLE IF NOT EXISTS stripe_events (
			event_id TEXT PRIMARY KEY,
			session_id TEXT NOT NULL,
			workspace_id TEXT NOT NULL,
			runs_credited INTEGER NOT NULL,
			created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
		);`,
		`CREATE TABLE IF NOT EXISTS request_rate_windows (
			scope TEXT NOT NULL,
			subject TEXT NOT NULL,
			window_start TIMESTAMP NOT NULL,
			request_count INTEGER NOT NULL DEFAULT 0,
			PRIMARY KEY (scope, subject, window_start)
		);`,
		`CREATE TABLE IF NOT EXISTS idempotency_requests (
			workspace_id TEXT NOT NULL,
			endpoint TEXT NOT NULL,
			idempotency_key TEXT NOT NULL,
			run_id TEXT NOT NULL DEFAULT '',
			status_code INTEGER NOT NULL DEFAULT 0,
			response_body TEXT NOT NULL DEFAULT '',
			created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
			PRIMARY KEY (workspace_id, endpoint, idempotency_key)
		);`,
		`CREATE TABLE IF NOT EXISTS supported_hardware (
			id TEXT PRIMARY KEY,
			name TEXT NOT NULL,
			type TEXT NOT NULL,
			repl_path TEXT NOT NULL,
			tier INTEGER NOT NULL DEFAULT 2
		);`,
		`CREATE TABLE IF NOT EXISTS catalog_assets (
			id TEXT PRIMARY KEY,
			name TEXT NOT NULL,
			description TEXT NOT NULL,
			pass_rate INTEGER DEFAULT 0,
			registers INTEGER DEFAULT 0,
			ir_url TEXT DEFAULT '',
			verified INTEGER NOT NULL DEFAULT 0,
			source_type TEXT NOT NULL DEFAULT 'unknown',
			source_ref TEXT NOT NULL DEFAULT ''
		);`,
		`ALTER TABLE catalog_assets ADD COLUMN verified INTEGER NOT NULL DEFAULT 0;`,
		`ALTER TABLE catalog_assets ADD COLUMN source_type TEXT NOT NULL DEFAULT 'unknown';`,
		`ALTER TABLE catalog_assets ADD COLUMN source_ref TEXT NOT NULL DEFAULT '';`,
		// Non-destructive: add monthly_quota column if it was missing in an older DB.
		`ALTER TABLE api_keys ADD COLUMN monthly_quota INTEGER NOT NULL DEFAULT 1000;`,
		// Non-destructive: add key_prefix column if missing.
		`ALTER TABLE api_keys ADD COLUMN key_prefix TEXT NOT NULL DEFAULT '';`,
		// Non-destructive: add artifacts_path column if missing.
		`ALTER TABLE simulation_runs ADD COLUMN artifacts_path TEXT DEFAULT '';`,
		// Non-destructive: add billable column if missing.
		`ALTER TABLE simulation_runs ADD COLUMN billable INTEGER NOT NULL DEFAULT 1;`,
		// Non-destructive: add worker lifecycle metadata columns if missing.
		`ALTER TABLE simulation_runs ADD COLUMN attempt_count INTEGER NOT NULL DEFAULT 0;`,
		`ALTER TABLE simulation_runs ADD COLUMN last_error TEXT NOT NULL DEFAULT '';`,
		`ALTER TABLE simulation_runs ADD COLUMN claimed_at TIMESTAMP NULL;`,
		`ALTER TABLE simulation_runs ADD COLUMN worker_id TEXT NOT NULL DEFAULT '';`,
		// Non-destructive: add clerk_user_id column if missing.
		`ALTER TABLE api_keys ADD COLUMN clerk_user_id TEXT NOT NULL DEFAULT '';`,
		`CREATE INDEX IF NOT EXISTS idx_api_keys_prefix ON api_keys(key_prefix);`,
		`CREATE INDEX IF NOT EXISTS idx_api_keys_clerk_user ON api_keys(clerk_user_id);`,
		`CREATE INDEX IF NOT EXISTS idx_simulation_runs_workspace_created ON simulation_runs(workspace_id, created_at);`,
		`CREATE INDEX IF NOT EXISTS idx_idempotency_created ON idempotency_requests(created_at);`,
		`CREATE INDEX IF NOT EXISTS idx_request_rate_windows_window_start ON request_rate_windows(window_start);`,
	}

	for _, q := range queries {
		if _, err := s.db.Exec(q); err != nil {
			// Ignore "column already exists" errors from the ALTER TABLE migrations.
			if isAlreadyExistsErr(err) {
				continue
			}
			return fmt.Errorf("migration failed: %w", err)
		}
	}

	return nil
}

// IncrementRateWindow increments request count for a scope+subject inside the current window.
// It returns the updated count and window reset timestamp.
func (s *Store) IncrementRateWindow(scope, subject string, now time.Time, window time.Duration) (int, time.Time, error) {
	if window <= 0 {
		return 0, time.Time{}, fmt.Errorf("window must be > 0")
	}
	windowStart := now.UTC().Truncate(window)
	windowStartStr := windowStart.Format("2006-01-02 15:04:05")
	resetAt := windowStart.Add(window)

	tx, err := s.db.Begin()
	if err != nil {
		return 0, time.Time{}, err
	}
	defer func() {
		_ = tx.Rollback()
	}()

	if _, err := tx.Exec(
		`INSERT OR IGNORE INTO request_rate_windows (scope, subject, window_start, request_count)
		 VALUES (?, ?, ?, 0)`,
		scope, subject, windowStartStr,
	); err != nil {
		return 0, time.Time{}, err
	}

	if _, err := tx.Exec(
		`UPDATE request_rate_windows
		 SET request_count = request_count + 1
		 WHERE scope = ? AND subject = ? AND window_start = ?`,
		scope, subject, windowStartStr,
	); err != nil {
		return 0, time.Time{}, err
	}

	var count int
	if err := tx.QueryRow(
		`SELECT request_count FROM request_rate_windows
		 WHERE scope = ? AND subject = ? AND window_start = ?`,
		scope, subject, windowStartStr,
	).Scan(&count); err != nil {
		return 0, time.Time{}, err
	}

	cutoff := windowStart.Add(-2 * window).Format("2006-01-02 15:04:05")
	if _, err := tx.Exec(
		`DELETE FROM request_rate_windows WHERE datetime(window_start) < datetime(?)`,
		cutoff,
	); err != nil {
		return 0, time.Time{}, err
	}

	if err := tx.Commit(); err != nil {
		return 0, time.Time{}, err
	}
	return count, resetAt, nil
}

// isAlreadyExistsErr returns true for SQLite "duplicate column" errors produced
// by the idempotent ALTER TABLE statements in migrate().
func isAlreadyExistsErr(err error) bool {
	if err == nil {
		return false
	}
	msg := err.Error()
	return contains(msg, "duplicate column name") || contains(msg, "already exists")
}

func contains(s, sub string) bool {
	return len(s) >= len(sub) && (s == sub || len(s) > 0 && containsStr(s, sub))
}

func containsStr(s, sub string) bool {
	for i := 0; i <= len(s)-len(sub); i++ {
		if s[i:i+len(sub)] == sub {
			return true
		}
	}
	return false
}

// SaveRun inserts a new simulation_runs row with the given initial status.
func (s *Store) SaveRun(runID, workspaceID, status string) error {
	_, err := s.db.Exec(
		"INSERT INTO simulation_runs (run_id, workspace_id, status) VALUES (?, ?, ?)",
		runID, workspaceID, status,
	)
	return err
}

// ReserveRunForWorkspace atomically inserts a run if the workspace still has quota.
func (s *Store) ReserveRunForWorkspace(runID, workspaceID, status string) error {
	res, err := s.db.Exec(
		`INSERT INTO simulation_runs (run_id, workspace_id, status)
		 SELECT ?, ?, ?
		 WHERE (
		   SELECT COALESCE(SUM(billable), 0) FROM simulation_runs
		   WHERE workspace_id = ? AND datetime(created_at) >= datetime('now', '-30 days')
		 ) < COALESCE(
		   (SELECT monthly_quota FROM api_keys WHERE workspace_id = ? AND revoked = FALSE LIMIT 1),
		   1000
		 )`,
		runID, workspaceID, status, workspaceID, workspaceID,
	)
	if err != nil {
		return err
	}
	n, err := res.RowsAffected()
	if err != nil {
		return err
	}
	if n == 0 {
		return ErrQuotaExceeded
	}
	return nil
}

// ReserveRunForWorkspaceWithInflight atomically inserts a run if workspace quota
// and workspace active-run inflight limit are both satisfied.
func (s *Store) ReserveRunForWorkspaceWithInflight(runID, workspaceID, status string, maxInflight int) error {
	if maxInflight <= 0 {
		return fmt.Errorf("maxInflight must be > 0")
	}
	res, err := s.db.Exec(
		`INSERT INTO simulation_runs (run_id, workspace_id, status)
		 SELECT ?, ?, ?
		 WHERE (
		   SELECT COUNT(*) FROM simulation_runs
		   WHERE workspace_id = ? AND status IN ('queued', 'running')
		 ) < ?
		 AND (
		   SELECT COALESCE(SUM(billable), 0) FROM simulation_runs
		   WHERE workspace_id = ? AND datetime(created_at) >= datetime('now', '-30 days')
		 ) < COALESCE(
		   (SELECT monthly_quota FROM api_keys WHERE workspace_id = ? AND revoked = FALSE LIMIT 1),
		   1000
		 )`,
		runID, workspaceID, status,
		workspaceID, maxInflight,
		workspaceID, workspaceID,
	)
	if err != nil {
		return err
	}
	n, err := res.RowsAffected()
	if err != nil {
		return err
	}
	if n == 1 {
		return nil
	}

	var active int
	if err := s.db.QueryRow(
		`SELECT COUNT(*) FROM simulation_runs WHERE workspace_id = ? AND status IN ('queued', 'running')`,
		workspaceID,
	).Scan(&active); err == nil && active >= maxInflight {
		return ErrInflightLimit
	}
	return ErrQuotaExceeded
}

// ReserveRunsForWorkspace atomically inserts multiple runs for a workspace.
// It either reserves all requested runs or none.
func (s *Store) ReserveRunsForWorkspace(runIDs []string, workspaceID, status string) error {
	if len(runIDs) == 0 {
		return nil
	}

	tx, err := s.db.Begin()
	if err != nil {
		return err
	}
	defer func() {
		_ = tx.Rollback()
	}()

	var used int
	if err := tx.QueryRow(
		`SELECT COALESCE(SUM(billable), 0) FROM simulation_runs
		 WHERE workspace_id = ? AND datetime(created_at) >= datetime('now', '-30 days')`,
		workspaceID,
	).Scan(&used); err != nil {
		return err
	}

	var limit int
	if err := tx.QueryRow(
		`SELECT COALESCE(
		   (SELECT monthly_quota FROM api_keys WHERE workspace_id = ? AND revoked = FALSE LIMIT 1),
		   1000
		 )`,
		workspaceID,
	).Scan(&limit); err != nil {
		return err
	}

	if used+len(runIDs) > limit {
		return ErrQuotaExceeded
	}

	stmt, err := tx.Prepare("INSERT INTO simulation_runs (run_id, workspace_id, status) VALUES (?, ?, ?)")
	if err != nil {
		return err
	}
	defer stmt.Close()

	for _, runID := range runIDs {
		if _, err := stmt.Exec(runID, workspaceID, status); err != nil {
			return err
		}
	}

	if err := tx.Commit(); err != nil {
		return err
	}
	return nil
}

// UpdateRunStatus updates the terminal status and assertion counts for a run.
func (s *Store) UpdateRunStatus(runID, status string, passed, total int, artifactsPath string) error {
	_, err := s.db.Exec(
		`UPDATE simulation_runs
		 SET status = ?, assertions_passed = ?, assertions_total = ?, artifacts_path = ?,
		     claimed_at = NULL, worker_id = ''
		 WHERE run_id = ?`,
		status, passed, total, artifactsPath, runID,
	)
	return err
}

// TryClaimQueuedRun transitions a run from queued -> running for one worker.
// Returns true only if this caller won the claim.
func (s *Store) TryClaimQueuedRun(runID, workerID string, now time.Time, maxAttempts int) (bool, error) {
	if maxAttempts <= 0 {
		maxAttempts = 1
	}
	res, err := s.db.Exec(
		`UPDATE simulation_runs
		 SET status = 'running',
		     attempt_count = attempt_count + 1,
		     claimed_at = ?,
		     worker_id = ?,
		     last_error = ''
		 WHERE run_id = ? AND status = 'queued' AND attempt_count < ?`,
		now.UTC().Format("2006-01-02 15:04:05"),
		workerID,
		runID,
		maxAttempts,
	)
	if err != nil {
		return false, err
	}
	n, err := res.RowsAffected()
	if err != nil {
		return false, err
	}
	return n == 1, nil
}

// FailExhaustedQueuedRuns marks queued runs with exhausted attempts as terminal errors.
func (s *Store) FailExhaustedQueuedRuns(maxAttempts int, reason string) (int64, error) {
	if maxAttempts <= 0 {
		maxAttempts = 1
	}
	res, err := s.db.Exec(
		`UPDATE simulation_runs
		 SET status = 'error',
		     last_error = ?,
		     claimed_at = NULL,
		     worker_id = ''
		 WHERE status = 'queued' AND attempt_count >= ?`,
		reason,
		maxAttempts,
	)
	if err != nil {
		return 0, err
	}
	n, err := res.RowsAffected()
	if err != nil {
		return 0, err
	}
	return n, nil
}

// CompleteClaimedRun writes terminal outcome and clears claim metadata.
func (s *Store) CompleteClaimedRun(runID, status string, passed, total int, artifactsPath, lastError string) error {
	_, err := s.db.Exec(
		`UPDATE simulation_runs
		 SET status = ?, assertions_passed = ?, assertions_total = ?, artifacts_path = ?,
		     last_error = ?, claimed_at = NULL, worker_id = ''
		 WHERE run_id = ?`,
		status, passed, total, artifactsPath, lastError, runID,
	)
	return err
}

// RequeueRunningRuns sets all running runs back to queued.
// Deprecated: prefer RequeueStaleRunningRuns for lease-based recovery.
func (s *Store) RequeueRunningRuns(reason string) (int64, error) {
	res, err := s.db.Exec(
		`UPDATE simulation_runs
		 SET status = 'queued',
		     last_error = ?,
		     claimed_at = NULL,
		     worker_id = ''
		 WHERE status = 'running'`,
		reason,
	)
	if err != nil {
		return 0, err
	}
	n, err := res.RowsAffected()
	if err != nil {
		return 0, err
	}
	return n, nil
}

// HeartbeatClaimedRun refreshes the lease timestamp for a claimed running job.
func (s *Store) HeartbeatClaimedRun(runID, workerID string, now time.Time) (bool, error) {
	res, err := s.db.Exec(
		`UPDATE simulation_runs
		 SET claimed_at = ?
		 WHERE run_id = ? AND status = 'running' AND worker_id = ?`,
		now.UTC().Format("2006-01-02 15:04:05"),
		runID,
		workerID,
	)
	if err != nil {
		return false, err
	}
	n, err := res.RowsAffected()
	if err != nil {
		return false, err
	}
	return n == 1, nil
}

// RequeueStaleRunningRuns requeues only stale running jobs whose lease expired.
func (s *Store) RequeueStaleRunningRuns(cutoff time.Time, reason string) (int64, error) {
	res, err := s.db.Exec(
		`UPDATE simulation_runs
		 SET status = 'queued',
		     last_error = ?,
		     claimed_at = NULL,
		     worker_id = ''
		 WHERE status = 'running'
		   AND (claimed_at IS NULL OR datetime(claimed_at) < datetime(?))`,
		reason,
		cutoff.UTC().Format("2006-01-02 15:04:05"),
	)
	if err != nil {
		return 0, err
	}
	n, err := res.RowsAffected()
	if err != nil {
		return 0, err
	}
	return n, nil
}

// GetRun returns the full run record for polling.
func (s *Store) GetRun(runID string) (*RunRecord, error) {
	row := s.db.QueryRow(
		`SELECT run_id, workspace_id, status, assertions_passed, assertions_total, artifacts_path, created_at
		 FROM simulation_runs WHERE run_id = ?`, runID,
	)
	var r RunRecord
	if err := row.Scan(&r.RunID, &r.WorkspaceID, &r.Status, &r.AssertionsPassed, &r.AssertionsTotal, &r.ArtifactsPath, &r.CreatedAt); err != nil {
		if err == sql.ErrNoRows {
			return nil, nil
		}
		return nil, err
	}
	return &r, nil
}

// GetRunForWorkspace returns a run only if it belongs to the given workspace.
func (s *Store) GetRunForWorkspace(runID, workspaceID string) (*RunRecord, error) {
	row := s.db.QueryRow(
		`SELECT run_id, workspace_id, status, assertions_passed, assertions_total, artifacts_path, created_at
		 FROM simulation_runs WHERE run_id = ? AND workspace_id = ?`,
		runID,
		workspaceID,
	)
	var r RunRecord
	if err := row.Scan(&r.RunID, &r.WorkspaceID, &r.Status, &r.AssertionsPassed, &r.AssertionsTotal, &r.ArtifactsPath, &r.CreatedAt); err != nil {
		if err == sql.ErrNoRows {
			return nil, nil
		}
		return nil, err
	}
	return &r, nil
}

// ListRunsWithArtifactsBefore returns runs with artifacts older than the cutoff.
func (s *Store) ListRunsWithArtifactsBefore(cutoff time.Time) ([]RunArtifactRecord, error) {
	rows, err := s.db.Query(
		`SELECT run_id, workspace_id, artifacts_path, created_at
		 FROM simulation_runs
		 WHERE artifacts_path != '' AND datetime(created_at) < datetime(?)
		 ORDER BY created_at ASC`,
		cutoff.UTC().Format("2006-01-02 15:04:05"),
	)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var records []RunArtifactRecord
	for rows.Next() {
		var r RunArtifactRecord
		if err := rows.Scan(&r.RunID, &r.WorkspaceID, &r.ArtifactsPath, &r.CreatedAt); err != nil {
			return nil, err
		}
		records = append(records, r)
	}
	return records, nil
}

// ClearRunArtifactsPath removes the artifacts_path pointer for a run.
func (s *Store) ClearRunArtifactsPath(runID string) error {
	_, err := s.db.Exec(
		`UPDATE simulation_runs SET artifacts_path = '' WHERE run_id = ?`,
		runID,
	)
	return err
}

// PruneTerminalRunsBefore deletes old terminal runs with no artifact pointers.
// Returns number of rows deleted.
func (s *Store) PruneTerminalRunsBefore(cutoff time.Time) (int64, error) {
	res, err := s.db.Exec(
		`DELETE FROM simulation_runs
		 WHERE datetime(created_at) < datetime(?)
		   AND artifacts_path = ''
		   AND status IN ('pass', 'fail', 'error')`,
		cutoff.UTC().Format("2006-01-02 15:04:05"),
	)
	if err != nil {
		return 0, err
	}
	n, err := res.RowsAffected()
	if err != nil {
		return 0, err
	}
	return n, nil
}

// PruneIdempotencyRequestsBefore deletes old idempotency records.
func (s *Store) PruneIdempotencyRequestsBefore(cutoff time.Time) (int64, error) {
	res, err := s.db.Exec(
		`DELETE FROM idempotency_requests
		 WHERE datetime(created_at) < datetime(?)`,
		cutoff.UTC().Format("2006-01-02 15:04:05"),
	)
	if err != nil {
		return 0, err
	}
	n, err := res.RowsAffected()
	if err != nil {
		return 0, err
	}
	return n, nil
}

// ListRunsForWorkspace returns the 50 most recent runs for the workspace.
func (s *Store) ListRunsForWorkspace(workspaceID string) ([]RunRecord, error) {
	rows, err := s.db.Query(
		`SELECT run_id, workspace_id, status, assertions_passed, assertions_total, artifacts_path, created_at
		 FROM simulation_runs WHERE workspace_id = ?
		 ORDER BY created_at DESC LIMIT 50`, workspaceID,
	)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var records []RunRecord
	for rows.Next() {
		var r RunRecord
		if err := rows.Scan(&r.RunID, &r.WorkspaceID, &r.Status, &r.AssertionsPassed, &r.AssertionsTotal, &r.ArtifactsPath, &r.CreatedAt); err != nil {
			return nil, err
		}
		records = append(records, r)
	}
	return records, nil
}

// ListRecoverableRuns returns runs that were in-flight during a crash/restart.
// These rows should be reconstructed and re-queued on process startup.
func (s *Store) ListRecoverableRuns() ([]RunRecord, error) {
	rows, err := s.db.Query(
		`SELECT run_id, workspace_id, status, assertions_passed, assertions_total, artifacts_path, created_at
		 FROM simulation_runs
		 WHERE status IN ('queued', 'running')
		 ORDER BY created_at ASC`,
	)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var records []RunRecord
	for rows.Next() {
		var r RunRecord
		if err := rows.Scan(&r.RunID, &r.WorkspaceID, &r.Status, &r.AssertionsPassed, &r.AssertionsTotal, &r.ArtifactsPath, &r.CreatedAt); err != nil {
			return nil, err
		}
		records = append(records, r)
	}
	return records, nil
}

// ListQueuedRuns returns oldest queued runs up to limit.
func (s *Store) ListQueuedRuns(limit int) ([]RunRecord, error) {
	if limit <= 0 {
		limit = 100
	}
	rows, err := s.db.Query(
		`SELECT run_id, workspace_id, status, assertions_passed, assertions_total, artifacts_path, created_at
		 FROM simulation_runs
		 WHERE status = 'queued'
		 ORDER BY created_at ASC
		 LIMIT ?`,
		limit,
	)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var records []RunRecord
	for rows.Next() {
		var r RunRecord
		if err := rows.Scan(&r.RunID, &r.WorkspaceID, &r.Status, &r.AssertionsPassed, &r.AssertionsTotal, &r.ArtifactsPath, &r.CreatedAt); err != nil {
			return nil, err
		}
		records = append(records, r)
	}
	return records, nil
}

// BeginIdempotencyRequest creates a pending idempotency record.
// Returns:
// - (new=true, existing=nil) when caller should proceed and later complete/cancel.
// - (new=false, existing!=nil) when request key already exists.
func (s *Store) BeginIdempotencyRequest(workspaceID, endpoint, key string) (bool, *IdempotencyRecord, error) {
	res, err := s.db.Exec(
		`INSERT OR IGNORE INTO idempotency_requests (workspace_id, endpoint, idempotency_key)
		 VALUES (?, ?, ?)`,
		workspaceID, endpoint, key,
	)
	if err != nil {
		return false, nil, err
	}
	inserted, err := res.RowsAffected()
	if err != nil {
		return false, nil, err
	}
	row := s.db.QueryRow(
		`SELECT run_id, status_code, response_body, created_at
		 FROM idempotency_requests
		 WHERE workspace_id = ? AND endpoint = ? AND idempotency_key = ?`,
		workspaceID, endpoint, key,
	)
	var rec IdempotencyRecord
	rec.WorkspaceID = workspaceID
	rec.Endpoint = endpoint
	rec.Key = key
	if err := row.Scan(&rec.RunID, &rec.StatusCode, &rec.ResponseBody, &rec.CreatedAt); err != nil {
		return false, nil, err
	}
	return inserted == 1, &rec, nil
}

// CompleteIdempotencyRequest stores the final replay response.
func (s *Store) CompleteIdempotencyRequest(workspaceID, endpoint, key, runID string, statusCode int, responseBody string) error {
	_, err := s.db.Exec(
		`UPDATE idempotency_requests
		 SET run_id = ?, status_code = ?, response_body = ?
		 WHERE workspace_id = ? AND endpoint = ? AND idempotency_key = ?`,
		runID, statusCode, responseBody, workspaceID, endpoint, key,
	)
	return err
}

// CancelPendingIdempotencyRequest removes a pending placeholder.
func (s *Store) CancelPendingIdempotencyRequest(workspaceID, endpoint, key string) error {
	_, err := s.db.Exec(
		`DELETE FROM idempotency_requests
		 WHERE workspace_id = ? AND endpoint = ? AND idempotency_key = ? AND status_code = 0`,
		workspaceID, endpoint, key,
	)
	return err
}

// CountRunsForWorkspace returns the number of runs in the last 30 days.
func (s *Store) CountRunsForWorkspace(workspaceID string) (int, error) {
	var count int
	err := s.db.QueryRow(`
		SELECT COALESCE(SUM(billable), 0) FROM simulation_runs
		WHERE workspace_id = ? AND datetime(created_at) >= datetime('now', '-30 days')
	`, workspaceID).Scan(&count)
	return count, err
}

// SetRunBillable updates quota accounting participation for a run.
func (s *Store) SetRunBillable(runID string, billable bool) error {
	v := 0
	if billable {
		v = 1
	}
	_, err := s.db.Exec(`UPDATE simulation_runs SET billable = ? WHERE run_id = ?`, v, runID)
	return err
}

// GetMonthlyQuota returns the monthly_quota for the workspace associated with the API key.
func (s *Store) GetMonthlyQuota(workspaceID string) (int, error) {
	var quota int
	err := s.db.QueryRow(
		`SELECT monthly_quota FROM api_keys WHERE workspace_id = ? AND revoked = FALSE LIMIT 1`,
		workspaceID,
	).Scan(&quota)
	if err != nil {
		return 1000, err // safe default
	}
	return quota, nil
}

// AddQuotaRuns increases the monthly_quota for a workspace (used by Stripe webhook credits).
func (s *Store) AddQuotaRuns(workspaceID string, runs int) error {
	_, err := s.db.Exec(
		`UPDATE api_keys SET monthly_quota = monthly_quota + ? WHERE workspace_id = ? AND revoked = FALSE`,
		runs, workspaceID,
	)
	return err
}

// ApplyStripeCreditIfNew atomically credits quota exactly once per Stripe event ID.
// Returns (applied=true) if credit was newly applied, (applied=false) for duplicate events.
func (s *Store) ApplyStripeCreditIfNew(eventID, sessionID, workspaceID string, runs int) (bool, error) {
	tx, err := s.db.Begin()
	if err != nil {
		return false, err
	}
	defer func() {
		_ = tx.Rollback()
	}()

	insertRes, err := tx.Exec(
		`INSERT OR IGNORE INTO stripe_events (event_id, session_id, workspace_id, runs_credited)
		 VALUES (?, ?, ?, ?)`,
		eventID, sessionID, workspaceID, runs,
	)
	if err != nil {
		return false, err
	}
	inserted, err := insertRes.RowsAffected()
	if err != nil {
		return false, err
	}
	if inserted == 0 {
		// Duplicate webhook event: already processed.
		if err := tx.Commit(); err != nil {
			return false, err
		}
		return false, nil
	}

	updateRes, err := tx.Exec(
		`UPDATE api_keys
		 SET monthly_quota = monthly_quota + ?
		 WHERE workspace_id = ? AND revoked = FALSE`,
		runs, workspaceID,
	)
	if err != nil {
		return false, err
	}
	updated, err := updateRes.RowsAffected()
	if err != nil {
		return false, err
	}
	if updated == 0 {
		return false, fmt.Errorf("workspace not found or all keys revoked: %s", workspaceID)
	}

	if err := tx.Commit(); err != nil {
		return false, err
	}
	return true, nil
}

// SeedHardware replaces the existing supported hardware catalog.
// It uses a transaction to clear and bulk-insert the new list.
func (s *Store) SeedHardware(items []HardwareItem) error {
	tx, err := s.db.Begin()
	if err != nil {
		return err
	}
	defer func() {
		_ = tx.Rollback()
	}()

	if _, err := tx.Exec(`DELETE FROM supported_hardware`); err != nil {
		return err
	}

	stmt, err := tx.Prepare(`INSERT INTO supported_hardware (id, name, type, repl_path, tier) VALUES (?, ?, ?, ?, ?)`)
	if err != nil {
		return err
	}
	defer stmt.Close()

	for _, item := range items {
		if _, err := stmt.Exec(item.ID, item.Name, item.Type, item.ReplPath, item.Tier); err != nil {
			return err
		}
	}

	return tx.Commit()
}

// ListHardware returns all supported hardware, sorted by tier, type, and name.
func (s *Store) ListHardware() ([]HardwareItem, error) {
	rows, err := s.db.Query(
		`SELECT id, name, type, repl_path, tier
		 FROM supported_hardware
		 ORDER BY tier ASC, type ASC, name ASC`,
	)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var items []HardwareItem
	for rows.Next() {
		var item HardwareItem
		if err := rows.Scan(&item.ID, &item.Name, &item.Type, &item.ReplPath, &item.Tier); err != nil {
			return nil, err
		}
		items = append(items, item)
	}
	return items, nil
}

// UpsertCatalogAsset inserts or updates a catalog asset.
func (s *Store) UpsertCatalogAsset(asset CatalogAsset) error {
	verified := 0
	if asset.Verified {
		verified = 1
	}
	_, err := s.db.Exec(
		`INSERT INTO catalog_assets (id, name, description, pass_rate, registers, ir_url, verified, source_type, source_ref)
		 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
		 ON CONFLICT(id) DO UPDATE SET
		   name = excluded.name,
		   description = excluded.description,
		   pass_rate = excluded.pass_rate,
		   registers = excluded.registers,
		   ir_url = excluded.ir_url,
		   verified = excluded.verified,
		   source_type = excluded.source_type,
		   source_ref = excluded.source_ref`,
		asset.ID, asset.Name, asset.Description, asset.PassRate, asset.Registers, asset.IrURL, verified, asset.SourceType, asset.SourceRef,
	)
	return err
}

// ListCatalogAssets returns all assets in the catalog.
func (s *Store) ListCatalogAssets() ([]CatalogAsset, error) {
	rows, err := s.db.Query(`SELECT id, name, description, pass_rate, registers, ir_url, verified, source_type, source_ref FROM catalog_assets ORDER BY id ASC`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var assets []CatalogAsset
	for rows.Next() {
		var a CatalogAsset
		var verified int
		if err := rows.Scan(&a.ID, &a.Name, &a.Description, &a.PassRate, &a.Registers, &a.IrURL, &verified, &a.SourceType, &a.SourceRef); err != nil {
			return nil, err
		}
		a.Verified = verified == 1
		assets = append(assets, a)
	}
	return assets, nil
}

// AccountUsage holds aggregated quota/usage for a Clerk account.
type AccountUsage struct {
	ClerkUserID        string `json:"clerk_user_id"`
	Tier               string `json:"tier"`
	RunsUsedThisMonth  int    `json:"runs_used_this_month"`
	Quota              int    `json:"quota"`
	RunsRemaining      int    `json:"runs_remaining"`
}

// GetAccountUsage returns aggregated usage across all non-revoked API keys for a Clerk user.
func (s *Store) GetAccountUsage(clerkUserID string) (AccountUsage, error) {
	var used, quota int
	var tier string
	err := s.db.QueryRow(`
		SELECT
			COALESCE(SUM(r.billable), 0),
			COALESCE(SUM(k.monthly_quota), 0),
			COALESCE(MAX(k.tier), 'builder')
		FROM api_keys k
		LEFT JOIN simulation_runs r
			ON r.workspace_id = k.workspace_id
			AND datetime(r.created_at) >= datetime('now', '-30 days')
		WHERE k.clerk_user_id = ? AND k.revoked = FALSE
	`, clerkUserID).Scan(&used, &quota, &tier)
	if err != nil {
		return AccountUsage{}, err
	}
	if quota == 0 {
		quota = 1000
	}
	remaining := quota - used
	if remaining < 0 {
		remaining = 0
	}
	return AccountUsage{
		ClerkUserID:       clerkUserID,
		Tier:              tier,
		RunsUsedThisMonth: used,
		Quota:             quota,
		RunsRemaining:     remaining,
	}, nil
}

// ListRunsForClerkUser returns the 50 most recent runs across all API keys for a Clerk user.
func (s *Store) ListRunsForClerkUser(clerkUserID string) ([]RunRecord, error) {
	rows, err := s.db.Query(`
		SELECT r.run_id, r.workspace_id, r.status, r.assertions_passed, r.assertions_total, r.artifacts_path, r.created_at
		FROM simulation_runs r
		JOIN api_keys k ON k.workspace_id = r.workspace_id
		WHERE k.clerk_user_id = ? AND k.revoked = FALSE
		ORDER BY r.created_at DESC LIMIT 50
	`, clerkUserID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var records []RunRecord
	for rows.Next() {
		var r RunRecord
		if err := rows.Scan(&r.RunID, &r.WorkspaceID, &r.Status, &r.AssertionsPassed, &r.AssertionsTotal, &r.ArtifactsPath, &r.CreatedAt); err != nil {
			return nil, err
		}
		records = append(records, r)
	}
	return records, nil
}

// GetCatalogAsset returns a single asset by ID.
func (s *Store) GetCatalogAsset(id string) (CatalogAsset, bool, error) {
	var a CatalogAsset
	var verified int
	err := s.db.QueryRow(
		`SELECT id, name, description, pass_rate, registers, ir_url, verified, source_type, source_ref FROM catalog_assets WHERE id = ?`,
		id,
	).Scan(&a.ID, &a.Name, &a.Description, &a.PassRate, &a.Registers, &a.IrURL, &verified, &a.SourceType, &a.SourceRef)
	if err != nil {
		if err == sql.ErrNoRows {
			return a, false, nil
		}
		return a, false, err
	}
	a.Verified = verified == 1
	return a, true, nil
}

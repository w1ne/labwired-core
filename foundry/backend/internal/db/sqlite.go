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

// HardwareItem represents a Renode-supported board or CPU in the database.
type HardwareItem struct {
	ID       string `json:"id"`
	Name     string `json:"name"`
	Type     string `json:"type"` // "board" or "cpu"
	ReplPath string `json:"repl_path"`
	Tier     int    `json:"tier"` // 1 (Top 20%) or 2 (Extended)
}

type Store struct {
	db *sql.DB
}

var ErrQuotaExceeded = errors.New("quota exceeded")

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
		`CREATE TABLE IF NOT EXISTS supported_hardware (
			id TEXT PRIMARY KEY,
			name TEXT NOT NULL,
			type TEXT NOT NULL,
			repl_path TEXT NOT NULL,
			tier INTEGER NOT NULL DEFAULT 2
		);`,
		// Non-destructive: add monthly_quota column if it was missing in an older DB.
		`ALTER TABLE api_keys ADD COLUMN monthly_quota INTEGER NOT NULL DEFAULT 1000;`,
		// Non-destructive: add key_prefix column if missing.
		`ALTER TABLE api_keys ADD COLUMN key_prefix TEXT NOT NULL DEFAULT '';`,
		// Non-destructive: add artifacts_path column if missing.
		`ALTER TABLE simulation_runs ADD COLUMN artifacts_path TEXT DEFAULT '';`,
		`CREATE INDEX IF NOT EXISTS idx_api_keys_prefix ON api_keys(key_prefix);`,
		`CREATE INDEX IF NOT EXISTS idx_simulation_runs_workspace_created ON simulation_runs(workspace_id, created_at);`,
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
		   SELECT COUNT(*) FROM simulation_runs
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
		`SELECT COUNT(*) FROM simulation_runs
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
		"UPDATE simulation_runs SET status = ?, assertions_passed = ?, assertions_total = ?, artifacts_path = ? WHERE run_id = ?",
		status, passed, total, artifactsPath, runID,
	)
	return err
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

// CountRunsForWorkspace returns the number of runs in the last 30 days.
func (s *Store) CountRunsForWorkspace(workspaceID string) (int, error) {
	var count int
	err := s.db.QueryRow(`
		SELECT COUNT(*) FROM simulation_runs
		WHERE workspace_id = ? AND datetime(created_at) >= datetime('now', '-30 days')
	`, workspaceID).Scan(&count)
	return count, err
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

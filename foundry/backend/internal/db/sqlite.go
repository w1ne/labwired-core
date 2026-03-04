package db

import (
	"database/sql"
	"fmt"

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

type Store struct {
	db *sql.DB
}

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

func (s *Store) migrate() error {
	queries := []string{
		`PRAGMA journal_mode=WAL;`,
		`PRAGMA busy_timeout = 5000;`,
		`CREATE TABLE IF NOT EXISTS api_keys (
			id UUID PRIMARY KEY,
			key_hash TEXT NOT NULL,
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
		// Non-destructive: add monthly_quota column if it was missing in an older DB.
		`ALTER TABLE api_keys ADD COLUMN monthly_quota INTEGER NOT NULL DEFAULT 1000;`,
		// Non-destructive: add artifacts_path column if missing.
		`ALTER TABLE simulation_runs ADD COLUMN artifacts_path TEXT DEFAULT '';`,
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

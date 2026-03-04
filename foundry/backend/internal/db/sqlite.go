package db

import (
	"database/sql"
	"fmt"

	_ "modernc.org/sqlite"
)

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

func (s *Store) migrate() error {
	queries := []string{
		`CREATE TABLE IF NOT EXISTS api_keys (
			id UUID PRIMARY KEY,
			key_hash TEXT NOT NULL,
			workspace_id UUID NOT NULL,
			tier TEXT NOT NULL DEFAULT 'free',
			revoked BOOLEAN DEFAULT FALSE
		);`,
		`CREATE TABLE IF NOT EXISTS simulation_runs (
			run_id UUID PRIMARY KEY,
			workspace_id UUID NOT NULL,
			status TEXT NOT NULL,
			assertions_passed INTEGER,
			assertions_total INTEGER,
			created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
		);`,
	}

	for _, q := range queries {
		if _, err := s.db.Exec(q); err != nil {
			return fmt.Errorf("migration failed: %w", err)
		}
	}

	return nil
}

func (s *Store) SaveRun(runID, workspaceID, status string) error {
	_, err := s.db.Exec(
		"INSERT INTO simulation_runs (run_id, workspace_id, status) VALUES (?, ?, ?)",
		runID, workspaceID, status,
	)
	return err
}

func (s *Store) UpdateRunStatus(runID, status string, passed, total int) error {
	_, err := s.db.Exec(
		"UPDATE simulation_runs SET status = ?, assertions_passed = ?, assertions_total = ? WHERE run_id = ?",
		status, passed, total, runID,
	)
	return err
}

func (s *Store) CountRunsForWorkspace(workspaceID string) (int, error) {
	var count int
	err := s.db.QueryRow(`
		SELECT COUNT(*) FROM simulation_runs
		WHERE workspace_id = ? AND datetime(created_at) >= datetime('now', '-30 days')
	`, workspaceID).Scan(&count)
	return count, err
}

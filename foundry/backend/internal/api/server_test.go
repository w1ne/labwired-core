package api

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/labwired/foundry-backend/internal/catalog"
	"github.com/labwired/foundry-backend/internal/db"
	"github.com/labwired/foundry-backend/internal/verification"
)

func newTestServer(t *testing.T) (*Server, *db.Store, string) {
	t.Helper()

	root := t.TempDir()
	dbPath := filepath.Join(root, "foundry_test.db")
	artifactsDir := filepath.Join(root, "artifacts")

	store, err := db.NewStore(dbPath)
	if err != nil {
		t.Fatalf("db.NewStore failed: %v", err)
	}
	t.Cleanup(func() {
		_ = store.Close()
	})

	srv := NewServer(verification.NewOrchestrator("labwired"), store, catalog.NewManager(), artifactsDir)
	t.Cleanup(func() {
		ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
		defer cancel()
		_ = srv.Shutdown(ctx)
	})

	return srv, store, artifactsDir
}

func createKey(t *testing.T, store *db.Store, workspaceID string) string {
	t.Helper()
	const keyPrefix = "lw_sk_live_"
	key := keyPrefix + workspaceID + "_test_key"
	if _, err := store.CreateKey(workspaceID, key); err != nil {
		t.Fatalf("CreateKey failed: %v", err)
	}
	return key
}

func doAuthRequest(t *testing.T, srv *Server, method, path, key string, body []byte) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequest(method, path, bytes.NewReader(body))
	if key != "" {
		req.Header.Set("Authorization", "Bearer "+key)
	}
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	rr := httptest.NewRecorder()
	srv.ServeHTTP(rr, req)
	return rr
}

func TestGetRunArtifact_EnforcesWorkspaceOwnership(t *testing.T) {
	srv, store, artifactsDir := newTestServer(t)

	keyOwner := createKey(t, store, "ws-owner")
	keyOther := createKey(t, store, "ws-other")

	runID := "run-artifact-1"
	runArtifactsDir := filepath.Join(artifactsDir, runID)
	if err := os.MkdirAll(runArtifactsDir, 0o755); err != nil {
		t.Fatalf("MkdirAll failed: %v", err)
	}
	expected := []byte(`{"ok":true}`)
	if err := os.WriteFile(filepath.Join(runArtifactsDir, "output.json"), expected, 0o644); err != nil {
		t.Fatalf("WriteFile failed: %v", err)
	}

	if err := store.SaveRun(runID, "ws-owner", "queued"); err != nil {
		t.Fatalf("SaveRun failed: %v", err)
	}
	if err := store.UpdateRunStatus(runID, "pass", 1, 1, runArtifactsDir); err != nil {
		t.Fatalf("UpdateRunStatus failed: %v", err)
	}

	rrOther := doAuthRequest(t, srv, http.MethodGet, "/v1/runs/"+runID+"/artifacts/output.json", keyOther, nil)
	if rrOther.Code != http.StatusNotFound {
		t.Fatalf("expected 404 for non-owner, got %d body=%s", rrOther.Code, rrOther.Body.String())
	}

	rrOwner := doAuthRequest(t, srv, http.MethodGet, "/v1/runs/"+runID+"/artifacts/output.json", keyOwner, nil)
	if rrOwner.Code != http.StatusOK {
		t.Fatalf("expected 200 for owner, got %d body=%s", rrOwner.Code, rrOwner.Body.String())
	}
	if rrOwner.Body.String() != string(expected) {
		t.Fatalf("unexpected artifact body: got=%q want=%q", rrOwner.Body.String(), string(expected))
	}
}

func TestSubmitJob_InflightLimitReturns429(t *testing.T) {
	t.Setenv("WORKSPACE_MAX_INFLIGHT", "1")

	srv, store, _ := newTestServer(t)
	key := createKey(t, store, "ws-limit")

	// Simulate one in-flight job already occupying the workspace slot.
	srv.inflightMu.Lock()
	srv.inflightByWorkspace["ws-limit"] = 1
	srv.inflightMu.Unlock()

	body := []byte(`{"chip_yaml":"registers: []"}`)
	rr := doAuthRequest(t, srv, http.MethodPost, "/v1/models/verify", key, body)
	if rr.Code != http.StatusTooManyRequests {
		t.Fatalf("expected 429, got %d body=%s", rr.Code, rr.Body.String())
	}

	var apiErr APIError
	if err := json.Unmarshal(rr.Body.Bytes(), &apiErr); err != nil {
		t.Fatalf("failed to decode APIError: %v", err)
	}
	if apiErr.Code != "WORKSPACE_INFLIGHT_LIMIT" {
		t.Fatalf("unexpected error code: got=%s", apiErr.Code)
	}
}

func TestSubmitJob_ShuttingDownReturns503(t *testing.T) {
	srv, store, _ := newTestServer(t)
	key := createKey(t, store, "ws-shutdown")

	srv.queueMu.Lock()
	srv.shuttingDown = true
	srv.queueMu.Unlock()

	body := []byte(`{"chip_yaml":"registers: []"}`)
	rr := doAuthRequest(t, srv, http.MethodPost, "/v1/models/verify", key, body)
	if rr.Code != http.StatusServiceUnavailable {
		t.Fatalf("expected 503, got %d body=%s", rr.Code, rr.Body.String())
	}

	var apiErr APIError
	if err := json.Unmarshal(rr.Body.Bytes(), &apiErr); err != nil {
		t.Fatalf("failed to decode APIError: %v", err)
	}
	if apiErr.Code != "SERVER_SHUTTING_DOWN" {
		t.Fatalf("unexpected error code: got=%s", apiErr.Code)
	}
}

func TestStripeWebhook_DuplicateEventDoesNotDoubleCredit(t *testing.T) {
	t.Setenv("ALLOW_INSECURE_STRIPE_WEBHOOKS", "true")
	t.Setenv("STRIPE_WEBHOOK_SECRET", "")

	srv, store, _ := newTestServer(t)
	_ = createKey(t, store, "ws-stripe")

	payload := []byte(`{
		"id": "evt_duplicate_1",
		"type": "checkout.session.completed",
		"data": {
			"object": {
				"id": "cs_test_1",
				"client_reference_id": "ws-stripe",
				"amount_total": 4900
			}
		}
	}`)

	quotaBefore, err := store.GetMonthlyQuota("ws-stripe")
	if err != nil {
		t.Fatalf("GetMonthlyQuota before failed: %v", err)
	}

	rr1 := doAuthRequest(t, srv, http.MethodPost, "/v1/webhooks/stripe", "", payload)
	if rr1.Code != http.StatusOK {
		t.Fatalf("first webhook call expected 200, got %d body=%s", rr1.Code, rr1.Body.String())
	}

	quotaAfterFirst, err := store.GetMonthlyQuota("ws-stripe")
	if err != nil {
		t.Fatalf("GetMonthlyQuota after first failed: %v", err)
	}
	if quotaAfterFirst != quotaBefore+1000 {
		t.Fatalf("unexpected first credit: got=%d want=%d", quotaAfterFirst, quotaBefore+1000)
	}

	rr2 := doAuthRequest(t, srv, http.MethodPost, "/v1/webhooks/stripe", "", payload)
	if rr2.Code != http.StatusOK {
		t.Fatalf("second webhook call expected 200, got %d body=%s", rr2.Code, rr2.Body.String())
	}

	quotaAfterSecond, err := store.GetMonthlyQuota("ws-stripe")
	if err != nil {
		t.Fatalf("GetMonthlyQuota after second failed: %v", err)
	}
	if quotaAfterSecond != quotaAfterFirst {
		t.Fatalf("duplicate event should not credit again: first=%d second=%d", quotaAfterFirst, quotaAfterSecond)
	}
}

func TestCleanupExpiredArtifacts_RemovesDirAndClearsDBPath(t *testing.T) {
	srv, store, artifactsDir := newTestServer(t)

	runID := "run-cleanup-1"
	runArtifactsDir := filepath.Join(artifactsDir, runID)
	if err := os.MkdirAll(runArtifactsDir, 0o755); err != nil {
		t.Fatalf("MkdirAll failed: %v", err)
	}
	if err := os.WriteFile(filepath.Join(runArtifactsDir, "result.json"), []byte(`{"ok":true}`), 0o644); err != nil {
		t.Fatalf("WriteFile failed: %v", err)
	}

	if err := store.SaveRun(runID, "ws-cleanup", "queued"); err != nil {
		t.Fatalf("SaveRun failed: %v", err)
	}
	if err := store.UpdateRunStatus(runID, "pass", 1, 1, runArtifactsDir); err != nil {
		t.Fatalf("UpdateRunStatus failed: %v", err)
	}

	// Force immediate eligibility in test.
	srv.artifactRetentionDays = -1
	if err := srv.cleanupExpiredArtifactsOnce(time.Now()); err != nil {
		t.Fatalf("cleanupExpiredArtifactsOnce failed: %v", err)
	}

	if _, err := os.Stat(runArtifactsDir); !os.IsNotExist(err) {
		t.Fatalf("expected artifacts directory to be removed, stat err=%v", err)
	}

	record, err := store.GetRunForWorkspace(runID, "ws-cleanup")
	if err != nil {
		t.Fatalf("GetRunForWorkspace failed: %v", err)
	}
	if record == nil {
		t.Fatalf("expected run record to exist")
	}
	if record.ArtifactsPath != "" {
		t.Fatalf("expected artifacts_path to be cleared, got=%q", record.ArtifactsPath)
	}
}

func TestCleanupExpiredArtifacts_DoesNotDeletePathsOutsideRoot(t *testing.T) {
	srv, store, _ := newTestServer(t)

	outsideRoot := filepath.Join(t.TempDir(), "outside-artifacts")
	if err := os.MkdirAll(outsideRoot, 0o755); err != nil {
		t.Fatalf("MkdirAll failed: %v", err)
	}
	outsideFile := filepath.Join(outsideRoot, "result.json")
	if err := os.WriteFile(outsideFile, []byte(`{"outside":true}`), 0o644); err != nil {
		t.Fatalf("WriteFile failed: %v", err)
	}

	runID := "run-cleanup-outside"
	if err := store.SaveRun(runID, "ws-cleanup-outside", "queued"); err != nil {
		t.Fatalf("SaveRun failed: %v", err)
	}
	if err := store.UpdateRunStatus(runID, "pass", 1, 1, outsideRoot); err != nil {
		t.Fatalf("UpdateRunStatus failed: %v", err)
	}

	srv.artifactRetentionDays = -1
	if err := srv.cleanupExpiredArtifactsOnce(time.Now()); err != nil {
		t.Fatalf("cleanupExpiredArtifactsOnce failed: %v", err)
	}

	if _, err := os.Stat(outsideFile); err != nil {
		t.Fatalf("expected outside artifact file to remain, err=%v", err)
	}

	record, err := store.GetRunForWorkspace(runID, "ws-cleanup-outside")
	if err != nil {
		t.Fatalf("GetRunForWorkspace failed: %v", err)
	}
	if record == nil {
		t.Fatalf("expected run record to exist")
	}
	if record.ArtifactsPath == "" {
		t.Fatalf("expected artifacts_path to remain for outside path")
	}
}

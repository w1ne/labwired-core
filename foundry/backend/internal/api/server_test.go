package api

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/labwired/foundry-backend/internal/catalog"
	"github.com/labwired/foundry-backend/internal/db"
	"github.com/labwired/foundry-backend/internal/verification"
)

func newTestServer(t *testing.T) (*Server, *db.Store, string) {
	return newTestServerWithOptions(t, nil)
}

func newTestServerWithOptions(t *testing.T, mutate func(*ServerOptions)) (*Server, *db.Store, string) {
	t.Helper()

	root := t.TempDir()
	dbPath := filepath.Join(root, "foundry_test.db")
	artifactsDir := filepath.Join(root, "artifacts")
	dataDir := filepath.Join(root, "data")

	store, err := db.NewStore(dbPath)
	if err != nil {
		t.Fatalf("db.NewStore failed: %v", err)
	}
	t.Cleanup(func() {
		_ = store.Close()
	})

	orch := verification.NewOrchestrator("labwired")
	opts := DefaultServerOptions()
	opts.MaxInflightPerWorkspace = 1
	if mutate != nil {
		mutate(&opts)
	}
	srv := NewServer(orch, store, catalog.NewManager(store), artifactsDir, dataDir, opts)
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

func doAuthRequestWithHeaders(t *testing.T, srv *Server, method, path, key string, body []byte, headers map[string]string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequest(method, path, bytes.NewReader(body))
	if key != "" {
		req.Header.Set("Authorization", "Bearer "+key)
	}
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	for k, v := range headers {
		req.Header.Set(k, v)
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

func TestAuthMiddleware_UnauthorizedReturnsJSON(t *testing.T) {
	srv, _, _ := newTestServer(t)

	rr := doAuthRequest(t, srv, http.MethodGet, "/v1/usage", "", nil)
	if rr.Code != http.StatusUnauthorized {
		t.Fatalf("expected 401, got %d body=%s", rr.Code, rr.Body.String())
	}

	var apiErr APIError
	if err := json.Unmarshal(rr.Body.Bytes(), &apiErr); err != nil {
		t.Fatalf("failed to decode APIError: %v", err)
	}
	if apiErr.Code != "UNAUTHORIZED" {
		t.Fatalf("unexpected error code: %s", apiErr.Code)
	}
}

func TestCORS_AllowsIdempotencyAndExposesRateLimitHeaders(t *testing.T) {
	srv, _, _ := newTestServer(t)

	req := httptest.NewRequest(http.MethodOptions, "/v1/usage", nil)
	rr := httptest.NewRecorder()
	srv.ServeHTTP(rr, req)
	if rr.Code != http.StatusOK {
		t.Fatalf("expected 200 for CORS preflight, got %d", rr.Code)
	}
	allowHeaders := rr.Header().Get("Access-Control-Allow-Headers")
	if !strings.Contains(allowHeaders, "Idempotency-Key") {
		t.Fatalf("expected Access-Control-Allow-Headers to include Idempotency-Key, got %q", allowHeaders)
	}
	exposeHeaders := rr.Header().Get("Access-Control-Expose-Headers")
	if !strings.Contains(exposeHeaders, "X-RateLimit-Remaining") {
		t.Fatalf("expected Access-Control-Expose-Headers to include rate-limit headers, got %q", exposeHeaders)
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

func TestSubmitJob_GlobalInflightLimitFromDBReturns429(t *testing.T) {
	srv, store, _ := newTestServerWithOptions(t, func(opts *ServerOptions) {
		opts.MaxInflightPerWorkspace = 1
	})
	key := createKey(t, store, "ws-global-inflight")

	if err := store.SaveRun("existing-queued-run", "ws-global-inflight", "queued"); err != nil {
		t.Fatalf("SaveRun failed: %v", err)
	}

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

func TestRateLimit_PerAPIKeyExceeded(t *testing.T) {
	srv, store, _ := newTestServerWithOptions(t, func(opts *ServerOptions) {
		opts.RateLimitPerAPIKey = 1
		opts.RateLimitPerWorkspace = 10
		opts.RateLimitWindow = time.Minute
	})
	key := createKey(t, store, "ws-rate-key")

	rr1 := doAuthRequest(t, srv, http.MethodGet, "/v1/usage", key, nil)
	if rr1.Code != http.StatusOK {
		t.Fatalf("expected first request 200, got %d body=%s", rr1.Code, rr1.Body.String())
	}
	if got := rr1.Header().Get("X-RateLimit-Limit"); got != "1" {
		t.Fatalf("expected X-RateLimit-Limit=1, got %q", got)
	}
	rr2 := doAuthRequest(t, srv, http.MethodGet, "/v1/usage", key, nil)
	if rr2.Code != http.StatusTooManyRequests {
		t.Fatalf("expected second request 429, got %d body=%s", rr2.Code, rr2.Body.String())
	}

	var apiErr APIError
	if err := json.Unmarshal(rr2.Body.Bytes(), &apiErr); err != nil {
		t.Fatalf("failed to decode APIError: %v", err)
	}
	if apiErr.Code != "RATE_LIMITED" {
		t.Fatalf("unexpected error code: %s", apiErr.Code)
	}
	if got := srv.metrics.RateLimitRejected.Load(); got != 1 {
		t.Fatalf("expected rate_limit_rejected metric 1, got %d", got)
	}
}

func TestRateLimit_PerWorkspaceAppliesAcrossKeys(t *testing.T) {
	srv, store, _ := newTestServerWithOptions(t, func(opts *ServerOptions) {
		opts.RateLimitPerAPIKey = 10
		opts.RateLimitPerWorkspace = 1
		opts.RateLimitWindow = time.Minute
	})
	key1 := createKey(t, store, "ws-rate-shared")
	key2 := createKey(t, store, "ws-rate-shared")

	rr1 := doAuthRequest(t, srv, http.MethodGet, "/v1/usage", key1, nil)
	if rr1.Code != http.StatusOK {
		t.Fatalf("expected first request 200, got %d body=%s", rr1.Code, rr1.Body.String())
	}
	rr2 := doAuthRequest(t, srv, http.MethodGet, "/v1/usage", key2, nil)
	if rr2.Code != http.StatusTooManyRequests {
		t.Fatalf("expected second request 429, got %d body=%s", rr2.Code, rr2.Body.String())
	}

	var apiErr APIError
	if err := json.Unmarshal(rr2.Body.Bytes(), &apiErr); err != nil {
		t.Fatalf("failed to decode APIError: %v", err)
	}
	if apiErr.Code != "RATE_LIMITED" {
		t.Fatalf("unexpected error code: %s", apiErr.Code)
	}
}

func TestSubmitJob_ShuttingDownReturns503(t *testing.T) {
	srv, store, _ := newTestServer(t)
	key := createKey(t, store, "ws-shutdown")

	srv.scheduleMu.Lock()
	srv.shuttingDown = true
	srv.scheduleMu.Unlock()

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

	used, err := store.CountRunsForWorkspace("ws-shutdown")
	if err != nil {
		t.Fatalf("CountRunsForWorkspace failed: %v", err)
	}
	if used != 0 {
		t.Fatalf("expected no quota usage on shutdown rejection, got used=%d", used)
	}
}

func TestSubmitJob_QueueFullDoesNotConsumeQuota(t *testing.T) {
	srv, store, _ := newTestServer(t)
	key := createKey(t, store, "ws-queuefull")

	// Force immediate queue-full path.
	srv.maxPendingJobs = 0

	body := []byte(`{"chip_yaml":"registers: []"}`)
	rr := doAuthRequest(t, srv, http.MethodPost, "/v1/models/verify", key, body)
	if rr.Code != http.StatusServiceUnavailable {
		t.Fatalf("expected 503, got %d body=%s", rr.Code, rr.Body.String())
	}

	var apiErr APIError
	if err := json.Unmarshal(rr.Body.Bytes(), &apiErr); err != nil {
		t.Fatalf("failed to decode APIError: %v", err)
	}
	if apiErr.Code != "QUEUE_FULL" {
		t.Fatalf("unexpected error code: got=%s", apiErr.Code)
	}

	used, err := store.CountRunsForWorkspace("ws-queuefull")
	if err != nil {
		t.Fatalf("CountRunsForWorkspace failed: %v", err)
	}
	if used != 0 {
		t.Fatalf("expected no quota usage on queue full rejection, got used=%d", used)
	}
}

func TestSubmitJob_IdempotencyKeyReplaysResponse(t *testing.T) {
	srv, store, _ := newTestServer(t)
	key := createKey(t, store, "ws-idem-api")

	body := []byte(`{"chip_yaml":"registers: []"}`)
	headers := map[string]string{"Idempotency-Key": "idem-run-1"}
	rr1 := doAuthRequestWithHeaders(t, srv, http.MethodPost, "/v1/models/verify", key, body, headers)
	if rr1.Code != http.StatusAccepted {
		t.Fatalf("expected first request 202, got %d body=%s", rr1.Code, rr1.Body.String())
	}
	var resp1 map[string]any
	if err := json.Unmarshal(rr1.Body.Bytes(), &resp1); err != nil {
		t.Fatalf("decode first response failed: %v", err)
	}
	run1, _ := resp1["run_id"].(string)
	if run1 == "" {
		t.Fatalf("expected run_id in first response")
	}

	rr2 := doAuthRequestWithHeaders(t, srv, http.MethodPost, "/v1/models/verify", key, body, headers)
	if rr2.Code != http.StatusAccepted {
		t.Fatalf("expected second request 202 replay, got %d body=%s", rr2.Code, rr2.Body.String())
	}
	var resp2 map[string]any
	if err := json.Unmarshal(rr2.Body.Bytes(), &resp2); err != nil {
		t.Fatalf("decode second response failed: %v", err)
	}
	run2, _ := resp2["run_id"].(string)
	if run2 != run1 {
		t.Fatalf("expected replayed run_id %q, got %q", run1, run2)
	}

	used, err := store.CountRunsForWorkspace("ws-idem-api")
	if err != nil {
		t.Fatalf("CountRunsForWorkspace failed: %v", err)
	}
	if used != 1 {
		t.Fatalf("expected quota usage 1 after replay, got %d", used)
	}
}

func TestSubmitJob_InvalidIdempotencyKeyReturns400(t *testing.T) {
	srv, store, _ := newTestServer(t)
	key := createKey(t, store, "ws-idem-invalid")

	body := []byte(`{"chip_yaml":"registers: []"}`)
	headers := map[string]string{"Idempotency-Key": "invalid key with spaces"}
	rr := doAuthRequestWithHeaders(t, srv, http.MethodPost, "/v1/models/verify", key, body, headers)
	if rr.Code != http.StatusBadRequest {
		t.Fatalf("expected 400 for invalid idempotency key, got %d body=%s", rr.Code, rr.Body.String())
	}

	var apiErr APIError
	if err := json.Unmarshal(rr.Body.Bytes(), &apiErr); err != nil {
		t.Fatalf("failed to decode APIError: %v", err)
	}
	if apiErr.Code != "INVALID_IDEMPOTENCY_KEY" {
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

func TestSynthesize_ReturnsPollableRunID(t *testing.T) {
	srv, store, _ := newTestServer(t)
	key := createKey(t, store, "ws-synth")

	body := []byte(`{"component_name":"TMP117","requirements":"Expose temperature register and status bit."}`)
	rr := doAuthRequest(t, srv, http.MethodPost, "/v1/synthesize", key, body)
	if rr.Code != http.StatusAccepted {
		t.Fatalf("expected 202, got %d body=%s", rr.Code, rr.Body.String())
	}

	var resp struct {
		JobID   string `json:"job_id"`
		RunID   string `json:"run_id"`
		PollURL string `json:"poll_url"`
	}
	if err := json.Unmarshal(rr.Body.Bytes(), &resp); err != nil {
		t.Fatalf("failed to decode synth response: %v", err)
	}
	if resp.JobID == "" || resp.RunID == "" {
		t.Fatalf("expected non-empty job_id and run_id, got job_id=%q run_id=%q", resp.JobID, resp.RunID)
	}
	wantPoll := fmt.Sprintf("/v1/runs/%s", resp.RunID)
	if resp.PollURL != wantPoll {
		t.Fatalf("unexpected poll_url: got=%q want=%q", resp.PollURL, wantPoll)
	}

	record, err := store.GetRunForWorkspace(resp.RunID, "ws-synth")
	if err != nil {
		t.Fatalf("GetRunForWorkspace failed: %v", err)
	}
	if record == nil {
		t.Fatalf("expected primary synth run row to exist for polling")
	}
}

func TestSynthesize_FinalizesNonPrimaryReservedRuns(t *testing.T) {
	srv, store, _ := newTestServer(t)
	key := createKey(t, store, "ws-synth-reserve")

	// Long requirements force high-cost path (50 reservations).
	body := []byte(`{"component_name":"ComplexDevice","requirements":"` + strings.Repeat("x", 700) + `"}`)
	rr := doAuthRequest(t, srv, http.MethodPost, "/v1/synthesize", key, body)
	if rr.Code != http.StatusAccepted {
		t.Fatalf("expected 202, got %d body=%s", rr.Code, rr.Body.String())
	}

	var resp struct {
		JobID string `json:"job_id"`
		RunID string `json:"run_id"`
	}
	if err := json.Unmarshal(rr.Body.Bytes(), &resp); err != nil {
		t.Fatalf("failed to decode synth response: %v", err)
	}
	if resp.JobID == "" || resp.RunID == "" {
		t.Fatalf("expected non-empty job_id and run_id")
	}

	// Non-primary reserved runs should not remain queued/running.
	recoverable, err := store.ListRecoverableRuns()
	if err != nil {
		t.Fatalf("ListRecoverableRuns failed: %v", err)
	}
	for _, r := range recoverable {
		if strings.HasPrefix(r.RunID, resp.JobID+"-") && r.RunID != resp.RunID {
			t.Fatalf("found non-primary synth run left recoverable: %s status=%s", r.RunID, r.Status)
		}
	}
}

func TestSynthesize_PayloadTooLargeReturns413(t *testing.T) {
	srv, store, _ := newTestServer(t)
	key := createKey(t, store, "ws-synth-too-large")

	body := []byte(`{"component_name":"BigDevice","requirements":"` + strings.Repeat("x", 300000) + `"}`)
	rr := doAuthRequest(t, srv, http.MethodPost, "/v1/synthesize", key, body)
	if rr.Code != http.StatusRequestEntityTooLarge {
		t.Fatalf("expected 413, got %d body=%s", rr.Code, rr.Body.String())
	}

	var apiErr APIError
	if err := json.Unmarshal(rr.Body.Bytes(), &apiErr); err != nil {
		t.Fatalf("failed to decode APIError: %v", err)
	}
	if apiErr.Code != "PAYLOAD_TOO_LARGE" {
		t.Fatalf("unexpected error code: got=%s", apiErr.Code)
	}

	used, err := store.CountRunsForWorkspace("ws-synth-too-large")
	if err != nil {
		t.Fatalf("CountRunsForWorkspace failed: %v", err)
	}
	if used != 0 {
		t.Fatalf("expected no quota usage for oversized payload, got %d", used)
	}
}

func TestEstimate_PayloadTooLargeReturns413(t *testing.T) {
	srv, store, _ := newTestServer(t)
	key := createKey(t, store, "ws-estimate-too-large")

	body := []byte(`{"component_name":"BigDevice","requirements":"` + strings.Repeat("x", 300000) + `"}`)
	rr := doAuthRequest(t, srv, http.MethodPost, "/v1/estimate", key, body)
	if rr.Code != http.StatusRequestEntityTooLarge {
		t.Fatalf("expected 413, got %d body=%s", rr.Code, rr.Body.String())
	}

	var apiErr APIError
	if err := json.Unmarshal(rr.Body.Bytes(), &apiErr); err != nil {
		t.Fatalf("failed to decode APIError: %v", err)
	}
	if apiErr.Code != "PAYLOAD_TOO_LARGE" {
		t.Fatalf("unexpected error code: got=%s", apiErr.Code)
	}
}

func TestVerifyModel_JSONChipYAMLIsPersistedAsRawYAML(t *testing.T) {
	srv, store, artifactsDir := newTestServer(t)
	key := createKey(t, store, "ws-verify-json")

	body := []byte(`{"peripheral_id":"demo","chip_yaml":"registers: []"}`)
	rr := doAuthRequest(t, srv, http.MethodPost, "/v1/models/verify", key, body)
	if rr.Code != http.StatusAccepted {
		t.Fatalf("expected 202, got %d body=%s", rr.Code, rr.Body.String())
	}

	var resp struct {
		RunID string `json:"run_id"`
	}
	if err := json.Unmarshal(rr.Body.Bytes(), &resp); err != nil {
		t.Fatalf("failed to decode response: %v", err)
	}
	if resp.RunID == "" {
		t.Fatalf("expected run_id in response")
	}

	inputPath := filepath.Join(artifactsDir, resp.RunID, "input.yaml")
	got, err := os.ReadFile(inputPath)
	if err != nil {
		t.Fatalf("failed reading persisted input.yaml: %v", err)
	}
	if string(got) != "registers: []" {
		t.Fatalf("unexpected persisted yaml: got=%q want=%q", string(got), "registers: []")
	}
}

func TestVerifyModel_InvalidJSONReturns400(t *testing.T) {
	srv, store, _ := newTestServer(t)
	key := createKey(t, store, "ws-verify-invalid-json")

	body := []byte(`{"chip_yaml":`)
	rr := doAuthRequest(t, srv, http.MethodPost, "/v1/models/verify", key, body)
	if rr.Code != http.StatusBadRequest {
		t.Fatalf("expected 400, got %d body=%s", rr.Code, rr.Body.String())
	}

	var apiErr APIError
	if err := json.Unmarshal(rr.Body.Bytes(), &apiErr); err != nil {
		t.Fatalf("failed to decode APIError: %v", err)
	}
	if apiErr.Code != "INVALID_JSON" {
		t.Fatalf("unexpected error code: %s", apiErr.Code)
	}
}

func TestVerifySystem_SystemYAMLIsAccepted(t *testing.T) {
	srv, store, artifactsDir := newTestServer(t)
	key := createKey(t, store, "ws-verify-system-json")

	body := []byte(`{"system_yaml":"mcu: demo"}`)
	rr := doAuthRequest(t, srv, http.MethodPost, "/v1/systems/verify", key, body)
	if rr.Code != http.StatusAccepted {
		t.Fatalf("expected 202, got %d body=%s", rr.Code, rr.Body.String())
	}

	var resp struct {
		RunID string `json:"run_id"`
	}
	if err := json.Unmarshal(rr.Body.Bytes(), &resp); err != nil {
		t.Fatalf("failed to decode response: %v", err)
	}
	if resp.RunID == "" {
		t.Fatalf("expected run_id in response")
	}

	inputPath := filepath.Join(artifactsDir, resp.RunID, "input.yaml")
	got, err := os.ReadFile(inputPath)
	if err != nil {
		t.Fatalf("failed reading persisted input.yaml: %v", err)
	}
	if string(got) != "mcu: demo" {
		t.Fatalf("unexpected persisted yaml: got=%q want=%q", string(got), "mcu: demo")
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

func TestCleanupExpiredArtifacts_PrunesOldTerminalMetadata(t *testing.T) {
	srv, store, _ := newTestServer(t)

	if err := store.SaveRun("run-old-meta", "ws-meta", "queued"); err != nil {
		t.Fatalf("SaveRun failed: %v", err)
	}
	if err := store.UpdateRunStatus("run-old-meta", "pass", 1, 1, ""); err != nil {
		t.Fatalf("UpdateRunStatus failed: %v", err)
	}

	// Force immediate metadata eligibility in test (cutoff becomes now + 24h).
	srv.runMetadataRetentionDays = -1
	if err := srv.cleanupExpiredArtifactsOnce(time.Now()); err != nil {
		t.Fatalf("cleanupExpiredArtifactsOnce failed: %v", err)
	}

	record, err := store.GetRunForWorkspace("run-old-meta", "ws-meta")
	if err != nil {
		t.Fatalf("GetRunForWorkspace failed: %v", err)
	}
	if record != nil {
		t.Fatalf("expected old terminal metadata to be pruned")
	}
	if got := srv.metrics.CleanupMetadataRowsDeleted.Load(); got != 1 {
		t.Fatalf("expected cleanup metadata metric to increment to 1, got %d", got)
	}
}

func TestCleanupExpiredArtifacts_PrunesOldIdempotencyRows(t *testing.T) {
	srv, store, _ := newTestServer(t)

	_, _, err := store.BeginIdempotencyRequest("ws-idem-cleanup", "/v1/models/verify", "old-key")
	if err != nil {
		t.Fatalf("BeginIdempotencyRequest failed: %v", err)
	}
	if err := store.CompleteIdempotencyRequest("ws-idem-cleanup", "/v1/models/verify", "old-key", "run-old", 202, `{"run_id":"run-old"}`); err != nil {
		t.Fatalf("CompleteIdempotencyRequest failed: %v", err)
	}

	// Force immediate eligibility in test (cutoff becomes now + 24h).
	srv.runMetadataRetentionDays = -1
	if err := srv.cleanupExpiredArtifactsOnce(time.Now()); err != nil {
		t.Fatalf("cleanupExpiredArtifactsOnce failed: %v", err)
	}

	isNew, existing, err := store.BeginIdempotencyRequest("ws-idem-cleanup", "/v1/models/verify", "old-key")
	if err != nil {
		t.Fatalf("BeginIdempotencyRequest after cleanup failed: %v", err)
	}
	if !isNew || existing == nil || existing.StatusCode != 0 {
		t.Fatalf("expected old idempotency row pruned and key re-usable; isNew=%v status=%d", isNew, existing.StatusCode)
	}
	if got := srv.metrics.IdempotencyRowsPruned.Load(); got != 1 {
		t.Fatalf("expected idempotency prune metric to increment to 1, got %d", got)
	}
}

func TestListHardware_ReturnsJSON(t *testing.T) {
	srv, store, _ := newTestServer(t)
	seed := []db.HardwareItem{
		{ID: "h1", Name: "h1", Type: "board", ReplPath: "test1", Tier: 1},
		{ID: "h2", Name: "h2", Type: "cpu", ReplPath: "test2", Tier: 2},
	}
	if err := store.SeedHardware(seed); err != nil {
		t.Fatalf("SeedHardware failed: %v", err)
	}

	rr := doAuthRequest(t, srv, http.MethodGet, "/v1/hardware", "", nil)
	if rr.Code != http.StatusOK {
		t.Fatalf("expected 200, got %d", rr.Code)
	}

	var items []db.HardwareItem
	if err := json.Unmarshal(rr.Body.Bytes(), &items); err != nil {
		t.Fatalf("failed to decode response: %v", err)
	}

	if len(items) != 2 {
		t.Fatalf("expected 2 items, got %d", len(items))
	}
	if items[0].ID != "h1" || items[1].ID != "h2" {
		t.Errorf("unexpected ordering or content: %+v", items)
	}
}

func TestDataCatalogArtifact_IsServedFromDataDir(t *testing.T) {
	srv, _, _ := newTestServer(t)

	modelData := []byte(`{"device":"demo"}`)
	err := srv.catalog.PromoteToCatalog(
		db.CatalogAsset{
			ID:          "demo-asset",
			Name:        "demo-asset",
			Description: "demo",
			PassRate:    100,
			Registers:   1,
		},
		modelData,
		srv.dataDir,
	)
	if err != nil {
		t.Fatalf("PromoteToCatalog failed: %v", err)
	}

	rrCatalog := httptest.NewRecorder()
	reqCatalog := httptest.NewRequest(http.MethodGet, "/v1/catalog/demo-asset", nil)
	srv.ServeHTTP(rrCatalog, reqCatalog)
	if rrCatalog.Code != http.StatusOK {
		t.Fatalf("expected 200 catalog response, got %d body=%s", rrCatalog.Code, rrCatalog.Body.String())
	}

	var asset struct {
		IrURL string `json:"ir_url"`
	}
	if err := json.Unmarshal(rrCatalog.Body.Bytes(), &asset); err != nil {
		t.Fatalf("failed to decode catalog asset: %v", err)
	}
	if asset.IrURL == "" {
		t.Fatalf("expected ir_url in catalog response")
	}

	rrFile := httptest.NewRecorder()
	reqFile := httptest.NewRequest(http.MethodGet, asset.IrURL, nil)
	srv.ServeHTTP(rrFile, reqFile)
	if rrFile.Code != http.StatusOK {
		t.Fatalf("expected 200 data artifact response, got %d body=%s", rrFile.Code, rrFile.Body.String())
	}
	if rrFile.Body.String() != string(modelData) {
		t.Fatalf("unexpected artifact body: got=%q want=%q", rrFile.Body.String(), string(modelData))
	}
}

func TestHealth_ExposesRuntimeMetrics(t *testing.T) {
	srv, _, artifactsDir := newTestServer(t)
	if err := os.MkdirAll(artifactsDir, 0o755); err != nil {
		t.Fatalf("MkdirAll artifacts dir failed: %v", err)
	}

	srv.metrics.InflightLimitRejected.Add(2)
	srv.metrics.QueueFullRejected.Add(3)
	srv.metrics.ShuttingDownRejected.Add(4)
	srv.metrics.CleanupArtifactDeleted.Add(5)
	srv.metrics.CleanupArtifactSkippedUnsafe.Add(6)
	srv.metrics.CleanupArtifactDeleteFailed.Add(7)
	srv.metrics.CleanupDBPathClearFailed.Add(8)
	srv.metrics.CleanupMetadataRowsDeleted.Add(9)
	srv.metrics.StripeDuplicateEvents.Add(10)
	srv.metrics.IdempotencyRowsPruned.Add(11)

	rr := doAuthRequest(t, srv, http.MethodGet, "/v1/health", "", nil)
	if rr.Code != http.StatusOK {
		t.Fatalf("expected 200, got %d body=%s", rr.Code, rr.Body.String())
	}

	var resp struct {
		Components map[string]json.RawMessage `json:"components"`
	}
	if err := json.Unmarshal(rr.Body.Bytes(), &resp); err != nil {
		t.Fatalf("failed to decode health response: %v", err)
	}
	rawMetrics, ok := resp.Components["metrics"]
	if !ok {
		t.Fatalf("metrics component missing in health response")
	}

	var metrics map[string]int64
	if err := json.Unmarshal(rawMetrics, &metrics); err != nil {
		t.Fatalf("failed to decode metrics component: %v", err)
	}
	if metrics["inflight_limit_rejected"] != 2 {
		t.Fatalf("unexpected inflight_limit_rejected metric: %d", metrics["inflight_limit_rejected"])
	}
	if metrics["cleanup_metadata_rows_deleted"] != 9 {
		t.Fatalf("unexpected cleanup_metadata_rows_deleted metric: %d", metrics["cleanup_metadata_rows_deleted"])
	}
	if metrics["stripe_duplicate_events"] != 10 {
		t.Fatalf("unexpected stripe_duplicate_events metric: %d", metrics["stripe_duplicate_events"])
	}
	if metrics["idempotency_rows_pruned"] != 11 {
		t.Fatalf("unexpected idempotency_rows_pruned metric: %d", metrics["idempotency_rows_pruned"])
	}
}

func TestServeHTTP_PanicRecoveryReturns500(t *testing.T) {
	srv, _, _ := newTestServer(t)
	srv.router.HandleFunc("/panic-test", func(w http.ResponseWriter, r *http.Request) {
		panic("boom")
	}).Methods(http.MethodGet)

	rr := httptest.NewRecorder()
	req := httptest.NewRequest(http.MethodGet, "/panic-test", nil)
	srv.ServeHTTP(rr, req)
	if rr.Code != http.StatusInternalServerError {
		t.Fatalf("expected 500, got %d body=%s", rr.Code, rr.Body.String())
	}

	var apiErr APIError
	if err := json.Unmarshal(rr.Body.Bytes(), &apiErr); err != nil {
		t.Fatalf("failed to decode APIError: %v", err)
	}
	if apiErr.Code != "INTERNAL_ERROR" {
		t.Fatalf("unexpected error code: %s", apiErr.Code)
	}
}

func newSchedulerOnlyServer(maxPending int) *Server {
	s := &Server{
		pendingByWorkspace: make(map[string][]*Job),
		maxPendingJobs:     maxPending,
	}
	s.scheduleCond = sync.NewCond(&s.scheduleMu)
	return s
}

func newRefillTestServer(store *db.Store, artifactsDir string) *Server {
	s := &Server{
		store:                   store,
		artifactsDir:            artifactsDir,
		pendingByWorkspace:      make(map[string][]*Job),
		maxPendingJobs:          100,
		maxInflightPerWorkspace: 8,
		inflightByWorkspace:     make(map[string]int),
	}
	s.scheduleCond = sync.NewCond(&s.scheduleMu)
	return s
}

func TestScheduler_RoundRobinAcrossWorkspaces(t *testing.T) {
	s := newSchedulerOnlyServer(10)

	if res := s.tryEnqueueJob(&Job{ID: "a1", WorkspaceID: "ws-a"}); res != enqueueOK {
		t.Fatalf("enqueue a1 failed: %v", res)
	}
	if res := s.tryEnqueueJob(&Job{ID: "a2", WorkspaceID: "ws-a"}); res != enqueueOK {
		t.Fatalf("enqueue a2 failed: %v", res)
	}
	if res := s.tryEnqueueJob(&Job{ID: "b1", WorkspaceID: "ws-b"}); res != enqueueOK {
		t.Fatalf("enqueue b1 failed: %v", res)
	}
	if res := s.tryEnqueueJob(&Job{ID: "b2", WorkspaceID: "ws-b"}); res != enqueueOK {
		t.Fatalf("enqueue b2 failed: %v", res)
	}

	order := []string{}
	for i := 0; i < 4; i++ {
		j, ok := s.dequeueJob()
		if !ok || j == nil {
			t.Fatalf("dequeue %d failed", i)
		}
		order = append(order, j.ID)
	}

	got := strings.Join(order, ",")
	want := "a1,b1,a2,b2"
	if got != want {
		t.Fatalf("unexpected round-robin order: got=%s want=%s", got, want)
	}
}

func TestScheduler_BoundedAdmission(t *testing.T) {
	s := newSchedulerOnlyServer(2)
	if res := s.tryEnqueueJob(&Job{ID: "a1", WorkspaceID: "ws-a"}); res != enqueueOK {
		t.Fatalf("enqueue a1 failed: %v", res)
	}
	if res := s.tryEnqueueJob(&Job{ID: "b1", WorkspaceID: "ws-b"}); res != enqueueOK {
		t.Fatalf("enqueue b1 failed: %v", res)
	}
	if res := s.tryEnqueueJob(&Job{ID: "c1", WorkspaceID: "ws-c"}); res != enqueueQueueFull {
		t.Fatalf("expected enqueueQueueFull, got %v", res)
	}
}

func TestRefillQueueFromDB_EnqueuesQueuedVerifyRun(t *testing.T) {
	root := t.TempDir()
	store, err := db.NewStore(filepath.Join(root, "test.db"))
	if err != nil {
		t.Fatalf("NewStore failed: %v", err)
	}
	t.Cleanup(func() { _ = store.Close() })

	runID := "run-model-refill-1"
	artifactDir := filepath.Join(root, "artifacts", runID)
	if err := os.MkdirAll(artifactDir, 0o755); err != nil {
		t.Fatalf("MkdirAll failed: %v", err)
	}
	if err := os.WriteFile(filepath.Join(artifactDir, "input.yaml"), []byte("registers: []"), 0o644); err != nil {
		t.Fatalf("WriteFile failed: %v", err)
	}
	if err := store.SaveRun(runID, "ws-refill", "queued"); err != nil {
		t.Fatalf("SaveRun failed: %v", err)
	}

	s := newRefillTestServer(store, filepath.Join(root, "artifacts"))
	s.refillQueueFromDB(10)

	if _, ok := s.jobs.Load(runID); !ok {
		t.Fatalf("expected run to be loaded into in-memory jobs map")
	}
	s.scheduleMu.Lock()
	defer s.scheduleMu.Unlock()
	if s.pendingJobs != 1 {
		t.Fatalf("expected one pending job, got %d", s.pendingJobs)
	}
}

func TestRefillQueueFromDB_MarksUnrecoverableRunError(t *testing.T) {
	root := t.TempDir()
	store, err := db.NewStore(filepath.Join(root, "test.db"))
	if err != nil {
		t.Fatalf("NewStore failed: %v", err)
	}
	t.Cleanup(func() { _ = store.Close() })

	runID := "run-model-bad-1"
	if err := store.SaveRun(runID, "ws-refill", "queued"); err != nil {
		t.Fatalf("SaveRun failed: %v", err)
	}

	s := newRefillTestServer(store, filepath.Join(root, "artifacts"))
	s.refillQueueFromDB(10)

	record, err := store.GetRunForWorkspace(runID, "ws-refill")
	if err != nil {
		t.Fatalf("GetRunForWorkspace failed: %v", err)
	}
	if record == nil {
		t.Fatalf("expected run record to exist")
	}
	if record.Status != "error" {
		t.Fatalf("expected unrecoverable queued run to transition to error, got %s", record.Status)
	}
}

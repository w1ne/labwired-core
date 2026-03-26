package api

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/labwired/foundry-backend/internal/catalog"
	"github.com/labwired/foundry-backend/internal/db"
	"github.com/labwired/foundry-backend/internal/synthesis"
	"github.com/labwired/foundry-backend/internal/verification"
)

func initGitRepo(t *testing.T, root string) {
	t.Helper()
	run := func(args ...string) {
		t.Helper()
		cmd := exec.Command("git", args...)
		cmd.Dir = root
		cmd.Env = append(os.Environ(),
			"GIT_AUTHOR_NAME=Test Bot",
			"GIT_AUTHOR_EMAIL=test@example.com",
			"GIT_COMMITTER_NAME=Test Bot",
			"GIT_COMMITTER_EMAIL=test@example.com",
		)
		output, err := cmd.CombinedOutput()
		if err != nil {
			t.Fatalf("git %s failed: %v (%s)", strings.Join(args, " "), err, string(output))
		}
	}
	run("init", "-b", "main")
	if err := os.WriteFile(filepath.Join(root, "README.md"), []byte("seed\n"), 0o644); err != nil {
		t.Fatalf("WriteFile seed failed: %v", err)
	}
	run("add", "README.md")
	run("commit", "-m", "seed")
}

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

func waitForRunCompletion(t *testing.T, srv *Server, key string, runID string) map[string]any {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		rr := doAuthRequest(t, srv, http.MethodGet, "/v1/runs/"+runID, key, nil)
		if rr.Code != http.StatusOK {
			t.Fatalf("poll run failed: status=%d body=%s", rr.Code, rr.Body.String())
		}
		var payload map[string]any
		if err := json.Unmarshal(rr.Body.Bytes(), &payload); err != nil {
			t.Fatalf("poll decode failed: %v", err)
		}
		status, _ := payload["status"].(string)
		if status == "pass" || status == "fail" || status == "error" {
			return payload
		}
		time.Sleep(25 * time.Millisecond)
	}
	t.Fatalf("timed out waiting for run %s", runID)
	return nil
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

func TestEstimate_BoardOnboardingPayloadReturnsNormalizedContract(t *testing.T) {
	srv, store, _ := newTestServer(t)
	key := createKey(t, store, "ws-estimate-board")

	body := []byte(`{
	  "kind":"board_onboarding",
	  "datasheet_url":"https://www.st.com/resource/en/datasheet/stm32wb55rg.pdf",
	  "documentation_urls":[
	    "https://www.st.com/resource/en/user_manual/um2819-stm32wb-nucleo64-board-mb1355-stmicroelectronics.pdf",
	    "https://www.st.com/resource/en/reference_manual/rm0434-stm32wb55xx-stm32wb35xx-advanced-armbased-32bit-mcus-stmicroelectronics.pdf",
	    "https://github.com/STMicroelectronics/STM32CubeWB/tree/master/Projects/P-NUCLEO-WB55.Nucleo/Applications/BLE/BLE_p2pServer"
	  ],
	  "board":{"vendor":"ST","marketing_name":"NUCLEO-WB55RG","board_id":"MB1355C","mcu":"STM32WB55RG"},
	  "desired_capabilities":["boot","uart_console","led_control","button_input"],
	  "validation_targets":["uart_smoke","unsupported_instruction_audit"],
	  "workload":{"type":"generated_smoke_firmware","example":"BLE_p2pServer"},
	  "constraints":{"ble_scope":"example_selection_only","must_write_repo_assets":true,"must_run_e2e_validation":true}
	}`)

	rr := doAuthRequest(t, srv, http.MethodPost, "/v1/estimate", key, body)
	if rr.Code != http.StatusOK {
		t.Fatalf("expected 200, got %d body=%s", rr.Code, rr.Body.String())
	}
	var got map[string]any
	if err := json.Unmarshal(rr.Body.Bytes(), &got); err != nil {
		t.Fatalf("json.Unmarshal failed: %v", err)
	}
	if got["kind"] != "board_onboarding" {
		t.Fatalf("expected kind=board_onboarding, got=%v", got["kind"])
	}
	componentName, _ := got["component_name"].(string)
	if !strings.Contains(componentName, "MB1355C / NUCLEO-WB55RG") {
		t.Fatalf("unexpected component_name: %q", componentName)
	}
}

func TestSynthesize_BoardOnboardingPayloadRequiresDesiredCapabilities(t *testing.T) {
	srv, store, _ := newTestServer(t)
	key := createKey(t, store, "ws-synth-board-missing-caps")

	body := []byte(`{
	  "kind":"board_onboarding",
	  "datasheet_url":"https://www.st.com/resource/en/datasheet/stm32wb55rg.pdf",
	  "documentation_urls":[
	    "https://www.st.com/resource/en/user_manual/um2819-stm32wb-nucleo64-board-mb1355-stmicroelectronics.pdf",
	    "https://www.st.com/resource/en/reference_manual/rm0434-stm32wb55xx-stm32wb35xx-advanced-armbased-32bit-mcus-stmicroelectronics.pdf"
	  ],
	  "board":{"marketing_name":"NUCLEO-WB55RG","board_id":"MB1355C"}
	}`)

	rr := doAuthRequest(t, srv, http.MethodPost, "/v1/synthesize", key, body)
	if rr.Code != http.StatusBadRequest {
		t.Fatalf("expected 400, got %d body=%s", rr.Code, rr.Body.String())
	}
	if !strings.Contains(rr.Body.String(), "desired_capabilities") {
		t.Fatalf("expected desired_capabilities error, got %s", rr.Body.String())
	}
}

func TestEstimate_BoardOnboardingDryRunDefaultsToArtifactOnly(t *testing.T) {
	srv, store, _ := newTestServer(t)
	key := createKey(t, store, "ws-estimate-board-dry-run")

	body := []byte(`{
	  "kind":"board_onboarding",
	  "dry_run":true,
	  "datasheet_url":"https://www.st.com/resource/en/datasheet/stm32wb55rg.pdf",
	  "documentation_urls":[
	    "https://www.st.com/resource/en/user_manual/um2819-stm32wb-nucleo64-board-mb1355-stmicroelectronics.pdf",
	    "https://www.st.com/resource/en/reference_manual/rm0434-stm32wb55xx-stm32wb35xx-advanced-armbased-32bit-mcus-stmicroelectronics.pdf"
	  ],
	  "board":{"marketing_name":"NUCLEO-WB55RG","board_id":"MB1355C","mcu":"STM32WB55RG"},
	  "desired_capabilities":["boot","uart_console"]
	}`)

	rr := doAuthRequest(t, srv, http.MethodPost, "/v1/estimate", key, body)
	if rr.Code != http.StatusOK {
		t.Fatalf("expected 200, got %d body=%s", rr.Code, rr.Body.String())
	}
	var got map[string]any
	if err := json.Unmarshal(rr.Body.Bytes(), &got); err != nil {
		t.Fatalf("json.Unmarshal failed: %v", err)
	}
	if got["dry_run"] != true {
		t.Fatalf("expected dry_run=true, got=%v", got["dry_run"])
	}
	if got["promotion_mode"] != "artifact_only" {
		t.Fatalf("expected promotion_mode=artifact_only, got=%v", got["promotion_mode"])
	}
}

func TestEstimate_PeripheralModelIngestPayloadPreservesComponentIdentity(t *testing.T) {
	srv, store, _ := newTestServer(t)
	key := createKey(t, store, "ws-estimate-peripheral")

	body := []byte(`{
	  "kind":"peripheral_model_ingest",
	  "component_name":"ADXL345",
	  "requirements":"I2C interface required. Register 0x00 should return Device ID 0xE5.",
	  "datasheet_url":"https://www.analog.com/media/en/technical-documentation/data-sheets/ADXL345.pdf"
	}`)

	rr := doAuthRequest(t, srv, http.MethodPost, "/v1/estimate", key, body)
	if rr.Code != http.StatusOK {
		t.Fatalf("expected 200, got %d body=%s", rr.Code, rr.Body.String())
	}
	var got map[string]any
	if err := json.Unmarshal(rr.Body.Bytes(), &got); err != nil {
		t.Fatalf("json.Unmarshal failed: %v", err)
	}
	if got["kind"] != "peripheral_model_ingest" {
		t.Fatalf("expected kind=peripheral_model_ingest, got=%v", got["kind"])
	}
	if got["component_name"] != "ADXL345" {
		t.Fatalf("unexpected component_name: %v", got["component_name"])
	}
}

func TestEstimate_BoardOnboardingMissingDocsRejected(t *testing.T) {
	srv, store, _ := newTestServer(t)
	key := createKey(t, store, "ws-estimate-board-missing-docs")

	body := []byte(`{
	  "kind":"board_onboarding",
	  "board":{"marketing_name":"NUCLEO-WB55RG","board_id":"MB1355C","mcu":"STM32WB55RG"},
	  "desired_capabilities":["boot","uart_console"]
	}`)

	rr := doAuthRequest(t, srv, http.MethodPost, "/v1/estimate", key, body)
	if rr.Code != http.StatusBadRequest {
		t.Fatalf("expected 400, got %d body=%s", rr.Code, rr.Body.String())
	}
	if !strings.Contains(rr.Body.String(), "insufficient docs") || !strings.Contains(rr.Body.String(), "datasheet_url") {
		t.Fatalf("expected missing-doc details, got %s", rr.Body.String())
	}
}

func TestEstimate_PeripheralModelIngestMissingDatasheetRejected(t *testing.T) {
	srv, store, _ := newTestServer(t)
	key := createKey(t, store, "ws-estimate-peripheral-missing-datasheet")

	body := []byte(`{
	  "kind":"peripheral_model_ingest",
	  "component_name":"ADXL345",
	  "requirements":"I2C interface required. Register 0x00 should return Device ID 0xE5."
	}`)

	rr := doAuthRequest(t, srv, http.MethodPost, "/v1/estimate", key, body)
	if rr.Code != http.StatusBadRequest {
		t.Fatalf("expected 400, got %d body=%s", rr.Code, rr.Body.String())
	}
	if !strings.Contains(rr.Body.String(), "datasheet_url is required") {
		t.Fatalf("expected datasheet requirement error, got %s", rr.Body.String())
	}
}

func TestSynthesize_BoardOnboardingDryRun_EndToEndArtifacts(t *testing.T) {
	srv, store, _ := newTestServer(t)
	const workspaceID = "ws-synth-board-e2e"
	key := createKey(t, store, workspaceID)
	if err := store.AddQuotaRuns(workspaceID, 5000); err != nil {
		t.Fatalf("AddQuotaRuns failed: %v", err)
	}

	fakeLabwiredDir := t.TempDir()
	fakeLabwired := filepath.Join(fakeLabwiredDir, "labwired")
	if err := os.WriteFile(fakeLabwired, []byte("#!/bin/sh\nprintf '{\"valid\":true,\"statistics\":{\"total_checks\":3}}'\n"), 0o755); err != nil {
		t.Fatalf("failed to write fake labwired: %v", err)
	}
	srv.orchestrator = verification.NewOrchestrator(fakeLabwired)

	body := []byte(`{
	  "kind":"board_onboarding",
	  "dry_run":true,
	  "datasheet_url":"https://www.st.com/resource/en/datasheet/stm32wb55rg.pdf",
	  "documentation_urls":[
	    "https://www.st.com/resource/en/user_manual/um2819-stm32wb-nucleo64-board-mb1355-stmicroelectronics.pdf",
	    "https://www.st.com/resource/en/reference_manual/rm0434-stm32wb55xx-stm32wb35xx-advanced-armbased-32bit-mcus-stmicroelectronics.pdf"
	  ],
	  "board":{"vendor":"ST","marketing_name":"NUCLEO-WB55RG","board_id":"MB1355C","mcu":"STM32WB55RG"},
	  "desired_capabilities":["boot","uart_console","led_control","button_input"],
	  "validation_targets":["uart_smoke","unsupported_instruction_audit"]
	}`)

	rr := doAuthRequest(t, srv, http.MethodPost, "/v1/synthesize", key, body)
	if rr.Code != http.StatusAccepted {
		t.Fatalf("expected 202, got %d body=%s", rr.Code, rr.Body.String())
	}
	var accepted map[string]any
	if err := json.Unmarshal(rr.Body.Bytes(), &accepted); err != nil {
		t.Fatalf("decode accepted response failed: %v", err)
	}
	runID, _ := accepted["run_id"].(string)
	if runID == "" {
		t.Fatalf("missing run_id in response: %s", rr.Body.String())
	}

	final := waitForRunCompletion(t, srv, key, runID)
	if final["status"] != "pass" {
		t.Fatalf("expected pass final status, got=%v payload=%v", final["status"], final)
	}

	out := doAuthRequest(t, srv, http.MethodGet, "/v1/runs/"+runID+"/artifacts/output.json", key, nil)
	if out.Code != http.StatusOK {
		t.Fatalf("expected output artifact 200, got %d body=%s", out.Code, out.Body.String())
	}
	var artifact map[string]any
	if err := json.Unmarshal(out.Body.Bytes(), &artifact); err != nil {
		t.Fatalf("decode output artifact failed: %v", err)
	}
	contract, ok := artifact["contract_result"].(map[string]any)
	if !ok {
		t.Fatalf("expected contract_result object, got=%T", artifact["contract_result"])
	}
	if contract["request_kind"] != "board_onboarding" {
		t.Fatalf("unexpected request_kind: %v", contract["request_kind"])
	}
	if contract["promotion_mode"] != "artifact_only" {
		t.Fatalf("unexpected promotion_mode: %v", contract["promotion_mode"])
	}

	result := doAuthRequest(t, srv, http.MethodGet, "/v1/runs/"+runID+"/artifacts/result.json", key, nil)
	if result.Code != http.StatusOK {
		t.Fatalf("expected result artifact 200, got %d body=%s", result.Code, result.Body.String())
	}
	var resultPayload map[string]any
	if err := json.Unmarshal(result.Body.Bytes(), &resultPayload); err != nil {
		t.Fatalf("decode result artifact failed: %v", err)
	}
	if resultPayload["pass"] != true {
		t.Fatalf("expected result pass=true, got %v", resultPayload["pass"])
	}
}

func TestSynthesize_PeripheralModelIngest_EndToEndArtifacts(t *testing.T) {
	srv, store, _ := newTestServer(t)
	key := createKey(t, store, "ws-synth-peripheral-e2e")

	body := []byte(`{
	  "kind":"peripheral_model_ingest",
	  "component_name":"ADXL345",
	  "requirements":"I2C interface required. Register 0x00 should return Device ID 0xE5.",
	  "datasheet_url":"https://www.analog.com/media/en/technical-documentation/data-sheets/ADXL345.pdf"
	}`)

	rr := doAuthRequest(t, srv, http.MethodPost, "/v1/synthesize", key, body)
	if rr.Code != http.StatusAccepted {
		t.Fatalf("expected 202, got %d body=%s", rr.Code, rr.Body.String())
	}
	var accepted map[string]any
	if err := json.Unmarshal(rr.Body.Bytes(), &accepted); err != nil {
		t.Fatalf("decode accepted response failed: %v", err)
	}
	runID, _ := accepted["run_id"].(string)
	if runID == "" {
		t.Fatalf("missing run_id in response: %s", rr.Body.String())
	}

	final := waitForRunCompletion(t, srv, key, runID)
	if final["status"] != "pass" {
		t.Fatalf("expected pass final status, got=%v payload=%v", final["status"], final)
	}

	out := doAuthRequest(t, srv, http.MethodGet, "/v1/runs/"+runID+"/artifacts/output.json", key, nil)
	if out.Code != http.StatusOK {
		t.Fatalf("expected output artifact 200, got %d body=%s", out.Code, out.Body.String())
	}
	var artifact map[string]any
	if err := json.Unmarshal(out.Body.Bytes(), &artifact); err != nil {
		t.Fatalf("decode output artifact failed: %v", err)
	}
	if artifact["artifact_type"] != "strict_ir_draft" {
		t.Fatalf("unexpected artifact_type: %v", artifact["artifact_type"])
	}
	contract, ok := artifact["contract_result"].(map[string]any)
	if !ok {
		t.Fatalf("expected contract_result object, got=%T", artifact["contract_result"])
	}
	if contract["request_kind"] != "peripheral_model_ingest" {
		t.Fatalf("unexpected request_kind: %v", contract["request_kind"])
	}
}

func TestSynthesize_UnknownBoardDryRunFailsWithoutGroundedFacts(t *testing.T) {
	srv, store, _ := newTestServer(t)
	const workspaceID = "ws-synth-unknown-board-e2e"
	key := createKey(t, store, workspaceID)
	if err := store.AddQuotaRuns(workspaceID, 5000); err != nil {
		t.Fatalf("AddQuotaRuns failed: %v", err)
	}

	fakeLabwiredDir := t.TempDir()
	fakeLabwired := filepath.Join(fakeLabwiredDir, "labwired")
	if err := os.WriteFile(fakeLabwired, []byte("#!/bin/sh\nprintf '{\"valid\":true,\"statistics\":{\"total_checks\":3}}'\n"), 0o755); err != nil {
		t.Fatalf("failed to write fake labwired: %v", err)
	}
	srv.orchestrator = verification.NewOrchestrator(fakeLabwired)

	body := []byte(`{
	  "kind":"board_onboarding",
	  "dry_run":true,
	  "datasheet_url":"https://example.com/protospark-x9-mcu.pdf",
	  "documentation_urls":[
	    "https://example.com/protospark-x9-board.pdf",
	    "https://example.com/protospark-x9-reference-manual.pdf"
	  ],
	  "board":{"vendor":"Acme","marketing_name":"ProtoSpark X9","board_id":"PSX9-REV-A","mcu":"XMegaFoo123"},
	  "desired_capabilities":["boot","uart_console","led_control"],
	  "validation_targets":["uart_smoke"]
	}`)

	rr := doAuthRequest(t, srv, http.MethodPost, "/v1/synthesize", key, body)
	if rr.Code != http.StatusAccepted {
		t.Fatalf("expected 202, got %d body=%s", rr.Code, rr.Body.String())
	}
	var accepted map[string]any
	if err := json.Unmarshal(rr.Body.Bytes(), &accepted); err != nil {
		t.Fatalf("decode accepted response failed: %v", err)
	}
	runID, _ := accepted["run_id"].(string)
	if runID == "" {
		t.Fatalf("missing run_id in response: %s", rr.Body.String())
	}

	final := waitForRunCompletion(t, srv, key, runID)
	if final["status"] != "error" {
		t.Fatalf("expected error final status, got=%v payload=%v", final["status"], final)
	}

	if _, ok := final["artifacts"]; ok {
		t.Fatalf("expected no artifacts for early grounded-facts failure, got=%v", final["artifacts"])
	}
}

func TestSynthesize_LocalDocExtraction_EndToEndArtifacts(t *testing.T) {
	srv, store, _ := newTestServer(t)
	const workspaceID = "ws-synth-doc-extract-e2e"
	key := createKey(t, store, workspaceID)
	if err := store.AddQuotaRuns(workspaceID, 5000); err != nil {
		t.Fatalf("AddQuotaRuns failed: %v", err)
	}

	fakeLabwiredDir := t.TempDir()
	fakeLabwired := filepath.Join(fakeLabwiredDir, "labwired")
	if err := os.WriteFile(fakeLabwired, []byte("#!/bin/sh\nprintf '{\"valid\":true,\"statistics\":{\"total_checks\":3}}'\n"), 0o755); err != nil {
		t.Fatalf("failed to write fake labwired: %v", err)
	}
	srv.orchestrator = verification.NewOrchestrator(fakeLabwired)

	docDir := t.TempDir()
	datasheetPath := filepath.Join(docDir, "sparkfun-x1-datasheet.pdf")
	boardDocPath := filepath.Join(docDir, "sparkfun-x1-board.pdf")
	schematicPath := filepath.Join(docDir, "sparkfun-x1-schematic.pdf")
	referencePath := filepath.Join(docDir, "sparkfun-x1-reference-manual.pdf")
	if err := os.WriteFile(datasheetPath, []byte("MCU STM32F411RE\nFLASH 512KB\nRAM 128KB\nRCC 0x40023800\nGPIOA 0x40020000\nGPIOB 0x40020400\nGPIOC 0x40020800\nUSART2 0x40004400 IRQ 38\nTX GPIOA 2\nRX GPIOA 3\n"), 0o644); err != nil {
		t.Fatalf("WriteFile datasheet failed: %v", err)
	}
	if err := os.WriteFile(boardDocPath, []byte("led_status GPIOC 13 active_high\nbutton_user GPIOA 0 active_low\n"), 0o644); err != nil {
		t.Fatalf("WriteFile board doc failed: %v", err)
	}
	if err := os.WriteFile(schematicPath, []byte("board SparkFun X1 RevA\n"), 0o644); err != nil {
		t.Fatalf("WriteFile schematic failed: %v", err)
	}
	if err := os.WriteFile(referencePath, []byte("reference manual STM32F411RE\n"), 0o644); err != nil {
		t.Fatalf("WriteFile reference failed: %v", err)
	}

	body, err := json.Marshal(map[string]any{
		"kind":          "board_onboarding",
		"dry_run":       true,
		"datasheet_url": datasheetPath,
		"documentation_urls": []string{
			boardDocPath,
			schematicPath,
			referencePath,
		},
		"board": map[string]any{
			"vendor":         "SparkFun",
			"marketing_name": "X1",
			"board_id":       "sparkfun-x1-reva",
			"mcu":            "STM32F411RE",
		},
		"desired_capabilities": []string{"boot", "uart_console", "led_control", "button_input"},
		"validation_targets":   []string{"uart_smoke", "io_smoke"},
	})
	if err != nil {
		t.Fatalf("json.Marshal failed: %v", err)
	}

	rr := doAuthRequest(t, srv, http.MethodPost, "/v1/synthesize", key, body)
	if rr.Code != http.StatusAccepted {
		t.Fatalf("expected 202, got %d body=%s", rr.Code, rr.Body.String())
	}
	var accepted map[string]any
	if err := json.Unmarshal(rr.Body.Bytes(), &accepted); err != nil {
		t.Fatalf("decode accepted response failed: %v", err)
	}
	runID, _ := accepted["run_id"].(string)
	if runID == "" {
		t.Fatalf("missing run_id in response: %s", rr.Body.String())
	}

	final := waitForRunCompletion(t, srv, key, runID)
	if final["status"] != "pass" {
		t.Fatalf("expected pass final status, got=%v payload=%v", final["status"], final)
	}

	out := doAuthRequest(t, srv, http.MethodGet, "/v1/runs/"+runID+"/artifacts/output.json", key, nil)
	if out.Code != http.StatusOK {
		t.Fatalf("expected output artifact 200, got %d body=%s", out.Code, out.Body.String())
	}
	var artifact map[string]any
	if err := json.Unmarshal(out.Body.Bytes(), &artifact); err != nil {
		t.Fatalf("decode output artifact failed: %v", err)
	}
	boardDraft, ok := artifact["board_draft"].(map[string]any)
	if !ok {
		t.Fatalf("expected board_draft object, got=%T", artifact["board_draft"])
	}
	if boardDraft["chip_guess"] != "stm32f411re" {
		t.Fatalf("expected chip_guess from local docs, got=%v", boardDraft["chip_guess"])
	}
	repoBundle, ok := artifact["repo_bundle"].(map[string]any)
	if !ok {
		t.Fatalf("expected repo_bundle object, got=%T", artifact["repo_bundle"])
	}
	files, ok := repoBundle["files"].([]any)
	if !ok || len(files) == 0 {
		t.Fatalf("expected repo bundle files, got=%v", repoBundle["files"])
	}
	foundChip := false
	foundSystem := false
	for _, entry := range files {
		fileObj, ok := entry.(map[string]any)
		if !ok {
			continue
		}
		path, _ := fileObj["path"].(string)
		content, _ := fileObj["content"].(string)
		if path == "core/configs/chips/stm32f411re.yaml" {
			foundChip = true
			if !strings.Contains(content, "size: \"512KB\"") || !strings.Contains(content, "size: \"128KB\"") || !strings.Contains(content, "0x40004400") || !strings.Contains(content, "irq: 38") {
				t.Fatalf("expected extracted chip facts in chip yaml, got=%s", content)
			}
		}
		if path == "core/configs/systems/sparkfun_x1_reva.yaml" {
			foundSystem = true
			if !strings.Contains(content, "led_status") || !strings.Contains(content, "button_user") {
				t.Fatalf("expected extracted GPIO mappings in system yaml, got=%s", content)
			}
		}
	}
	if !foundChip || !foundSystem {
		t.Fatalf("expected extracted chip/system files, foundChip=%t foundSystem=%t", foundChip, foundSystem)
	}
}

func TestRunSynthesisJob_BoardRequestWritesStructuredDraft(t *testing.T) {
	t.Setenv("XAI_API_KEY", "")

	dir := t.TempDir()
	repoRoot := t.TempDir()
	fakeLabwired := filepath.Join(dir, "labwired")
	script := "#!/bin/sh\nprintf '{\"valid\":true,\"statistics\":{\"total_checks\":3}}'\n"
	if err := os.WriteFile(fakeLabwired, []byte(script), 0o755); err != nil {
		t.Fatalf("failed to write fake labwired: %v", err)
	}
	result, err := runSynthesisJob(context.Background(), &Job{
		ID:                  "synth-board-test",
		Type:                JobTypeSynthesize,
		ArtifactDir:         dir,
		LabWiredPath:        fakeLabwired,
		RepoRootDir:         repoRoot,
		SynthesisKind:       "board_onboarding",
		PromotionMode:       "apply_to_repo",
		ComponentName:       "MB1355C / NUCLEO-WB55RG board onboarding proof",
		Requirements:        "Need deterministic board onboarding proof for STM32WB55RG app core, LED mapping, user button mapping, UART debug path, and BLE-oriented example selection.",
		Board:               &synthesis.BoardSpec{MarketingName: "NUCLEO-WB55RG", BoardID: "MB1355C", MCU: "STM32WB55RG"},
		DesiredCapabilities: []string{"boot", "uart_console", "led_control", "button_input"},
		ValidationTargets:   []string{"uart_smoke", "unsupported_instruction_audit"},
	})
	if err != nil {
		t.Fatalf("runSynthesisJob failed: %v", err)
	}
	if !result.Pass {
		t.Fatalf("expected synthesis pass, got fail: %+v", result)
	}

	data, err := os.ReadFile(filepath.Join(dir, "output.json"))
	if err != nil {
		t.Fatalf("ReadFile output.json failed: %v", err)
	}
	var artifact map[string]any
	if err := json.Unmarshal(data, &artifact); err != nil {
		t.Fatalf("json.Unmarshal failed: %v", err)
	}
	if got := artifact["artifact_type"]; got != "board_onboarding_draft" {
		t.Fatalf("unexpected artifact_type: got=%v", got)
	}
	contractResult, ok := artifact["contract_result"].(map[string]any)
	if !ok {
		t.Fatalf("expected contract_result object, got=%T", artifact["contract_result"])
	}
	if contractResult["request_kind"] != "board_onboarding" {
		t.Fatalf("unexpected contract_result.request_kind: %v", contractResult["request_kind"])
	}
	boardDraft, ok := artifact["board_draft"].(map[string]any)
	if !ok {
		t.Fatalf("expected board_draft object, got=%T", artifact["board_draft"])
	}
	requestedCapabilities, ok := boardDraft["requested_capabilities"].([]any)
	if !ok || len(requestedCapabilities) == 0 {
		t.Fatalf("expected requested_capabilities in board draft, got=%v", boardDraft["requested_capabilities"])
	}
	repoArtifacts, ok := boardDraft["repo_artifacts"].([]any)
	if !ok || len(repoArtifacts) < 3 {
		t.Fatalf("expected repo_artifacts in board draft, got=%v", boardDraft["repo_artifacts"])
	}
	repoBundle, ok := artifact["repo_bundle"].(map[string]any)
	if !ok {
		t.Fatalf("expected repo_bundle object, got=%T", artifact["repo_bundle"])
	}
	files, ok := repoBundle["files"].([]any)
	if !ok || len(files) < 7 {
		t.Fatalf("expected generated file bundle, got=%v", repoBundle["files"])
	}
	contents := map[string]string{}
	for _, entry := range files {
		fileObj, ok := entry.(map[string]any)
		if !ok {
			continue
		}
		path, _ := fileObj["path"].(string)
		content, _ := fileObj["content"].(string)
		contents[path] = content
	}
	if strings.Contains(contents["core/configs/chips/stm32wb55.yaml"], "TODO") {
		t.Fatalf("expected mb1355c chip profile to be fully populated, got: %s", contents["core/configs/chips/stm32wb55.yaml"])
	}
	if strings.Contains(contents["core/configs/systems/mb1355c.yaml"], "TODO") {
		t.Fatalf("expected mb1355c system profile to be fully populated, got: %s", contents["core/configs/systems/mb1355c.yaml"])
	}
	sourceDocs, ok := artifact["source_docs"].([]any)
	if !ok || len(sourceDocs) < 3 {
		t.Fatalf("expected auto-resolved source docs, got=%v", artifact["source_docs"])
	}
	if _, exists := artifact["requirements"]; exists {
		t.Fatalf("legacy placeholder shape leaked into artifact: %s", string(data))
	}
	appliedChip := filepath.Join(repoRoot, "core/configs/chips/stm32wb55.yaml")
	if _, err := os.Stat(appliedChip); err != nil {
		t.Fatalf("expected promoted repo bundle file at %s: %v", appliedChip, err)
	}
	appliedSmokeManifest := filepath.Join(repoRoot, "core/examples/mb1355c/board_firmware/Cargo.toml")
	if _, err := os.Stat(appliedSmokeManifest); err != nil {
		t.Fatalf("expected smoke firmware manifest at %s: %v", appliedSmokeManifest, err)
	}
	if _, ok := contents["core/examples/mb1355c/uart-smoke.yaml"]; !ok {
		t.Fatalf("expected uart smoke script in generated bundle")
	}
}

func TestRunSynthesisJob_PeripheralRequestWritesStrictIRDraft(t *testing.T) {
	t.Setenv("XAI_API_KEY", "")

	dir := t.TempDir()
	result, err := runSynthesisJob(context.Background(), &Job{
		ID:            "synth-peripheral-test",
		Type:          JobTypeSynthesize,
		ArtifactDir:   dir,
		ComponentName: "ADXL345",
		Requirements:  "I2C interface required. Register 0x00 should return Device ID 0xE5.",
		DatasheetURL:  "https://www.analog.com/media/en/technical-documentation/data-sheets/ADXL345.pdf",
	})
	if err != nil {
		t.Fatalf("runSynthesisJob failed: %v", err)
	}
	if !result.Pass {
		t.Fatalf("expected synthesis pass, got fail: %+v", result)
	}

	data, err := os.ReadFile(filepath.Join(dir, "output.json"))
	if err != nil {
		t.Fatalf("ReadFile output.json failed: %v", err)
	}
	var artifact map[string]any
	if err := json.Unmarshal(data, &artifact); err != nil {
		t.Fatalf("json.Unmarshal failed: %v", err)
	}
	if got := artifact["artifact_type"]; got != "strict_ir_draft" {
		t.Fatalf("unexpected artifact_type: got=%v", got)
	}
	contractResult, ok := artifact["contract_result"].(map[string]any)
	if !ok {
		t.Fatalf("expected contract_result object, got=%T", artifact["contract_result"])
	}
	if contractResult["request_kind"] != "peripheral_model_ingest" {
		t.Fatalf("unexpected contract_result.request_kind: %v", contractResult["request_kind"])
	}
	modelDraft, ok := artifact["model_draft"].(map[string]any)
	if !ok {
		t.Fatalf("expected model_draft object, got=%T", artifact["model_draft"])
	}
	registers, ok := modelDraft["registers"].([]any)
	if !ok || len(registers) == 0 {
		t.Fatalf("expected register hints in model draft, got=%v", modelDraft["registers"])
	}
	if _, exists := artifact["requirements"]; exists {
		t.Fatalf("legacy placeholder shape leaked into artifact: %s", string(data))
	}
}

func TestRunSynthesisJob_ArtifactOnlyDoesNotWriteRepo(t *testing.T) {
	t.Setenv("XAI_API_KEY", "")

	dir := t.TempDir()
	repoRoot := t.TempDir()
	fakeLabwired := filepath.Join(dir, "labwired")
	script := "#!/bin/sh\nprintf '{\"valid\":true,\"statistics\":{\"total_checks\":3}}'\n"
	if err := os.WriteFile(fakeLabwired, []byte(script), 0o755); err != nil {
		t.Fatalf("failed to write fake labwired: %v", err)
	}
	result, err := runSynthesisJob(context.Background(), &Job{
		ID:                  "synth-board-artifact-only",
		Type:                JobTypeSynthesize,
		ArtifactDir:         dir,
		LabWiredPath:        fakeLabwired,
		RepoRootDir:         repoRoot,
		SynthesisKind:       "board_onboarding",
		DryRun:              true,
		PromotionMode:       "artifact_only",
		ComponentName:       "MB1355C / NUCLEO-WB55RG board onboarding proof",
		Requirements:        "Board onboarding contract: boot and uart_console.",
		Board:               &synthesis.BoardSpec{MarketingName: "NUCLEO-WB55RG", BoardID: "MB1355C", MCU: "STM32WB55RG"},
		DesiredCapabilities: []string{"boot", "uart_console"},
		ValidationTargets:   []string{"uart_smoke"},
	})
	if err != nil {
		t.Fatalf("runSynthesisJob failed: %v", err)
	}
	if !result.Pass {
		t.Fatalf("expected synthesis pass, got fail: %+v", result)
	}
	appliedChip := filepath.Join(repoRoot, "core/configs/chips/stm32wb55.yaml")
	if _, err := os.Stat(appliedChip); !os.IsNotExist(err) {
		t.Fatalf("expected no repo write for artifact_only mode, stat err=%v", err)
	}
}

func TestApplyRepoBundleToRepo_RejectsPathTraversal(t *testing.T) {
	repoRoot := t.TempDir()
	err := applyRepoBundleToRepo(&Job{RepoRootDir: repoRoot}, &synthesis.RepoBundle{
		Files: []synthesis.GeneratedFile{
			{
				Path:    "../escape.txt",
				Content: "nope",
			},
		},
	})
	if err == nil {
		t.Fatal("expected path traversal rejection, got nil")
	}
	if !strings.Contains(err.Error(), "outside repo root") {
		t.Fatalf("unexpected error: %v", err)
	}
}

func TestApplyRepoBundleToRepo_RequiresRepoRoot(t *testing.T) {
	err := applyRepoBundleToRepo(&Job{}, &synthesis.RepoBundle{
		Files: []synthesis.GeneratedFile{
			{
				Path:    "core/configs/chips/test.yaml",
				Content: "schema_version: \"1.0\"\n",
			},
		},
	})
	if err == nil {
		t.Fatal("expected repo root validation error, got nil")
	}
	if !strings.Contains(err.Error(), "repo root unavailable") {
		t.Fatalf("unexpected error: %v", err)
	}
}

func TestNormalizeSynthesisRequest_RejectsPeripheralGitPromotion(t *testing.T) {
	_, err := normalizeSynthesisRequest(synthesisAPIRequest{
		Kind:          "peripheral_model_ingest",
		PromotionMode: "open_pr",
		ComponentName: "ADXL345",
		Requirements:  "I2C register model",
		DatasheetURL:  "https://example.com/adxl345.pdf",
	})
	if err == nil {
		t.Fatal("expected peripheral promotion mode rejection")
	}
	if !strings.Contains(err.Error(), "unsupported for kind=peripheral_model_ingest") {
		t.Fatalf("unexpected error: %v", err)
	}
}

func TestPromoteRepoBundleWithGit_CommitToBranch(t *testing.T) {
	repoRoot := t.TempDir()
	initGitRepo(t, repoRoot)

	job := &Job{
		ID:            "synth-pr-1",
		ArtifactDir:   t.TempDir(),
		RepoRootDir:   repoRoot,
		PromotionMode: "commit_to_branch",
		ComponentName: "NUCLEO-G474RE board onboarding proof",
		Board:         &synthesis.BoardSpec{MarketingName: "NUCLEO-G474RE", BoardID: "nucleo_g474re", MCU: "STM32G474RE"},
	}
	bundle := &synthesis.RepoBundle{
		Files: []synthesis.GeneratedFile{
			{Path: "core/configs/chips/stm32g474re.yaml", Content: "schema_version: \"1.0\"\nname: \"STM32G474RE\"\narch: \"arm\"\nflash:\n  base: 0x08000000\n  size: \"512KB\"\nram:\n  base: 0x20000000\n  size: \"128KB\"\nperipherals: []\n"},
			{Path: "core/configs/systems/nucleo_g474re.yaml", Content: "schema_version: \"1.0\"\nname: \"nucleo_g474re\"\nchip: \"../chips/stm32g474re.yaml\"\nboard_io: []\n"},
		},
	}

	assertions, err := promoteRepoBundleWithGit(job, bundle)
	if err != nil {
		t.Fatalf("promoteRepoBundleWithGit failed: %v", err)
	}
	if assertions < 2 {
		t.Fatalf("expected git promotion assertions, got %d", assertions)
	}

	branchOutput, err := gitOutput(repoRoot, nil, "branch", "--list", "foundry/onboard-nucleo-g474re-1")
	if err != nil {
		t.Fatalf("git branch list failed: %v", err)
	}
	if !strings.Contains(branchOutput, "foundry/onboard-nucleo-g474re-1") {
		t.Fatalf("expected promotion branch, got %q", branchOutput)
	}
	data, err := os.ReadFile(filepath.Join(job.ArtifactDir, "git_promotion_result.json"))
	if err != nil {
		t.Fatalf("expected git promotion artifact: %v", err)
	}
	if !strings.Contains(string(data), "\"mode\": \"commit_to_branch\"") {
		t.Fatalf("unexpected promotion result: %s", string(data))
	}
}

func TestPromoteRepoBundleWithGit_OpenPR(t *testing.T) {
	repoRoot := t.TempDir()
	initGitRepo(t, repoRoot)
	remoteRoot := t.TempDir()
	cmd := exec.Command("git", "init", "--bare", remoteRoot)
	if output, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("git init --bare failed: %v (%s)", err, string(output))
	}
	if err := runGitCommand(repoRoot, gitAuthorEnv(), "remote", "add", "origin", remoteRoot); err != nil {
		t.Fatalf("failed to add remote: %v", err)
	}
	if err := runGitCommand(repoRoot, gitAuthorEnv(), "push", "-u", "origin", "main"); err != nil {
		t.Fatalf("failed to seed remote main: %v", err)
	}

	fakeGHDir := t.TempDir()
	fakeGH := filepath.Join(fakeGHDir, "gh")
	if err := os.WriteFile(fakeGH, []byte("#!/bin/sh\nprintf 'https://example.com/pr/123\\n'\n"), 0o755); err != nil {
		t.Fatalf("failed to write fake gh: %v", err)
	}
	t.Setenv("GH_PATH", fakeGH)

	job := &Job{
		ID:            "synth-pr-2",
		ArtifactDir:   t.TempDir(),
		RepoRootDir:   repoRoot,
		PromotionMode: "open_pr",
		ComponentName: "ESP32-C3 DevKit board onboarding proof",
		Board:         &synthesis.BoardSpec{MarketingName: "ESP32-C3 DevKit", BoardID: "esp32c3_devkit", MCU: "ESP32C3"},
	}
	bundle := &synthesis.RepoBundle{
		Files: []synthesis.GeneratedFile{
			{Path: "core/configs/chips/esp32c3.yaml", Content: "schema_version: \"1.0\"\nname: \"ESP32C3\"\narch: \"riscv\"\nflash:\n  base: 0x42000000\n  size: \"4MB\"\nram:\n  base: 0x3FC80000\n  size: \"400KB\"\nperipherals: []\n"},
		},
	}

	assertions, err := promoteRepoBundleWithGit(job, bundle)
	if err != nil {
		t.Fatalf("promoteRepoBundleWithGit failed: %v", err)
	}
	if assertions < 3 {
		t.Fatalf("expected open_pr assertions, got %d", assertions)
	}

	data, err := os.ReadFile(filepath.Join(job.ArtifactDir, "git_promotion_result.json"))
	if err != nil {
		t.Fatalf("expected git promotion artifact: %v", err)
	}
	if !strings.Contains(string(data), "https://example.com/pr/123") {
		t.Fatalf("expected PR URL in promotion result, got %s", string(data))
	}
	refs, err := exec.Command("git", "--git-dir", remoteRoot, "for-each-ref", "--format=%(refname)", "refs/heads/foundry/onboard-esp32c3-devkit-2").CombinedOutput()
	if err != nil {
		t.Fatalf("failed to list remote refs: %v (%s)", err, string(refs))
	}
	if !strings.Contains(string(refs), "refs/heads/foundry/onboard-esp32c3-devkit-2") {
		t.Fatalf("expected pushed branch in bare remote, got %q", string(refs))
	}
}

func TestInferRustTargetFromSmokeScript(t *testing.T) {
	script := `schema_version: "1.0"
inputs:
  firmware: "./board_firmware/target/thumbv8m.main-none-eabi/release/firmware-demo"
  system: "./system.yaml"
`
	if got := inferRustTargetFromSmokeScript(script); got != "thumbv8m.main-none-eabi" {
		t.Fatalf("unexpected target: %s", got)
	}
	if got := inferRustTargetFromSmokeScript(`schema_version: "1.0"`); got != "thumbv7em-none-eabi" {
		t.Fatalf("unexpected default target: %s", got)
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
	if err := store.UpsertCatalogAsset(db.CatalogAsset{
		ID:         "h1",
		Name:       "h1",
		PassRate:   100,
		Verified:   true,
		SourceRef:  "core/configs/onboarding/h1.yaml",
		SourceType: "core-config",
	}); err != nil {
		t.Fatalf("UpsertCatalogAsset(h1) failed: %v", err)
	}
	if err := store.UpsertCatalogAsset(db.CatalogAsset{
		ID:         "h2",
		Name:       "h2",
		PassRate:   75,
		Verified:   false,
		SourceRef:  "core/configs/onboarding/h2.yaml",
		SourceType: "core-config",
	}); err != nil {
		t.Fatalf("UpsertCatalogAsset(h2) failed: %v", err)
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
	if items[0].ID != "h1" || items[0].Tier != 1 || items[0].Type != "board" {
		t.Errorf("unexpected first item: %+v", items[0])
	}
	if items[1].ID != "h2" || items[1].Tier != 2 || items[1].Type != "board" {
		t.Errorf("unexpected second item: %+v", items[1])
	}
	if items[0].ReplPath == "" || items[1].ReplPath == "" {
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
	srv, store, artifactsDir := newTestServer(t)
	if err := os.MkdirAll(artifactsDir, 0o755); err != nil {
		t.Fatalf("MkdirAll artifacts dir failed: %v", err)
	}
	key := createKey(t, store, "ws-health-metrics")

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

	rr := doAuthRequest(t, srv, http.MethodGet, "/v1/health", key, nil)
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

func TestRunLabWiredAssetValidate_RejectsZeroCheckValidation(t *testing.T) {
	dir := t.TempDir()
	fakeLabwired := filepath.Join(dir, "labwired")
	if err := os.WriteFile(fakeLabwired, []byte("#!/bin/sh\nprintf '{\"valid\":true,\"statistics\":{\"total_checks\":0}}'\n"), 0o755); err != nil {
		t.Fatalf("failed to write fake labwired: %v", err)
	}

	job := &Job{
		LabWiredPath: fakeLabwired,
		ArtifactDir:  dir,
	}
	dummyAsset := filepath.Join(dir, "chip.yaml")
	if err := os.WriteFile(dummyAsset, []byte("schema_version: \"1.0\"\nname: test\narch: arm\nflash:\n  base: 0x08000000\n  size: \"64KB\"\nram:\n  base: 0x20000000\n  size: \"16KB\"\nperipherals: []\n"), 0o644); err != nil {
		t.Fatalf("failed to write dummy asset: %v", err)
	}

	err := runLabWiredAssetValidate(job, dir, "--chip", dummyAsset, "validate_chip.json")
	if err == nil {
		t.Fatal("expected zero-check validation to fail")
	}
	if !strings.Contains(err.Error(), "no substantive checks") {
		t.Fatalf("unexpected error: %v", err)
	}
}

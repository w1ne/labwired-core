package api

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/gorilla/mux"
	"github.com/labwired/foundry-backend/internal/catalog"
	"github.com/labwired/foundry-backend/internal/db"
	"github.com/labwired/foundry-backend/internal/verification"
	stripe "github.com/stripe/stripe-go/v76"
	"github.com/stripe/stripe-go/v76/webhook"
)

// maxInputLen caps user-controlled strings before they reach synthesis or the LLM.
const maxInputLen = 32 * 1024 // 32 KB
const maxVerifyBodyBytes = int64(1 << 20)

func sanitizeInput(s string) string {
	if len(s) > maxInputLen {
		return s[:maxInputLen]
	}
	return s
}

type JobStatus string

const (
	StatusQueued  JobStatus = "queued"
	StatusRunning JobStatus = "running"
	StatusPass    JobStatus = "pass"
	StatusFail    JobStatus = "fail"
	StatusError   JobStatus = "error"
)

type JobType string

const (
	JobTypeVerify     JobType = "verify"
	JobTypeSynthesize JobType = "synthesize"
)

// Job carries the data needed by the background worker to execute a simulation or synthesis.
type Job struct {
	ID            string
	WorkspaceID   string
	Type          JobType
	IRPath        string // temp file containing the submitted YAML/JSON (Verify)
	ArtifactDir   string // directory where output files will be written
	DatasheetURL  string // (Synthesize) URL to parse
	ComponentName string // (Synthesize) Name of the component
	Requirements  string // (Synthesize)
}

type Server struct {
	router       *mux.Router
	jobs         sync.Map // run_id -> *Job (in-memory while queued/running)
	jobQueue     chan *Job
	workerCount  int
	queueMu      sync.RWMutex
	shuttingDown bool
	workersWG    sync.WaitGroup
	orchestrator *verification.Orchestrator
	store        *db.Store
	catalog      *catalog.Manager
	artifactsDir string

	maxInflightPerWorkspace int
	inflightMu              sync.Mutex
	inflightByWorkspace     map[string]int

	artifactRetentionDays    int
	runMetadataRetentionDays int
	cleanupInterval          time.Duration
	cleanupStopCh            chan struct{}

	metrics serverMetrics
}

type ServerOptions struct {
	WorkerCount              int
	MaxInflightPerWorkspace  int
	ArtifactRetentionDays    int
	RunMetadataRetentionDays int
	CleanupInterval          time.Duration
}

func DefaultServerOptions() ServerOptions {
	return ServerOptions{
		WorkerCount:              4,
		MaxInflightPerWorkspace:  8,
		ArtifactRetentionDays:    14,
		RunMetadataRetentionDays: 90,
		CleanupInterval:          time.Hour,
	}
}

type serverMetrics struct {
	InflightLimitRejected atomic.Int64
	QueueFullRejected     atomic.Int64
	ShuttingDownRejected  atomic.Int64

	CleanupArtifactDeleted       atomic.Int64
	CleanupArtifactSkippedUnsafe atomic.Int64
	CleanupArtifactDeleteFailed  atomic.Int64
	CleanupDBPathClearFailed     atomic.Int64
	CleanupMetadataRowsDeleted   atomic.Int64

	StripeDuplicateEvents atomic.Int64
}

func NewServer(orch *verification.Orchestrator, store *db.Store, cat *catalog.Manager, artifactsDir string, opts ServerOptions) *Server {
	if opts.WorkerCount <= 0 {
		opts.WorkerCount = 1
	}
	if opts.MaxInflightPerWorkspace <= 0 {
		opts.MaxInflightPerWorkspace = 1
	}
	if opts.ArtifactRetentionDays <= 0 {
		opts.ArtifactRetentionDays = 14
	}
	if opts.RunMetadataRetentionDays <= 0 {
		opts.RunMetadataRetentionDays = 90
	}
	if opts.CleanupInterval <= 0 {
		opts.CleanupInterval = time.Hour
	}

	s := &Server{
		router:                   mux.NewRouter(),
		jobQueue:                 make(chan *Job, 100),
		workerCount:              opts.WorkerCount,
		orchestrator:             orch,
		store:                    store,
		catalog:                  cat,
		artifactsDir:             artifactsDir,
		maxInflightPerWorkspace:  opts.MaxInflightPerWorkspace,
		inflightByWorkspace:      make(map[string]int),
		artifactRetentionDays:    opts.ArtifactRetentionDays,
		runMetadataRetentionDays: opts.RunMetadataRetentionDays,
		cleanupInterval:          opts.CleanupInterval,
		cleanupStopCh:            make(chan struct{}),
	}
	s.routes()
	for i := 0; i < s.workerCount; i++ {
		s.workersWG.Add(1)
		go s.worker(i)
	}
	s.workersWG.Add(1)
	go s.cleanupLoop()
	return s
}

func (s *Server) routes() {
	// Public endpoints
	s.router.HandleFunc("/v1/catalog", s.handleListCatalog).Methods("GET")
	s.router.HandleFunc("/v1/catalog/{id}", s.handleGetCatalogAsset).Methods("GET")
	s.router.HandleFunc("/v1/info", s.handleInfo).Methods("GET")
	s.router.HandleFunc("/v1/hardware", s.handleListHardware).Methods("GET")
	s.router.HandleFunc("/v1/health", s.handleHealth).Methods("GET")
	s.router.HandleFunc("/v1/schema/synthesis", s.handleSchemaSynthesis).Methods("GET")
	s.router.HandleFunc("/v1/openapi.yaml", func(w http.ResponseWriter, r *http.Request) {
		http.ServeFile(w, r, "static/openapi.yaml")
	}).Methods("GET")
	s.router.PathPrefix("/v1/docs").Handler(http.StripPrefix("/v1/docs", http.FileServer(http.Dir("static"))))

	// Stripe webhook (no API key auth — verified by signature)
	s.router.HandleFunc("/v1/webhooks/stripe", s.handleStripeWebhook).Methods("POST")

	// Protected VaaS Routes (Requires API Key)
	protected := s.router.PathPrefix("/v1").Subrouter()
	protected.Use(s.authMiddleware)

	// Synthesis-as-a-Service endpoints
	protected.Handle("/estimate", http.HandlerFunc(s.handleEstimate)).Methods("POST")
	protected.Handle("/synthesize", s.quotaMiddleware(http.HandlerFunc(s.handleSynthesize))).Methods("POST")

	// Quota-protected endpoints (Consume run credits)
	protected.Handle("/models/verify", s.quotaMiddleware(http.HandlerFunc(s.handleVerifyModel))).Methods("POST")
	protected.Handle("/systems/verify", s.quotaMiddleware(http.HandlerFunc(s.handleVerifySystem))).Methods("POST")

	// Run polling
	protected.HandleFunc("/runs/{run_id}", s.handleGetRun).Methods("GET")
	protected.HandleFunc("/runs/{run_id}/artifacts/{file}", s.handleGetRunArtifact).Methods("GET")
	protected.HandleFunc("/runs", s.handleListRuns).Methods("GET")

	protected.HandleFunc("/usage", s.handleUsage).Methods("GET")
}

func (s *Server) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	s.corsMiddleware(s.router).ServeHTTP(w, r)
}

// Shutdown stops accepting new jobs, drains queued work, and waits for workers.
func (s *Server) Shutdown(ctx context.Context) error {
	s.queueMu.Lock()
	if !s.shuttingDown {
		s.shuttingDown = true
		close(s.jobQueue)
		close(s.cleanupStopCh)
	}
	s.queueMu.Unlock()

	done := make(chan struct{})
	go func() {
		s.workersWG.Wait()
		close(done)
	}()

	select {
	case <-done:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

func (s *Server) cleanupLoop() {
	defer s.workersWG.Done()
	ticker := time.NewTicker(s.cleanupInterval)
	defer ticker.Stop()

	for {
		select {
		case <-s.cleanupStopCh:
			return
		case <-ticker.C:
			if err := s.cleanupExpiredArtifactsOnce(time.Now()); err != nil {
				log.Printf("[cleanup] artifact cleanup failed: %v", err)
			}
		}
	}
}

func (s *Server) cleanupExpiredArtifactsOnce(now time.Time) error {
	cutoff := now.Add(-time.Duration(s.artifactRetentionDays) * 24 * time.Hour)
	candidates, err := s.store.ListRunsWithArtifactsBefore(cutoff)
	if err != nil {
		return err
	}
	if len(candidates) > 0 {
		rootAbs, err := filepath.Abs(s.artifactsDir)
		if err != nil {
			return err
		}
		rootAbs = filepath.Clean(rootAbs)

		for _, c := range candidates {
			targetAbs, err := filepath.Abs(c.ArtifactsPath)
			if err != nil {
				log.Printf("[cleanup] skipping run %s: resolve artifact path failed: %v", c.RunID, err)
				continue
			}
			targetAbs = filepath.Clean(targetAbs)
			if targetAbs != rootAbs && !strings.HasPrefix(targetAbs, rootAbs+string(os.PathSeparator)) {
				log.Printf("[cleanup] skipping run %s: artifact path outside root (%s)", c.RunID, targetAbs)
				s.metrics.CleanupArtifactSkippedUnsafe.Add(1)
				continue
			}

			if err := os.RemoveAll(targetAbs); err != nil {
				log.Printf("[cleanup] failed to remove artifacts for run %s: %v", c.RunID, err)
				s.metrics.CleanupArtifactDeleteFailed.Add(1)
				continue
			}
			if err := s.store.ClearRunArtifactsPath(c.RunID); err != nil {
				log.Printf("[cleanup] removed artifacts but failed to clear DB path for run %s: %v", c.RunID, err)
				s.metrics.CleanupDBPathClearFailed.Add(1)
				continue
			}
			s.metrics.CleanupArtifactDeleted.Add(1)
		}
	}

	metadataCutoff := now.Add(-time.Duration(s.runMetadataRetentionDays) * 24 * time.Hour)
	deleted, err := s.store.PruneTerminalRunsBefore(metadataCutoff)
	if err != nil {
		return err
	}
	if deleted > 0 {
		s.metrics.CleanupMetadataRowsDeleted.Add(deleted)
	}
	return nil
}

func (s *Server) tryAcquireWorkspaceSlot(workspaceID string) bool {
	s.inflightMu.Lock()
	defer s.inflightMu.Unlock()
	n := s.inflightByWorkspace[workspaceID]
	if n >= s.maxInflightPerWorkspace {
		s.metrics.InflightLimitRejected.Add(1)
		return false
	}
	s.inflightByWorkspace[workspaceID] = n + 1
	return true
}

func (s *Server) releaseWorkspaceSlot(workspaceID string) {
	s.inflightMu.Lock()
	defer s.inflightMu.Unlock()
	n := s.inflightByWorkspace[workspaceID]
	if n <= 1 {
		delete(s.inflightByWorkspace, workspaceID)
		return
	}
	s.inflightByWorkspace[workspaceID] = n - 1
}

// ── Public Catalog ───────────────────────────────────────────────────────────

func (s *Server) handleListCatalog(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(s.catalog.List())
}

func (s *Server) handleGetCatalogAsset(w http.ResponseWriter, r *http.Request) {
	id := mux.Vars(r)["id"]
	asset, ok := s.catalog.Get(id)
	if !ok {
		sendError(w, http.StatusNotFound, "ASSET_NOT_FOUND", "The requested asset ID does not exist in the catalog.", "Check /v1/catalog for a list of valid asset IDs.")
		return
	}
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(asset)
}

func (s *Server) handleInfo(w http.ResponseWriter, r *http.Request) {
	info := map[string]interface{}{
		"version":      "1.0.0",
		"engine":       "LabWired Foundry (Go Native)",
		"capabilities": []string{"synthesis", "solid_proof", "formal_verification"},
		"docs_url":     "/v1/docs",
	}
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(info)
}

func (s *Server) handleListHardware(w http.ResponseWriter, r *http.Request) {
	items, err := s.store.ListHardware()
	if err != nil {
		sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to retrieve hardware list.", "")
		return
	}
	if items == nil {
		items = []db.HardwareItem{}
	}
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(items)
}

func (s *Server) handleHealth(w http.ResponseWriter, r *http.Request) {
	status := "healthy"
	components := map[string]interface{}{
		"api": map[string]string{"status": "up"},
	}

	// Check Database
	dbStatus := "up"
	if err := s.store.Ping(); err != nil {
		dbStatus = "down"
		status = "unhealthy"
		log.Printf("[health] database connection failed: %v", err)
	}
	components["database"] = map[string]string{"status": dbStatus}

	// Check Storage (Artifacts directory)
	storageStatus := "up"
	if _, err := os.Stat(s.artifactsDir); os.IsNotExist(err) {
		storageStatus = "down"
		status = "unhealthy"
		log.Printf("[health] artifacts directory does not exist: %s", s.artifactsDir)
	} else if err := os.MkdirAll(s.artifactsDir, 0755); err != nil {
		storageStatus = "degraded"
		status = "degraded"
		log.Printf("[health] artifacts directory not writable: %v", err)
	}
	components["storage"] = map[string]string{"status": storageStatus}

	// Check Worker (Job queue)
	workerStatus := "up"
	if cap(s.jobQueue) == len(s.jobQueue) {
		workerStatus = "saturated"
		// Not necessarily unhealthy, but worth reporting
	}
	components["worker"] = map[string]interface{}{
		"status": workerStatus,
		"queue": map[string]int{
			"length":   len(s.jobQueue),
			"capacity": cap(s.jobQueue),
		},
		"max_inflight_per_workspace": s.maxInflightPerWorkspace,
	}
	components["metrics"] = map[string]int64{
		"inflight_limit_rejected":       s.metrics.InflightLimitRejected.Load(),
		"queue_full_rejected":           s.metrics.QueueFullRejected.Load(),
		"shutting_down_rejected":        s.metrics.ShuttingDownRejected.Load(),
		"cleanup_artifact_deleted":      s.metrics.CleanupArtifactDeleted.Load(),
		"cleanup_artifact_skipped":      s.metrics.CleanupArtifactSkippedUnsafe.Load(),
		"cleanup_artifact_failures":     s.metrics.CleanupArtifactDeleteFailed.Load(),
		"cleanup_db_path_clear_failed":  s.metrics.CleanupDBPathClearFailed.Load(),
		"cleanup_metadata_rows_deleted": s.metrics.CleanupMetadataRowsDeleted.Load(),
		"stripe_duplicate_events":       s.metrics.StripeDuplicateEvents.Load(),
	}

	w.Header().Set("Content-Type", "application/json")
	if status == "unhealthy" {
		w.WriteHeader(http.StatusServiceUnavailable)
	}
	json.NewEncoder(w).Encode(map[string]interface{}{
		"status":     status,
		"timestamp":  time.Now().Format(time.RFC3339),
		"components": components,
	})
}

// ── Estimate ─────────────────────────────────────────────────────────────────

func calculateSynthesisCost(componentName, requirements string) int {
	cost := 15
	lower := strings.ToLower(componentName + " " + requirements)
	switch {
	case strings.Contains(lower, "mcu") || strings.Contains(lower, "core") || strings.Contains(lower, "cpu"):
		cost = 1500
	case len(requirements) > 500:
		cost = 50
	}
	return cost
}

func (s *Server) handleEstimate(w http.ResponseWriter, r *http.Request) {
	var req struct {
		ComponentName string `json:"component_name"`
		Requirements  string `json:"requirements"`
		DatasheetURL  string `json:"datasheet_url,omitempty"`
	}
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		sendError(w, http.StatusBadRequest, "INVALID_JSON", "The request body could not be parsed as valid JSON.", "Verify the JSON syntax.")
		return
	}
	if req.ComponentName == "" || req.Requirements == "" {
		sendError(w, http.StatusBadRequest, "MISSING_REQUIRED_FIELDS", "component_name and requirements are required.", "")
		return
	}
	req.ComponentName = sanitizeInput(req.ComponentName)
	req.Requirements = sanitizeInput(req.Requirements)

	cost := calculateSynthesisCost(req.ComponentName, req.Requirements)
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(map[string]interface{}{
		"component_name":      req.ComponentName,
		"estimated_cost_runs": cost,
		"message":             fmt.Sprintf("Synthesizing %s will cost approximately %d runs.", req.ComponentName, cost),
	})
}

// ── Synthesize ────────────────────────────────────────────────────────────────

func (s *Server) handleSynthesize(w http.ResponseWriter, r *http.Request) {
	var req struct {
		ComponentName string `json:"component_name"`
		Requirements  string `json:"requirements"`
		DatasheetURL  string `json:"datasheet_url,omitempty"`
	}
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		sendError(w, http.StatusBadRequest, "INVALID_JSON", "The request body could not be parsed as valid JSON.", "Verify the JSON syntax.")
		return
	}
	if req.ComponentName == "" || req.Requirements == "" {
		sendError(w, http.StatusBadRequest, "MISSING_REQUIRED_FIELDS", "component_name and requirements are required.", "")
		return
	}
	req.ComponentName = sanitizeInput(req.ComponentName)
	req.Requirements = sanitizeInput(req.Requirements)

	cost := calculateSynthesisCost(req.ComponentName, req.Requirements)
	jobID := fmt.Sprintf("synth-%d", time.Now().UnixNano())
	apiKey, ok := apiKeyFromContext(r.Context())
	if !ok {
		sendError(w, http.StatusUnauthorized, "UNAUTHORIZED", "API key context missing.", "Ensure auth middleware is enabled.")
		return
	}

	// Try to acquire an execution slot for this workspace.
	if !s.tryAcquireWorkspaceSlot(apiKey.WorkspaceID) {
		sendError(w, http.StatusTooManyRequests, "WORKSPACE_INFLIGHT_LIMIT", fmt.Sprintf("Workspace has reached max in-flight jobs (%d).", s.maxInflightPerWorkspace), "Wait for running jobs to complete before submitting more.")
		return
	}
	enqueued := false
	defer func() {
		if !enqueued {
			s.releaseWorkspaceSlot(apiKey.WorkspaceID)
		}
	}()

	artifactDir := filepath.Join(s.artifactsDir, jobID)
	if err := os.MkdirAll(artifactDir, 0755); err != nil {
		sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to create artifact directory.", "")
		return
	}

	job := &Job{
		ID:            jobID,
		WorkspaceID:   apiKey.WorkspaceID,
		Type:          JobTypeSynthesize,
		ArtifactDir:   artifactDir,
		DatasheetURL:  req.DatasheetURL,
		ComponentName: req.ComponentName,
		Requirements:  req.Requirements,
	}
	s.jobs.Store(jobID, job)

	s.queueMu.RLock()
	if s.shuttingDown {
		s.queueMu.RUnlock()
		sendError(w, http.StatusServiceUnavailable, "SERVER_SHUTTING_DOWN", "Server is shutting down.", "")
		return
	}
	// Reserve all synthesis runs atomically to avoid quota races.
	runIDs := make([]string, 0, cost)
	for i := 0; i < cost; i++ {
		runIDs = append(runIDs, fmt.Sprintf("%s-%d", jobID, i))
	}
	if err := s.store.ReserveRunsForWorkspace(runIDs, apiKey.WorkspaceID, string(StatusQueued)); err != nil {
		s.queueMu.RUnlock()
		if err == db.ErrQuotaExceeded {
			sendError(w, http.StatusTooManyRequests, "QUOTA_EXCEEDED", "Workspace has exceeded its monthly run limit.", "Purchase more credits or upgrade your tier.")
			return
		}
		sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to reserve synthesis runs.", "Retry later.")
		return
	}

	select {
	case s.jobQueue <- job:
		enqueued = true
	default:
		s.queueMu.RUnlock()
		// Revert the DB reservation since we couldn't queue it
		for _, rid := range runIDs {
			_ = s.store.UpdateRunStatus(rid, string(StatusError), 0, 0, "")
		}
		sendError(w, http.StatusServiceUnavailable, "QUEUE_FULL", "The queue is at capacity.", "Retry later.")
		return
	}
	s.queueMu.RUnlock()

	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(http.StatusAccepted)
	json.NewEncoder(w).Encode(map[string]interface{}{
		"job_id":   jobID,
		"status":   string(StatusQueued),
		"poll_url": fmt.Sprintf("/v1/runs/%s", jobID),
		"message":  "Synthesis job queued. The AI engine is analyzing the datasheet and generating the model.",
	})
}

// ── Async Verify Handlers ─────────────────────────────────────────────────────

// submitJob writes the YAML body to a temp file, creates the DB row and enqueues the job.
func (s *Server) submitJob(w http.ResponseWriter, r *http.Request, prefix string, body []byte) {
	apiKey, ok := apiKeyFromContext(r.Context())
	if !ok {
		sendError(w, http.StatusUnauthorized, "UNAUTHORIZED", "API key context missing.", "Ensure auth middleware is enabled.")
		return
	}

	if !s.tryAcquireWorkspaceSlot(apiKey.WorkspaceID) {
		sendError(
			w,
			http.StatusTooManyRequests,
			"WORKSPACE_INFLIGHT_LIMIT",
			fmt.Sprintf("Workspace has reached max in-flight jobs (%d).", s.maxInflightPerWorkspace),
			"Wait for running jobs to complete before submitting more.",
		)
		return
	}
	enqueued := false
	defer func() {
		if !enqueued {
			s.releaseWorkspaceSlot(apiKey.WorkspaceID)
		}
	}()

	log.Printf("[%s] Submitting job for workspace %s", prefix, apiKey.WorkspaceID)

	runID := fmt.Sprintf("%s-%d", prefix, time.Now().UnixNano())
	artifactDir := filepath.Join(s.artifactsDir, runID)
	if err := os.MkdirAll(artifactDir, 0755); err != nil {
		sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to create artifact directory.", "")
		return
	}

	// Write the submitted YAML to a temp file inside the artifact dir.
	irPath := filepath.Join(artifactDir, "input.yaml")
	if err := os.WriteFile(irPath, body, 0644); err != nil {
		sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to persist input.", "")
		return
	}

	// Persist the run as "queued" with atomic quota reservation.
	if err := s.store.ReserveRunForWorkspace(runID, apiKey.WorkspaceID, string(StatusQueued)); err != nil {
		if err == db.ErrQuotaExceeded {
			sendError(w, http.StatusTooManyRequests, "QUOTA_EXCEEDED", "Workspace has exceeded its monthly run limit.", "Purchase more credits or upgrade your tier.")
			return
		}
		log.Printf("[%s] ERROR: Failed to record run in DB: %v", prefix, err)
		sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to record simulation run.", "")
		return
	}

	job := &Job{
		ID:          runID,
		WorkspaceID: apiKey.WorkspaceID,
		Type:        JobTypeVerify,
		IRPath:      irPath,
		ArtifactDir: artifactDir,
	}
	s.jobs.Store(runID, job)

	s.queueMu.RLock()
	if s.shuttingDown {
		s.queueMu.RUnlock()
		s.metrics.ShuttingDownRejected.Add(1)
		sendError(w, http.StatusServiceUnavailable, "SERVER_SHUTTING_DOWN", "Server is shutting down and not accepting new jobs.", "Retry after the service recovers.")
		return
	}
	select {
	case s.jobQueue <- job:
		log.Printf("[%s] Job enqueued: %s", prefix, runID)
		enqueued = true
	default:
		// Queue full.
		_ = s.store.UpdateRunStatus(runID, string(StatusError), 0, 0, "")
		s.metrics.QueueFullRejected.Add(1)
		sendError(w, http.StatusServiceUnavailable, "QUEUE_FULL", "The simulation queue is at capacity.", "Retry after 5-10 seconds.")
		s.queueMu.RUnlock()
		return
	}
	s.queueMu.RUnlock()

	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(http.StatusAccepted)
	json.NewEncoder(w).Encode(map[string]interface{}{
		"run_id":   runID,
		"status":   string(StatusQueued),
		"poll_url": fmt.Sprintf("/v1/runs/%s", runID),
	})
}

func (s *Server) handleVerifyModel(w http.ResponseWriter, r *http.Request) {
	r.Body = http.MaxBytesReader(w, r.Body, maxVerifyBodyBytes)
	body, err := io.ReadAll(r.Body)
	if err != nil {
		sendError(w, http.StatusBadRequest, "INVALID_BODY", "Could not read request body.", "")
		return
	}
	s.submitJob(w, r, "run-model", body)
}

func (s *Server) handleVerifySystem(w http.ResponseWriter, r *http.Request) {
	r.Body = http.MaxBytesReader(w, r.Body, maxVerifyBodyBytes)
	body, err := io.ReadAll(r.Body)
	if err != nil {
		sendError(w, http.StatusBadRequest, "INVALID_BODY", "Could not read request body.", "")
		return
	}
	s.submitJob(w, r, "run-system", body)
}

// ── Polling ───────────────────────────────────────────────────────────────────

func (s *Server) handleGetRun(w http.ResponseWriter, r *http.Request) {
	runID := mux.Vars(r)["run_id"]
	apiKey, ok := apiKeyFromContext(r.Context())
	if !ok {
		sendError(w, http.StatusUnauthorized, "UNAUTHORIZED", "API key context missing.", "Ensure auth middleware is enabled.")
		return
	}
	record, err := s.store.GetRunForWorkspace(runID, apiKey.WorkspaceID)
	if err != nil {
		sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to fetch run status.", "")
		return
	}
	if record == nil {
		sendError(w, http.StatusNotFound, "RUN_NOT_FOUND", "No run found with that ID.", "")
		return
	}

	resp := map[string]interface{}{
		"run_id":            record.RunID,
		"status":            record.Status,
		"assertions_passed": record.AssertionsPassed,
		"assertions_total":  record.AssertionsTotal,
		"created_at":        record.CreatedAt,
	}
	if record.ArtifactsPath != "" {
		baseURL := "/v1/runs/" + record.RunID + "/artifacts"
		resp["artifacts"] = map[string]string{
			"ir_url":     baseURL + "/output.json",
			"vcd_url":    baseURL + "/proof.vcd",
			"result_url": baseURL + "/result.json",
		}
	}

	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(resp)
}

func (s *Server) handleGetRunArtifact(w http.ResponseWriter, r *http.Request) {
	apiKey, ok := apiKeyFromContext(r.Context())
	if !ok {
		sendError(w, http.StatusUnauthorized, "UNAUTHORIZED", "API key context missing.", "Ensure auth middleware is enabled.")
		return
	}

	runID := mux.Vars(r)["run_id"]
	record, err := s.store.GetRunForWorkspace(runID, apiKey.WorkspaceID)
	if err != nil {
		sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to fetch run status.", "")
		return
	}
	if record == nil {
		sendError(w, http.StatusNotFound, "RUN_NOT_FOUND", "No run found with that ID.", "")
		return
	}
	if record.ArtifactsPath == "" {
		sendError(w, http.StatusNotFound, "ARTIFACT_NOT_FOUND", "No artifacts available for this run.", "")
		return
	}

	name := mux.Vars(r)["file"]
	allowed := map[string]struct{}{
		"output.json": {},
		"proof.vcd":   {},
		"result.json": {},
		"error.log":   {},
	}
	if _, ok := allowed[name]; !ok {
		sendError(w, http.StatusNotFound, "ARTIFACT_NOT_FOUND", "Requested artifact is not available.", "")
		return
	}

	artifactPath := filepath.Join(record.ArtifactsPath, name)
	if _, err := os.Stat(artifactPath); err != nil {
		sendError(w, http.StatusNotFound, "ARTIFACT_NOT_FOUND", "Requested artifact was not found.", "")
		return
	}

	http.ServeFile(w, r, artifactPath)
}

func (s *Server) handleListRuns(w http.ResponseWriter, r *http.Request) {
	apiKey, ok := apiKeyFromContext(r.Context())
	if !ok {
		sendError(w, http.StatusUnauthorized, "UNAUTHORIZED", "API key context missing.", "Ensure auth middleware is enabled.")
		return
	}
	records, err := s.store.ListRunsForWorkspace(apiKey.WorkspaceID)
	if err != nil {
		sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to fetch run list.", "")
		return
	}
	if records == nil {
		records = []db.RunRecord{}
	}
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(records)
}

// ── Usage ─────────────────────────────────────────────────────────────────────

func (s *Server) handleUsage(w http.ResponseWriter, r *http.Request) {
	apiKey, ok := apiKeyFromContext(r.Context())
	if !ok {
		sendError(w, http.StatusUnauthorized, "UNAUTHORIZED", "API key context missing.", "Ensure auth middleware is enabled.")
		return
	}
	used, _ := s.store.CountRunsForWorkspace(apiKey.WorkspaceID)
	quota, _ := s.store.GetMonthlyQuota(apiKey.WorkspaceID)
	json.NewEncoder(w).Encode(map[string]any{
		"workspace_id":         apiKey.WorkspaceID,
		"tier":                 apiKey.Tier,
		"runs_used_this_month": used,
		"quota":                quota,
		"runs_remaining":       quota - used,
	})
}

// ── Stripe Webhook ────────────────────────────────────────────────────────────

func (s *Server) handleStripeWebhook(w http.ResponseWriter, r *http.Request) {
	const maxBodyBytes = int64(65536)
	r.Body = http.MaxBytesReader(w, r.Body, maxBodyBytes)
	payload, err := io.ReadAll(r.Body)
	if err != nil {
		sendError(w, http.StatusBadRequest, "INVALID_BODY", "Could not read webhook body.", "")
		return
	}

	webhookSecret := os.Getenv("STRIPE_WEBHOOK_SECRET")
	allowInsecureWebhook := os.Getenv("ALLOW_INSECURE_STRIPE_WEBHOOKS") == "true"
	if webhookSecret == "" && !allowInsecureWebhook {
		log.Println("[stripe] STRIPE_WEBHOOK_SECRET not set and insecure mode disabled")
		sendError(
			w,
			http.StatusServiceUnavailable,
			"WEBHOOK_CONFIG_ERROR",
			"Stripe webhook secret is not configured.",
			"Set STRIPE_WEBHOOK_SECRET or explicitly enable ALLOW_INSECURE_STRIPE_WEBHOOKS=true for local development.",
		)
		return
	}
	if webhookSecret == "" && allowInsecureWebhook {
		log.Println("[stripe] WARNING: running with insecure webhook mode (signature verification disabled)")
	} else {
		sigHeader := r.Header.Get("Stripe-Signature")
		if _, err := webhook.ConstructEvent(payload, sigHeader, webhookSecret); err != nil {
			sendError(w, http.StatusBadRequest, "INVALID_SIGNATURE", "Webhook signature verification failed.", "")
			return
		}
	}

	var event stripe.Event
	if err := json.Unmarshal(payload, &event); err != nil {
		sendError(w, http.StatusBadRequest, "INVALID_JSON", "Could not parse Stripe event.", "")
		return
	}

	if event.Type != "checkout.session.completed" {
		w.WriteHeader(http.StatusOK) // Acknowledge but ignore other event types.
		return
	}

	var session stripe.CheckoutSession
	if err := json.Unmarshal(event.Data.Raw, &session); err != nil {
		sendError(w, http.StatusBadRequest, "INVALID_SESSION", "Could not parse checkout session.", "")
		return
	}

	// workspace_id is passed as client_reference_id when creating the Stripe checkout session.
	workspaceID := session.ClientReferenceID
	if workspaceID == "" {
		log.Printf("[stripe] checkout.session.completed missing client_reference_id: %s", session.ID)
		w.WriteHeader(http.StatusOK)
		return
	}

	// Map amount_total (in cents) to runs. €49 = 4900 cents -> 1000 runs.
	const centsPerCreditPack = 4900
	runsToCredit := int(session.AmountTotal/int64(centsPerCreditPack)) * 1000
	if runsToCredit <= 0 {
		runsToCredit = 1000 // safe fallback for manual or test payments
	}

	applied, err := s.store.ApplyStripeCreditIfNew(event.ID, session.ID, workspaceID, runsToCredit)
	if err != nil {
		log.Printf("[stripe] failed to credit %d runs to workspace %s for event %s: %v", runsToCredit, workspaceID, event.ID, err)
		w.WriteHeader(http.StatusInternalServerError)
		return
	}
	if !applied {
		s.metrics.StripeDuplicateEvents.Add(1)
		log.Printf("[stripe] duplicate webhook event ignored: event=%s session=%s workspace=%s", event.ID, session.ID, workspaceID)
		w.WriteHeader(http.StatusOK)
		return
	}

	log.Printf("[stripe] credited %d runs to workspace %s (session %s, event %s)", runsToCredit, workspaceID, session.ID, event.ID)
	w.WriteHeader(http.StatusOK)
}

// ── Schema ────────────────────────────────────────────────────────────────────

func (s *Server) handleSchemaSynthesis(w http.ResponseWriter, r *http.Request) {
	schema := `{
		"$schema": "http://json-schema.org/draft-07/schema#",
		"type": "object",
		"properties": {
			"peripheral_id": { "type": "string", "description": "Unique identifier" },
			"chip_yaml": { "type": "string", "description": "LabWired YAML specification" }
		},
		"required": ["peripheral_id", "chip_yaml"]
	}`
	w.Header().Set("Content-Type", "application/json")
	w.Write([]byte(schema))
}

// ── Background Worker ─────────────────────────────────────────────────────────

func (s *Server) worker(workerID int) {
	defer s.workersWG.Done()
	for job := range s.jobQueue {
		// Mark as running in DB.
		_ = s.store.UpdateRunStatus(job.ID, string(StatusRunning), 0, 0, "")

		ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
		result, err := s.orchestrator.RunSimulation(ctx, job.IRPath, job.ArtifactDir)
		cancel()

		if err != nil {
			log.Printf("[worker:%d] run %s error: %v", workerID, job.ID, err)
			_ = s.store.UpdateRunStatus(job.ID, string(StatusError), 0, 0, "")
		} else if result.Pass {
			_ = s.store.UpdateRunStatus(job.ID, string(StatusPass), result.AssertionsPassed, result.AssertionsTotal, job.ArtifactDir)
		} else {
			_ = s.store.UpdateRunStatus(job.ID, string(StatusFail), result.AssertionsPassed, result.AssertionsTotal, job.ArtifactDir)
		}

		s.jobs.Delete(job.ID)
		s.releaseWorkspaceSlot(job.WorkspaceID)
	}
}

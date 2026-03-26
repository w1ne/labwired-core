package api

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/gorilla/mux"
	"github.com/labwired/foundry-backend/internal/catalog"
	"github.com/labwired/foundry-backend/internal/db"
	"github.com/labwired/foundry-backend/internal/synthesis"
	"github.com/labwired/foundry-backend/internal/verification"
	stripe "github.com/stripe/stripe-go/v76"
	"github.com/stripe/stripe-go/v76/webhook"
)

// maxInputLen caps user-controlled strings before they reach synthesis or the LLM.
const maxInputLen = 32 * 1024 // 32 KB
const maxVerifyBodyBytes = int64(1 << 20)
const maxSynthesisBodyBytes = int64(256 << 10)
const maxIdempotencyKeyLen = 128

func sanitizeInput(s string) string {
	if len(s) > maxInputLen {
		return s[:maxInputLen]
	}
	return s
}

func normalizeSynthesisRequest(req synthesisAPIRequest) (normalizedSynthesisRequest, error) {
	req.Kind = strings.TrimSpace(req.Kind)
	req.PromotionMode = strings.TrimSpace(req.PromotionMode)
	req.ComponentName = strings.TrimSpace(req.ComponentName)
	req.Requirements = strings.TrimSpace(req.Requirements)
	req.DatasheetURL = strings.TrimSpace(req.DatasheetURL)
	for i := range req.DocumentationURLs {
		req.DocumentationURLs[i] = strings.TrimSpace(req.DocumentationURLs[i])
	}
	if req.Board != nil {
		req.Board.Vendor = strings.TrimSpace(req.Board.Vendor)
		req.Board.MarketingName = strings.TrimSpace(req.Board.MarketingName)
		req.Board.BoardID = strings.TrimSpace(req.Board.BoardID)
		req.Board.MCU = strings.TrimSpace(req.Board.MCU)
	}
	if req.Workload != nil {
		req.Workload.Type = strings.TrimSpace(req.Workload.Type)
		req.Workload.FirmwarePath = strings.TrimSpace(req.Workload.FirmwarePath)
		req.Workload.Example = strings.TrimSpace(req.Workload.Example)
	}
	if req.Constraints != nil {
		req.Constraints.BLEScope = strings.TrimSpace(req.Constraints.BLEScope)
	}
	req.DesiredCapabilities = nonEmptyStrings(req.DesiredCapabilities)
	req.ValidationTargets = nonEmptyStrings(req.ValidationTargets)

	normalized := normalizedSynthesisRequest{
		Kind:              req.Kind,
		DryRun:            req.DryRun,
		PromotionMode:     req.PromotionMode,
		ComponentName:     req.ComponentName,
		Requirements:      req.Requirements,
		DatasheetURL:      req.DatasheetURL,
		DocumentationURLs: append([]string(nil), req.DocumentationURLs...),
		Original:          req,
	}
	if !isValidPromotionMode(normalized.PromotionMode) {
		return normalizedSynthesisRequest{}, fmt.Errorf("unsupported promotion_mode %q", normalized.PromotionMode)
	}

	if req.Kind == "" {
		if normalized.PromotionMode == "" {
			normalized.PromotionMode = "artifact_only"
		}
		if normalized.DryRun {
			normalized.PromotionMode = "artifact_only"
		}
		if normalized.ComponentName == "" || normalized.Requirements == "" {
			return normalizedSynthesisRequest{}, fmt.Errorf("component_name and requirements are required")
		}
		return normalized, nil
	}

	switch req.Kind {
	case "board_onboarding":
		if normalized.PromotionMode == "" {
			normalized.PromotionMode = "apply_to_repo"
		}
		if normalized.DryRun {
			normalized.PromotionMode = "artifact_only"
		}
		if req.Board == nil {
			return normalizedSynthesisRequest{}, fmt.Errorf("board is required for kind=board_onboarding")
		}
		if req.Board.MarketingName == "" && req.Board.BoardID == "" {
			return normalizedSynthesisRequest{}, fmt.Errorf("board.marketing_name or board.board_id is required for kind=board_onboarding")
		}
		if len(req.DesiredCapabilities) == 0 {
			return normalizedSynthesisRequest{}, fmt.Errorf("desired_capabilities is required for kind=board_onboarding")
		}
		if missing := missingBoardDocs(req); len(missing) > 0 {
			return normalizedSynthesisRequest{}, fmt.Errorf("insufficient docs for kind=board_onboarding: missing %s", strings.Join(missing, ", "))
		}
		normalized.ComponentName = buildBoardComponentName(req)
		normalized.Requirements = buildBoardRequirements(req)
		return normalized, nil
	case "peripheral_model_ingest":
		if normalized.PromotionMode == "" {
			normalized.PromotionMode = "artifact_only"
		}
		if normalized.DryRun {
			normalized.PromotionMode = "artifact_only"
		}
		if normalized.PromotionMode != "artifact_only" {
			return normalizedSynthesisRequest{}, fmt.Errorf("promotion_mode %q is unsupported for kind=peripheral_model_ingest", normalized.PromotionMode)
		}
		if normalized.ComponentName == "" {
			return normalizedSynthesisRequest{}, fmt.Errorf("component_name is required for kind=peripheral_model_ingest")
		}
		if normalized.Requirements == "" {
			return normalizedSynthesisRequest{}, fmt.Errorf("requirements are required for kind=peripheral_model_ingest")
		}
		if strings.TrimSpace(normalized.DatasheetURL) == "" {
			return normalizedSynthesisRequest{}, fmt.Errorf("datasheet_url is required for kind=peripheral_model_ingest")
		}
		return normalized, nil
	default:
		return normalizedSynthesisRequest{}, fmt.Errorf("unsupported synthesis kind %q", req.Kind)
	}
}

func missingBoardDocs(req synthesisAPIRequest) []string {
	missing := []string{}
	if strings.TrimSpace(req.DatasheetURL) == "" {
		missing = append(missing, "datasheet_url")
	}

	hasBoardDoc := false
	hasReferenceManual := false
	hasExampleDoc := false
	for _, raw := range req.DocumentationURLs {
		doc := strings.ToLower(strings.TrimSpace(raw))
		switch {
		case strings.Contains(doc, "schematic"), strings.Contains(doc, "board"), strings.Contains(doc, "manual"), strings.Contains(doc, "um"):
			hasBoardDoc = true
		}
		switch {
		case strings.Contains(doc, "reference"), strings.Contains(doc, "refman"), strings.Contains(doc, "/rm"), strings.Contains(doc, "programming"):
			hasReferenceManual = true
		}
		switch {
		case strings.Contains(doc, "example"), strings.Contains(doc, "examples"), strings.Contains(doc, "github"), strings.Contains(doc, "cube"), strings.Contains(doc, "sdk"):
			hasExampleDoc = true
		}
	}
	if !hasBoardDoc {
		missing = append(missing, "board manual or schematic in documentation_urls")
	}
	if !hasReferenceManual {
		missing = append(missing, "reference manual or programming reference in documentation_urls")
	}
	if boardNeedsVendorExample(req) && !hasExampleDoc {
		missing = append(missing, "vendor example or firmware reference in documentation_urls")
	}
	return missing
}

func boardNeedsVendorExample(req synthesisAPIRequest) bool {
	parts := []string{req.Requirements}
	parts = append(parts, req.DesiredCapabilities...)
	parts = append(parts, req.ValidationTargets...)
	if req.Constraints != nil {
		parts = append(parts, req.Constraints.BLEScope)
	}
	if req.Workload != nil {
		parts = append(parts, req.Workload.Type, req.Workload.Example)
	}
	lower := strings.ToLower(strings.Join(parts, " "))
	return strings.Contains(lower, "ble") || strings.Contains(lower, "example") || strings.Contains(lower, "firmware")
}

func isValidPromotionMode(mode string) bool {
	switch mode {
	case "", "artifact_only", "apply_to_repo", "commit_to_branch", "open_pr":
		return true
	default:
		return false
	}
}

func buildBoardComponentName(req synthesisAPIRequest) string {
	if req.Board == nil {
		return req.ComponentName
	}
	parts := []string{}
	if req.Board.BoardID != "" {
		parts = append(parts, req.Board.BoardID)
	}
	if req.Board.MarketingName != "" {
		parts = append(parts, req.Board.MarketingName)
	}
	if len(parts) == 0 && req.Board.MCU != "" {
		parts = append(parts, req.Board.MCU)
	}
	if len(parts) == 0 {
		return req.ComponentName
	}
	return strings.Join(parts, " / ") + " board onboarding proof"
}

func buildBoardRequirements(req synthesisAPIRequest) string {
	lines := []string{"Board onboarding contract:"}
	if req.Board != nil {
		if req.Board.MCU != "" {
			lines = append(lines, "MCU: "+req.Board.MCU)
		}
		if req.Board.Vendor != "" {
			lines = append(lines, "Vendor: "+req.Board.Vendor)
		}
	}
	if len(req.DesiredCapabilities) > 0 {
		lines = append(lines, "Desired capabilities: "+strings.Join(req.DesiredCapabilities, ", "))
	}
	if len(req.ValidationTargets) > 0 {
		lines = append(lines, "Validation targets: "+strings.Join(req.ValidationTargets, ", "))
	}
	if req.Workload != nil {
		if req.Workload.Type != "" {
			lines = append(lines, "Workload type: "+req.Workload.Type)
		}
		if req.Workload.Example != "" {
			lines = append(lines, "Preferred example: "+req.Workload.Example)
		}
		if req.Workload.FirmwarePath != "" {
			lines = append(lines, "Firmware path: "+req.Workload.FirmwarePath)
		}
	}
	if req.Constraints != nil {
		if req.Constraints.BLEScope != "" {
			lines = append(lines, "BLE scope: "+req.Constraints.BLEScope)
		}
		if req.Constraints.MustWriteRepoAssets {
			lines = append(lines, "Must write repo assets: true")
		}
		if req.Constraints.MustRunE2EValidation {
			lines = append(lines, "Must run e2e validation: true")
		}
	}
	if req.Requirements != "" {
		lines = append(lines, "Additional notes: "+req.Requirements)
	}
	return strings.Join(lines, "\n")
}

func nonEmptyStrings(values []string) []string {
	out := make([]string, 0, len(values))
	for _, value := range values {
		if trimmed := strings.TrimSpace(value); trimmed != "" {
			out = append(out, trimmed)
		}
	}
	return out
}

func toSynthesisBoardSpec(spec *synthesisBoardSpec) *synthesis.BoardSpec {
	if spec == nil {
		return nil
	}
	return &synthesis.BoardSpec{
		Vendor:        spec.Vendor,
		MarketingName: spec.MarketingName,
		BoardID:       spec.BoardID,
		MCU:           spec.MCU,
	}
}

func toSynthesisWorkloadSpec(spec *synthesisWorkloadSpec) *synthesis.WorkloadSpec {
	if spec == nil {
		return nil
	}
	return &synthesis.WorkloadSpec{
		Type:         spec.Type,
		FirmwarePath: spec.FirmwarePath,
		Example:      spec.Example,
	}
}

func toSynthesisConstraintSpec(spec *synthesisConstraintSpec) *synthesis.ConstraintSpec {
	if spec == nil {
		return nil
	}
	return &synthesis.ConstraintSpec{
		BLEScope:             spec.BLEScope,
		MustWriteRepoAssets:  spec.MustWriteRepoAssets,
		MustRunE2EValidation: spec.MustRunE2EValidation,
	}
}

func getIdempotencyKey(r *http.Request) string {
	return strings.TrimSpace(r.Header.Get("Idempotency-Key"))
}

func isValidIdempotencyKey(key string) bool {
	if key == "" || len(key) > maxIdempotencyKeyLen {
		return false
	}
	for i := 0; i < len(key); i++ {
		c := key[i]
		if (c >= 'a' && c <= 'z') ||
			(c >= 'A' && c <= 'Z') ||
			(c >= '0' && c <= '9') ||
			c == '-' || c == '_' || c == '.' || c == ':' {
			continue
		}
		return false
	}
	return true
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
	ID                  string
	WorkspaceID         string
	Type                JobType
	IRPath              string // temp file containing the submitted YAML/JSON (Verify)
	ArtifactDir         string // directory where output files will be written
	DatasheetURL        string // (Synthesize) URL to parse
	DocumentationURLs   []string
	LabWiredPath        string
	RepoRootDir         string
	SynthesisKind       string
	DryRun              bool
	PromotionMode       string
	Board               *synthesis.BoardSpec
	DesiredCapabilities []string
	ValidationTargets   []string
	Workload            *synthesis.WorkloadSpec
	Constraints         *synthesis.ConstraintSpec
	ComponentName       string // (Synthesize) Name of the component
	Requirements        string // (Synthesize)
}

type synthesisBoardSpec struct {
	Vendor        string `json:"vendor,omitempty"`
	MarketingName string `json:"marketing_name,omitempty"`
	BoardID       string `json:"board_id,omitempty"`
	MCU           string `json:"mcu,omitempty"`
}

type synthesisWorkloadSpec struct {
	Type         string `json:"type,omitempty"`
	FirmwarePath string `json:"firmware_path,omitempty"`
	Example      string `json:"example,omitempty"`
}

type synthesisConstraintSpec struct {
	BLEScope             string `json:"ble_scope,omitempty"`
	MustWriteRepoAssets  bool   `json:"must_write_repo_assets,omitempty"`
	MustRunE2EValidation bool   `json:"must_run_e2e_validation,omitempty"`
}

type synthesisAPIRequest struct {
	Kind                string                   `json:"kind,omitempty"`
	DryRun              bool                     `json:"dry_run,omitempty"`
	PromotionMode       string                   `json:"promotion_mode,omitempty"`
	ComponentName       string                   `json:"component_name,omitempty"`
	Requirements        string                   `json:"requirements,omitempty"`
	DatasheetURL        string                   `json:"datasheet_url,omitempty"`
	DocumentationURLs   []string                 `json:"documentation_urls,omitempty"`
	Board               *synthesisBoardSpec      `json:"board,omitempty"`
	DesiredCapabilities []string                 `json:"desired_capabilities,omitempty"`
	ValidationTargets   []string                 `json:"validation_targets,omitempty"`
	Workload            *synthesisWorkloadSpec   `json:"workload,omitempty"`
	Constraints         *synthesisConstraintSpec `json:"constraints,omitempty"`
}

type normalizedSynthesisRequest struct {
	Kind              string
	DryRun            bool
	PromotionMode     string
	ComponentName     string
	Requirements      string
	DatasheetURL      string
	DocumentationURLs []string
	Original          synthesisAPIRequest
}

type Server struct {
	router         *mux.Router
	jobs           sync.Map // run_id -> *Job (in-memory while queued/running)
	workerCount    int
	workersWG      sync.WaitGroup
	orchestrator   *verification.Orchestrator
	store          *db.Store
	catalog        *catalog.Manager
	artifactsDir   string
	dataDir        string
	repoRootDir    string
	coreConfigsDir string
	clerkSecretKey string

	scheduleMu         sync.Mutex
	scheduleCond       *sync.Cond
	pendingByWorkspace map[string][]*Job
	workspaceOrder     []string
	nextWorkspaceIdx   int
	pendingJobs        int
	maxPendingJobs     int
	shuttingDown       bool

	maxInflightPerWorkspace int
	inflightMu              sync.Mutex
	inflightByWorkspace     map[string]int

	artifactRetentionDays    int
	runMetadataRetentionDays int
	cleanupInterval          time.Duration
	workerLeaseTimeout       time.Duration
	workerHeartbeatInterval  time.Duration
	maxRunAttempts           int
	rateLimitPerAPIKey       int
	rateLimitPerWorkspace    int
	rateLimitWindow          time.Duration
	cleanupStopCh            chan struct{}

	metrics        serverMetrics
	allowedOrigins []string
}

type ServerOptions struct {
	WorkerCount              int
	MaxInflightPerWorkspace  int
	ArtifactRetentionDays    int
	RunMetadataRetentionDays int
	CleanupInterval          time.Duration
	WorkerLeaseTimeout       time.Duration
	WorkerHeartbeatInterval  time.Duration
	MaxRunAttempts           int
	RateLimitPerAPIKey       int
	RateLimitPerWorkspace    int
	RateLimitWindow          time.Duration
	ClerkSecretKey           string
	RepoRootDir              string
	CoreConfigsDir           string
}

func DefaultServerOptions() ServerOptions {
	return ServerOptions{
		WorkerCount:              4,
		MaxInflightPerWorkspace:  8,
		ArtifactRetentionDays:    14,
		RunMetadataRetentionDays: 90,
		CleanupInterval:          time.Hour,
		WorkerLeaseTimeout:       45 * time.Second,
		WorkerHeartbeatInterval:  10 * time.Second,
		MaxRunAttempts:           3,
		RateLimitPerAPIKey:       120,
		RateLimitPerWorkspace:    600,
		RateLimitWindow:          time.Minute,
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
	LeaseRequeues         atomic.Int64
	AttemptsExhausted     atomic.Int64
	IdempotencyRowsPruned atomic.Int64
	RateLimitRejected     atomic.Int64
}

type enqueueResult int

const (
	enqueueOK enqueueResult = iota
	enqueueShuttingDown
	enqueueQueueFull
)

func NewServer(orch *verification.Orchestrator, store *db.Store, cat *catalog.Manager, artifactsDir, dataDir string, opts ServerOptions) *Server {
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
	if opts.WorkerLeaseTimeout <= 0 {
		opts.WorkerLeaseTimeout = 45 * time.Second
	}
	if opts.WorkerHeartbeatInterval <= 0 {
		opts.WorkerHeartbeatInterval = 10 * time.Second
	}
	if opts.WorkerHeartbeatInterval >= opts.WorkerLeaseTimeout {
		opts.WorkerHeartbeatInterval = opts.WorkerLeaseTimeout / 2
		if opts.WorkerHeartbeatInterval <= 0 {
			opts.WorkerHeartbeatInterval = 1 * time.Second
		}
	}
	if opts.MaxRunAttempts <= 0 {
		opts.MaxRunAttempts = 3
	}
	if opts.RateLimitPerAPIKey <= 0 {
		opts.RateLimitPerAPIKey = 120
	}
	if opts.RateLimitPerWorkspace <= 0 {
		opts.RateLimitPerWorkspace = 600
	}
	if opts.RateLimitWindow <= 0 {
		opts.RateLimitWindow = time.Minute
	}

	s := &Server{
		router:                   mux.NewRouter(),
		workerCount:              opts.WorkerCount,
		orchestrator:             orch,
		store:                    store,
		catalog:                  cat,
		artifactsDir:             artifactsDir,
		dataDir:                  dataDir,
		repoRootDir:              opts.RepoRootDir,
		coreConfigsDir:           opts.CoreConfigsDir,
		pendingByWorkspace:       make(map[string][]*Job),
		maxPendingJobs:           100,
		maxInflightPerWorkspace:  opts.MaxInflightPerWorkspace,
		inflightByWorkspace:      make(map[string]int),
		artifactRetentionDays:    opts.ArtifactRetentionDays,
		runMetadataRetentionDays: opts.RunMetadataRetentionDays,
		cleanupInterval:          opts.CleanupInterval,
		workerLeaseTimeout:       opts.WorkerLeaseTimeout,
		workerHeartbeatInterval:  opts.WorkerHeartbeatInterval,
		maxRunAttempts:           opts.MaxRunAttempts,
		rateLimitPerAPIKey:       opts.RateLimitPerAPIKey,
		rateLimitPerWorkspace:    opts.RateLimitPerWorkspace,
		rateLimitWindow:          opts.RateLimitWindow,
		cleanupStopCh:            make(chan struct{}),
		clerkSecretKey:           opts.ClerkSecretKey,
		allowedOrigins:           parseAllowedOrigins(),
	}
	s.scheduleCond = sync.NewCond(&s.scheduleMu)
	s.routes()
	s.recoverPendingRuns()
	for i := 0; i < s.workerCount; i++ {
		s.workersWG.Add(1)
		go s.worker(i)
	}
	s.workersWG.Add(1)
	go s.cleanupLoop()
	s.workersWG.Add(1)
	go s.leaseRecoveryLoop()
	return s
}

// parseAllowedOrigins reads CORS_ALLOWED_ORIGINS env var (comma-separated).
// Falls back to ["*"] if unset (dev mode). Set to your frontend domain in production.
func parseAllowedOrigins() []string {
	raw := os.Getenv("CORS_ALLOWED_ORIGINS")
	if raw == "" {
		return []string{"*"}
	}
	var origins []string
	for _, o := range strings.Split(raw, ",") {
		o = strings.TrimSpace(o)
		if o != "" {
			origins = append(origins, o)
		}
	}
	if len(origins) == 0 {
		return []string{"*"}
	}
	return origins
}

func (s *Server) recoverPendingRuns() {
	if n, err := s.store.FailExhaustedQueuedRuns(s.maxRunAttempts, "max attempts exhausted"); err != nil {
		log.Printf("[recovery] failed to mark exhausted queued runs: %v", err)
	} else if n > 0 {
		s.metrics.AttemptsExhausted.Add(n)
	}

	cutoff := time.Now().Add(-s.workerLeaseTimeout)
	if n, err := s.store.RequeueStaleRunningRuns(cutoff, "worker lease expired during restart recovery"); err != nil {
		log.Printf("[recovery] failed to requeue stale running runs: %v", err)
	} else if n > 0 {
		log.Printf("[recovery] requeued %d stale running run(s)", n)
		s.metrics.LeaseRequeues.Add(n)
	}

	runs, err := s.store.ListRecoverableRuns()
	if err != nil {
		log.Printf("[recovery] failed to list recoverable runs: %v", err)
		return
	}
	for _, r := range runs {
		if !s.tryAcquireWorkspaceSlot(r.WorkspaceID) {
			continue
		}

		job, err := s.recoverJobFromRun(r)
		if err != nil {
			log.Printf("[recovery] failed to reconstruct run %s: %v", r.RunID, err)
			s.releaseWorkspaceSlot(r.WorkspaceID)
			_ = s.store.UpdateRunStatus(r.RunID, string(StatusError), 0, 0, "")
			continue
		}

		if s.tryEnqueueJob(job) == enqueueOK {
			s.jobs.Store(job.ID, job)
		} else {
			s.releaseWorkspaceSlot(r.WorkspaceID)
			_ = s.store.UpdateRunStatus(r.RunID, string(StatusError), 0, 0, "")
		}
	}
	s.refillQueueFromDB(200)
}

func (s *Server) leaseRecoveryLoop() {
	defer s.workersWG.Done()
	ticker := time.NewTicker(s.workerHeartbeatInterval)
	defer ticker.Stop()

	for {
		select {
		case <-s.cleanupStopCh:
			return
		case <-ticker.C:
			if n, err := s.store.FailExhaustedQueuedRuns(s.maxRunAttempts, "max attempts exhausted"); err != nil {
				log.Printf("[lease] fail exhausted queued runs failed: %v", err)
			} else if n > 0 {
				s.metrics.AttemptsExhausted.Add(n)
			}

			cutoff := time.Now().Add(-s.workerLeaseTimeout)
			n, err := s.store.RequeueStaleRunningRuns(cutoff, "worker lease expired")
			if err != nil {
				log.Printf("[lease] stale-claim requeue failed: %v", err)
				continue
			}
			if n > 0 {
				log.Printf("[lease] requeued %d stale running run(s)", n)
				s.metrics.LeaseRequeues.Add(n)
			}
			s.refillQueueFromDB(200)
		}
	}
}

func (s *Server) refillQueueFromDB(limit int) {
	queued, err := s.store.ListQueuedRuns(limit)
	if err != nil {
		log.Printf("[reconcile] failed listing queued runs: %v", err)
		return
	}
	for _, r := range queued {
		if _, exists := s.jobs.Load(r.RunID); exists {
			continue
		}
		if !s.tryAcquireWorkspaceSlot(r.WorkspaceID) {
			// Respect per-workspace inflight cap; try on next reconcile tick.
			continue
		}
		job, err := s.recoverJobFromRun(r)
		if err != nil {
			log.Printf("[reconcile] failed to reconstruct queued run %s: %v", r.RunID, err)
			s.releaseWorkspaceSlot(r.WorkspaceID)
			_ = s.store.CompleteClaimedRun(r.RunID, string(StatusError), 0, 0, "", "queued run reconstruction failed")
			continue
		}

		switch s.tryEnqueueJob(job) {
		case enqueueOK:
			s.jobs.Store(job.ID, job)
		case enqueueQueueFull:
			s.releaseWorkspaceSlot(r.WorkspaceID)
			return
		case enqueueShuttingDown:
			s.releaseWorkspaceSlot(r.WorkspaceID)
			return
		}
	}
}

func (s *Server) recoverJobFromRun(r db.RunRecord) (*Job, error) {
	artifactDir := filepath.Join(s.artifactsDir, r.RunID)
	switch {
	case strings.HasPrefix(r.RunID, "synth-"):
		reqPath := filepath.Join(artifactDir, "synth_request.json")
		data, err := os.ReadFile(reqPath)
		if err != nil {
			return nil, err
		}
		var req struct {
			ComponentName string `json:"component_name"`
			Requirements  string `json:"requirements"`
			DatasheetURL  string `json:"datasheet_url,omitempty"`
		}
		if err := json.Unmarshal(data, &req); err != nil {
			return nil, err
		}
		return &Job{
			ID:            r.RunID,
			WorkspaceID:   r.WorkspaceID,
			Type:          JobTypeSynthesize,
			ArtifactDir:   artifactDir,
			DatasheetURL:  req.DatasheetURL,
			ComponentName: req.ComponentName,
			Requirements:  req.Requirements,
		}, nil
	default:
		irPath := filepath.Join(artifactDir, "input.yaml")
		if _, err := os.Stat(irPath); err != nil {
			return nil, err
		}
		return &Job{
			ID:          r.RunID,
			WorkspaceID: r.WorkspaceID,
			Type:        JobTypeVerify,
			IRPath:      irPath,
			ArtifactDir: artifactDir,
		}, nil
	}
}

func (s *Server) tryEnqueueJob(job *Job) enqueueResult {
	s.scheduleMu.Lock()
	defer s.scheduleMu.Unlock()

	if s.shuttingDown {
		return enqueueShuttingDown
	}
	if s.pendingJobs >= s.maxPendingJobs {
		return enqueueQueueFull
	}
	if len(s.pendingByWorkspace[job.WorkspaceID]) == 0 {
		s.workspaceOrder = append(s.workspaceOrder, job.WorkspaceID)
	}
	s.pendingByWorkspace[job.WorkspaceID] = append(s.pendingByWorkspace[job.WorkspaceID], job)
	s.pendingJobs++
	s.scheduleCond.Signal()
	return enqueueOK
}

func (s *Server) dequeueJob() (*Job, bool) {
	s.scheduleMu.Lock()
	defer s.scheduleMu.Unlock()

	for s.pendingJobs == 0 && !s.shuttingDown {
		s.scheduleCond.Wait()
	}
	if s.pendingJobs == 0 && s.shuttingDown {
		return nil, false
	}
	if len(s.workspaceOrder) == 0 {
		return nil, false
	}
	if s.nextWorkspaceIdx >= len(s.workspaceOrder) {
		s.nextWorkspaceIdx = 0
	}

	start := s.nextWorkspaceIdx
	for scanned := 0; scanned < len(s.workspaceOrder); scanned++ {
		idx := (start + scanned) % len(s.workspaceOrder)
		ws := s.workspaceOrder[idx]
		q := s.pendingByWorkspace[ws]
		if len(q) == 0 {
			s.removeWorkspaceAtLocked(idx)
			if len(s.workspaceOrder) == 0 {
				return nil, s.pendingJobs > 0
			}
			if idx < start && start > 0 {
				start--
			}
			scanned--
			continue
		}

		job := q[0]
		q = q[1:]
		s.pendingJobs--
		if len(q) == 0 {
			delete(s.pendingByWorkspace, ws)
			s.removeWorkspaceAtLocked(idx)
			if len(s.workspaceOrder) == 0 {
				s.nextWorkspaceIdx = 0
			} else if idx >= len(s.workspaceOrder) {
				s.nextWorkspaceIdx = 0
			} else {
				s.nextWorkspaceIdx = idx
			}
		} else {
			s.pendingByWorkspace[ws] = q
			s.nextWorkspaceIdx = (idx + 1) % len(s.workspaceOrder)
		}
		return job, true
	}
	return nil, false
}

func (s *Server) removeWorkspaceAtLocked(idx int) {
	s.workspaceOrder = append(s.workspaceOrder[:idx], s.workspaceOrder[idx+1:]...)
}

func (s *Server) routes() {
	// Public endpoints
	s.router.HandleFunc("/v1/catalog", s.handleListCatalog).Methods("GET")
	s.router.HandleFunc("/v1/catalog/{id:.*}/source", s.handleGetCatalogSource).Methods("GET")
	s.router.HandleFunc("/v1/catalog/{id:.*}", s.handleGetCatalogAsset).Methods("GET")
	s.router.HandleFunc("/v1/info", s.handleInfo).Methods("GET")
	s.router.HandleFunc("/v1/hardware", s.handleListHardware).Methods("GET")
	s.router.HandleFunc("/v1/health", s.handleHealth).Methods("GET")
	s.router.HandleFunc("/v1/schema/synthesis", s.handleSchemaSynthesis).Methods("GET")
	s.router.HandleFunc("/v1/openapi.yaml", func(w http.ResponseWriter, r *http.Request) {
		http.ServeFile(w, r, "static/openapi.yaml")
	}).Methods("GET")
	s.router.PathPrefix("/v1/docs").Handler(http.StripPrefix("/v1/docs", http.FileServer(http.Dir("static"))))
	s.router.PathPrefix("/data/").Handler(http.StripPrefix("/data/", http.FileServer(http.Dir(s.dataDir))))
	s.router.PathPrefix("/v1/catalog/traces/").Handler(http.StripPrefix("/v1/catalog/traces/", http.FileServer(http.Dir("core/configs/onboarding/traces"))))

	// Stripe webhook (no API key auth — verified by signature)
	s.router.HandleFunc("/v1/webhooks/stripe", s.handleStripeWebhook).Methods("POST")

	// Protected VaaS Routes (Requires API Key)
	protected := s.router.PathPrefix("/v1").Subrouter()
	protected.Use(s.authMiddleware)
	protected.Use(s.rateLimitMiddleware)

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

	// Account routes (Clerk JWT auth — personal cabinet)
	if s.clerkSecretKey != "" {
		account := s.router.PathPrefix("/v1/account").Subrouter()
		account.Use(s.clerkAuthMiddleware)
		account.Use(s.accountRateLimitMiddleware)
		account.HandleFunc("/usage", s.handleAccountUsage).Methods("GET")
		account.HandleFunc("/runs", s.handleAccountRuns).Methods("GET")
		account.HandleFunc("/runs/{run_id}", s.handleAccountRun).Methods("GET")
		account.HandleFunc("/runs/{run_id}/artifacts/{file}", s.handleAccountRunArtifact).Methods("GET")
		account.HandleFunc("/estimate", s.handleAccountEstimate).Methods("POST")
		account.HandleFunc("/synthesize", s.handleAccountSynthesize).Methods("POST")
		account.HandleFunc("/models/verify", s.handleAccountVerifyModel).Methods("POST")
		account.HandleFunc("/systems/verify", s.handleAccountVerifySystem).Methods("POST")
		account.HandleFunc("/keys", s.handleListAccountKeys).Methods("GET")
		account.HandleFunc("/keys", s.handleCreateAccountKey).Methods("POST")
		account.HandleFunc("/keys/{key_id}", s.handleRevokeAccountKey).Methods("DELETE")
		account.HandleFunc("/quickstart", s.handleAccountQuickstart).Methods("GET")
		account.HandleFunc("/usage/breakdown", s.handleAccountUsageBreakdown).Methods("GET")

		// Organization management
		account.HandleFunc("/org", s.handleCreateOrg).Methods("POST")
		account.HandleFunc("/org", s.handleListOrgs).Methods("GET")

		// Per-org routes with RBAC
		orgRouter := account.PathPrefix("/org/{org_id}").Subrouter()
		orgRouter.Handle("/members", s.requireRole("viewer")(http.HandlerFunc(s.handleListOrgMembers))).Methods("GET")
		orgRouter.Handle("/members", s.requireRole("admin")(http.HandlerFunc(s.handleAddOrgMember))).Methods("POST")
		orgRouter.Handle("/audit", s.requireRole("admin")(http.HandlerFunc(s.handleOrgAuditLog))).Methods("GET")
	}
}

func (s *Server) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	s.panicRecoveryMiddleware(s.corsMiddleware(s.router)).ServeHTTP(w, r)
}

func (s *Server) panicRecoveryMiddleware(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		defer func() {
			if rec := recover(); rec != nil {
				log.Printf("[panic] recovered panic for %s %s: %v", r.Method, r.URL.Path, rec)
				sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "An unexpected internal error occurred.", "Retry later.")
			}
		}()
		next.ServeHTTP(w, r)
	})
}

// Shutdown stops accepting new jobs, drains queued work, and waits for workers.
func (s *Server) Shutdown(ctx context.Context) error {
	s.scheduleMu.Lock()
	if !s.shuttingDown {
		s.shuttingDown = true
		close(s.cleanupStopCh)
		s.scheduleCond.Broadcast()
	}
	s.scheduleMu.Unlock()

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
	idemDeleted, err := s.store.PruneIdempotencyRequestsBefore(metadataCutoff)
	if err != nil {
		return err
	}
	if idemDeleted > 0 {
		s.metrics.IdempotencyRowsPruned.Add(idemDeleted)
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

func (s *Server) handleGetCatalogSource(w http.ResponseWriter, r *http.Request) {
	id := mux.Vars(r)["id"]
	// Strip the trailing "/source" that the router leaves in the wildcard.
	id = strings.TrimSuffix(id, "/source")
	asset, ok := s.catalog.Get(id)
	if !ok {
		sendError(w, http.StatusNotFound, "ASSET_NOT_FOUND", "The requested asset ID does not exist in the catalog.", "Check /v1/catalog for a list of valid asset IDs.")
		return
	}
	if s.coreConfigsDir == "" || strings.TrimSpace(asset.SourceRef) == "" {
		sendError(w, http.StatusNotFound, "SOURCE_NOT_AVAILABLE", "No source YAML is available for this asset.", "")
		return
	}
	// Resolve safely — reject path traversal.
	cleaned := filepath.Clean(asset.SourceRef)
	if strings.Contains(cleaned, "..") {
		sendError(w, http.StatusBadRequest, "INVALID_SOURCE_REF", "Source reference contains invalid path.", "")
		return
	}
	fullPath := filepath.Join(s.coreConfigsDir, cleaned)
	data, err := os.ReadFile(fullPath)
	if err != nil {
		sendError(w, http.StatusNotFound, "SOURCE_NOT_AVAILABLE", "Source file not found on disk.", "")
		return
	}
	w.Header().Set("Content-Type", "text/yaml; charset=utf-8")
	w.Write(data)
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
	assets := s.catalog.List()
	items := make([]db.HardwareItem, 0, len(assets))
	for _, a := range assets {
		tier := 2
		if a.Verified && a.PassRate >= 100 {
			tier = 1
		}
		replPath := a.SourceRef
		if replPath == "" {
			replPath = a.ID
		}
		items = append(items, db.HardwareItem{
			ID:       a.ID,
			Name:     a.Name,
			Type:     "board",
			ReplPath: replPath,
			Tier:     tier,
		})
	}
	sort.Slice(items, func(i, j int) bool {
		if items[i].Tier != items[j].Tier {
			return items[i].Tier < items[j].Tier
		}
		if items[i].Type != items[j].Type {
			return items[i].Type < items[j].Type
		}
		return items[i].Name < items[j].Name
	})

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
	s.scheduleMu.Lock()
	queueLen := s.pendingJobs
	queueCap := s.maxPendingJobs
	scheduleShuttingDown := s.shuttingDown
	s.scheduleMu.Unlock()
	if queueCap == queueLen && queueCap > 0 {
		workerStatus = "saturated"
		// Not necessarily unhealthy, but worth reporting
	}
	components["worker"] = map[string]interface{}{
		"status": workerStatus,
		"queue": map[string]int{
			"length":   queueLen,
			"capacity": queueCap,
		},
		"shutting_down":              scheduleShuttingDown,
		"max_inflight_per_workspace": s.maxInflightPerWorkspace,
	}
	// Internal metrics only exposed to authenticated callers
	showMetrics := false
	if authHeader := r.Header.Get("Authorization"); strings.HasPrefix(authHeader, "Bearer ") {
		key := strings.TrimSpace(strings.TrimPrefix(authHeader, "Bearer "))
		if _, err := s.store.ValidateKey(key); err == nil {
			showMetrics = true
		}
	}
	if showMetrics {
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
			"lease_requeues":                s.metrics.LeaseRequeues.Load(),
			"attempts_exhausted":            s.metrics.AttemptsExhausted.Load(),
			"idempotency_rows_pruned":       s.metrics.IdempotencyRowsPruned.Load(),
			"rate_limit_rejected":           s.metrics.RateLimitRejected.Load(),
		}
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

func calculateSynthesisCost(kind, componentName, requirements string, desiredCapabilities, validationTargets []string) int {
	if kind == "board_onboarding" {
		cost := 1200
		cost += 50 * len(nonEmptyStrings(desiredCapabilities))
		cost += 75 * len(nonEmptyStrings(validationTargets))
		return cost
	}
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
	r.Body = http.MaxBytesReader(w, r.Body, maxSynthesisBodyBytes)
	var req synthesisAPIRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		if strings.Contains(err.Error(), "http: request body too large") {
			sendError(w, http.StatusRequestEntityTooLarge, "PAYLOAD_TOO_LARGE", "Request body exceeds size limit.", "Reduce request size and retry.")
			return
		}
		sendError(w, http.StatusBadRequest, "INVALID_JSON", "The request body could not be parsed as valid JSON.", "Verify the JSON syntax.")
		return
	}
	normalized, err := normalizeSynthesisRequest(req)
	if err != nil {
		sendError(w, http.StatusBadRequest, "MISSING_REQUIRED_FIELDS", err.Error(), "")
		return
	}
	normalized.ComponentName = sanitizeInput(normalized.ComponentName)
	normalized.Requirements = sanitizeInput(normalized.Requirements)

	cost := calculateSynthesisCost(normalized.Kind, normalized.ComponentName, normalized.Requirements, normalized.Original.DesiredCapabilities, normalized.Original.ValidationTargets)
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(map[string]interface{}{
		"kind":                normalized.Kind,
		"dry_run":             normalized.DryRun,
		"promotion_mode":      normalized.PromotionMode,
		"component_name":      normalized.ComponentName,
		"estimated_cost_runs": cost,
		"message":             fmt.Sprintf("Synthesizing %s will cost approximately %d runs.", normalized.ComponentName, cost),
	})
}

// ── Synthesize ────────────────────────────────────────────────────────────────

func (s *Server) handleSynthesize(w http.ResponseWriter, r *http.Request) {
	r.Body = http.MaxBytesReader(w, r.Body, maxSynthesisBodyBytes)
	var req synthesisAPIRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		if strings.Contains(err.Error(), "http: request body too large") {
			sendError(w, http.StatusRequestEntityTooLarge, "PAYLOAD_TOO_LARGE", "Request body exceeds size limit.", "Reduce request size and retry.")
			return
		}
		sendError(w, http.StatusBadRequest, "INVALID_JSON", "The request body could not be parsed as valid JSON.", "Verify the JSON syntax.")
		return
	}
	normalized, err := normalizeSynthesisRequest(req)
	if err != nil {
		sendError(w, http.StatusBadRequest, "MISSING_REQUIRED_FIELDS", err.Error(), "")
		return
	}
	normalized.ComponentName = sanitizeInput(normalized.ComponentName)
	normalized.Requirements = sanitizeInput(normalized.Requirements)

	cost := calculateSynthesisCost(normalized.Kind, normalized.ComponentName, normalized.Requirements, normalized.Original.DesiredCapabilities, normalized.Original.ValidationTargets)
	jobID := fmt.Sprintf("synth-%d", time.Now().UnixNano())
	primaryRunID := fmt.Sprintf("%s-%d", jobID, 0)
	apiKey, ok := apiKeyFromContext(r.Context())
	if !ok {
		sendError(w, http.StatusUnauthorized, "UNAUTHORIZED", "API key context missing.", "Ensure auth middleware is enabled.")
		return
	}
	idempotencyKey := getIdempotencyKey(r)
	idemStarted := false
	idemCompleted := false
	if idempotencyKey != "" {
		if !isValidIdempotencyKey(idempotencyKey) {
			sendError(w, http.StatusBadRequest, "INVALID_IDEMPOTENCY_KEY", "Idempotency key format is invalid.", "Use 1-128 characters from [A-Za-z0-9-_.:].")
			return
		}
		isNew, existing, err := s.store.BeginIdempotencyRequest(apiKey.WorkspaceID, "/v1/synthesize", idempotencyKey)
		if err != nil {
			sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to initialize idempotency request.", "Retry later.")
			return
		}
		if !isNew {
			if existing.StatusCode > 0 {
				w.Header().Set("Content-Type", "application/json")
				w.WriteHeader(existing.StatusCode)
				_, _ = w.Write([]byte(existing.ResponseBody))
				return
			}
			sendError(w, http.StatusConflict, "IDEMPOTENCY_IN_PROGRESS", "A request with this idempotency key is still in progress.", "Retry with a new key or wait for completion.")
			return
		}
		idemStarted = true
		defer func() {
			if idemStarted && !idemCompleted {
				_ = s.store.CancelPendingIdempotencyRequest(apiKey.WorkspaceID, "/v1/synthesize", idempotencyKey)
			}
		}()
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

	artifactDir := filepath.Join(s.artifactsDir, primaryRunID)
	if err := os.MkdirAll(artifactDir, 0755); err != nil {
		sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to create artifact directory.", "")
		return
	}
	reqPayload, err := json.Marshal(map[string]any{
		"request":    normalized.Original,
		"normalized": normalized,
	})
	if err != nil {
		sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to persist synthesis input.", "")
		return
	}
	if err := os.WriteFile(filepath.Join(artifactDir, "synth_request.json"), reqPayload, 0644); err != nil {
		sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to persist synthesis input.", "")
		return
	}

	job := &Job{
		ID:                  primaryRunID,
		WorkspaceID:         apiKey.WorkspaceID,
		Type:                JobTypeSynthesize,
		ArtifactDir:         artifactDir,
		DatasheetURL:        normalized.DatasheetURL,
		LabWiredPath:        s.orchestrator.LabwiredPath,
		RepoRootDir:         s.repoRootDir,
		SynthesisKind:       normalized.Kind,
		DryRun:              normalized.DryRun,
		PromotionMode:       normalized.PromotionMode,
		Board:               toSynthesisBoardSpec(normalized.Original.Board),
		DesiredCapabilities: append([]string(nil), normalized.Original.DesiredCapabilities...),
		ValidationTargets:   append([]string(nil), normalized.Original.ValidationTargets...),
		Workload:            toSynthesisWorkloadSpec(normalized.Original.Workload),
		Constraints:         toSynthesisConstraintSpec(normalized.Original.Constraints),
		ComponentName:       normalized.ComponentName,
		Requirements:        normalized.Requirements,
		DocumentationURLs:   normalized.DocumentationURLs,
	}

	// Reserve primary run with atomic quota+global-inflight check.
	runIDs := make([]string, 0, cost)
	for i := 0; i < cost; i++ {
		runIDs = append(runIDs, fmt.Sprintf("%s-%d", jobID, i))
	}
	if err := s.store.ReserveRunForWorkspaceWithInflight(primaryRunID, apiKey.WorkspaceID, string(StatusQueued), s.maxInflightPerWorkspace); err != nil {
		_ = s.store.UpdateRunStatus(primaryRunID, string(StatusError), 0, 0, "")
		if err == db.ErrInflightLimit {
			sendError(w, http.StatusTooManyRequests, "WORKSPACE_INFLIGHT_LIMIT", fmt.Sprintf("Workspace has reached max in-flight jobs (%d).", s.maxInflightPerWorkspace), "Wait for running jobs to complete before submitting more.")
			return
		}
		if err == db.ErrQuotaExceeded {
			sendError(w, http.StatusTooManyRequests, "QUOTA_EXCEEDED", "Workspace has exceeded its monthly run limit.", "Purchase more credits or upgrade your tier.")
			return
		}
		sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to reserve synthesis runs.", "Retry later.")
		return
	}
	if len(runIDs) > 1 {
		if err := s.store.ReserveRunsForWorkspace(runIDs[1:], apiKey.WorkspaceID, string(StatusPass)); err != nil {
			_ = s.store.SetRunBillable(primaryRunID, false)
			_ = s.store.UpdateRunStatus(primaryRunID, string(StatusError), 0, 0, "")
			if err == db.ErrQuotaExceeded {
				sendError(w, http.StatusTooManyRequests, "QUOTA_EXCEEDED", "Workspace has exceeded its monthly run limit.", "Purchase more credits or upgrade your tier.")
				return
			}
			sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to reserve synthesis runs.", "Retry later.")
			return
		}
	}

	switch s.tryEnqueueJob(job) {
	case enqueueShuttingDown:
		for _, rid := range runIDs {
			_ = s.store.SetRunBillable(rid, false)
			_ = s.store.UpdateRunStatus(rid, string(StatusError), 0, 0, "")
		}
		s.metrics.ShuttingDownRejected.Add(1)
		sendError(w, http.StatusServiceUnavailable, "SERVER_SHUTTING_DOWN", "Server is shutting down.", "")
		return
	case enqueueQueueFull:
		for _, rid := range runIDs {
			_ = s.store.SetRunBillable(rid, false)
			_ = s.store.UpdateRunStatus(rid, string(StatusError), 0, 0, "")
		}
		s.metrics.QueueFullRejected.Add(1)
		sendError(w, http.StatusServiceUnavailable, "QUEUE_FULL", "The queue is at capacity.", "Retry later.")
		return
	}

	s.jobs.Store(primaryRunID, job)
	enqueued = true
	resp := map[string]interface{}{
		"job_id":   jobID,
		"run_id":   primaryRunID,
		"status":   string(StatusQueued),
		"poll_url": fmt.Sprintf("/v1/runs/%s", primaryRunID),
		"message":  "Synthesis job queued. The AI engine is analyzing the datasheet and generating the model.",
	}
	w.Header().Set("Content-Type", "application/json")
	if idempotencyKey != "" {
		body, err := json.Marshal(resp)
		if err != nil {
			sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to persist idempotency response.", "Retry later.")
			return
		}
		if err := s.completeIdempotencyWithRetry(apiKey.WorkspaceID, "/v1/synthesize", idempotencyKey, primaryRunID, http.StatusAccepted, string(body)); err != nil {
			sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to persist idempotency response.", "Retry later.")
			return
		}
		idemCompleted = true
		w.WriteHeader(http.StatusAccepted)
		_, _ = w.Write(body)
		return
	}
	w.WriteHeader(http.StatusAccepted)
	json.NewEncoder(w).Encode(resp)
}

// ── Async Verify Handlers ─────────────────────────────────────────────────────

// submitJob writes the YAML body to a temp file, creates the DB row and enqueues the job.
func (s *Server) submitJob(w http.ResponseWriter, r *http.Request, prefix string, body []byte) {
	apiKey, ok := apiKeyFromContext(r.Context())
	if !ok {
		sendError(w, http.StatusUnauthorized, "UNAUTHORIZED", "API key context missing.", "Ensure auth middleware is enabled.")
		return
	}
	endpoint := "/v1/models/verify"
	if prefix == "run-system" {
		endpoint = "/v1/systems/verify"
	}
	idempotencyKey := getIdempotencyKey(r)
	idemStarted := false
	idemCompleted := false
	if idempotencyKey != "" {
		if !isValidIdempotencyKey(idempotencyKey) {
			sendError(w, http.StatusBadRequest, "INVALID_IDEMPOTENCY_KEY", "Idempotency key format is invalid.", "Use 1-128 characters from [A-Za-z0-9-_.:].")
			return
		}
		isNew, existing, err := s.store.BeginIdempotencyRequest(apiKey.WorkspaceID, endpoint, idempotencyKey)
		if err != nil {
			sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to initialize idempotency request.", "Retry later.")
			return
		}
		if !isNew {
			if existing.StatusCode > 0 {
				w.Header().Set("Content-Type", "application/json")
				w.WriteHeader(existing.StatusCode)
				_, _ = w.Write([]byte(existing.ResponseBody))
				return
			}
			sendError(w, http.StatusConflict, "IDEMPOTENCY_IN_PROGRESS", "A request with this idempotency key is still in progress.", "Retry with a new key or wait for completion.")
			return
		}
		idemStarted = true
		defer func() {
			if idemStarted && !idemCompleted {
				_ = s.store.CancelPendingIdempotencyRequest(apiKey.WorkspaceID, endpoint, idempotencyKey)
			}
		}()
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

	// Persist the run as "queued" with atomic quota + global inflight reservation.
	if err := s.store.ReserveRunForWorkspaceWithInflight(runID, apiKey.WorkspaceID, string(StatusQueued), s.maxInflightPerWorkspace); err != nil {
		if err == db.ErrInflightLimit {
			sendError(w, http.StatusTooManyRequests, "WORKSPACE_INFLIGHT_LIMIT", fmt.Sprintf("Workspace has reached max in-flight jobs (%d).", s.maxInflightPerWorkspace), "Wait for running jobs to complete before submitting more.")
			return
		}
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

	switch s.tryEnqueueJob(job) {
	case enqueueShuttingDown:
		_ = s.store.SetRunBillable(runID, false)
		_ = s.store.UpdateRunStatus(runID, string(StatusError), 0, 0, "")
		s.metrics.ShuttingDownRejected.Add(1)
		sendError(w, http.StatusServiceUnavailable, "SERVER_SHUTTING_DOWN", "Server is shutting down and not accepting new jobs.", "Retry after the service recovers.")
		return
	case enqueueQueueFull:
		_ = s.store.SetRunBillable(runID, false)
		_ = s.store.UpdateRunStatus(runID, string(StatusError), 0, 0, "")
		s.metrics.QueueFullRejected.Add(1)
		sendError(w, http.StatusServiceUnavailable, "QUEUE_FULL", "The simulation queue is at capacity.", "Retry after 5-10 seconds.")
		return
	default:
		log.Printf("[%s] Job enqueued: %s", prefix, runID)
		enqueued = true
	}

	resp := map[string]interface{}{
		"run_id":   runID,
		"status":   string(StatusQueued),
		"poll_url": fmt.Sprintf("/v1/runs/%s", runID),
	}
	w.Header().Set("Content-Type", "application/json")
	if idempotencyKey != "" {
		respBody, err := json.Marshal(resp)
		if err != nil {
			sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to persist idempotency response.", "Retry later.")
			return
		}
		if err := s.completeIdempotencyWithRetry(apiKey.WorkspaceID, endpoint, idempotencyKey, runID, http.StatusAccepted, string(respBody)); err != nil {
			sendError(w, http.StatusInternalServerError, "INTERNAL_ERROR", "Failed to persist idempotency response.", "Retry later.")
			return
		}
		idemCompleted = true
		w.WriteHeader(http.StatusAccepted)
		_, _ = w.Write(respBody)
		return
	}
	w.WriteHeader(http.StatusAccepted)
	json.NewEncoder(w).Encode(resp)
}

func (s *Server) handleVerifyModel(w http.ResponseWriter, r *http.Request) {
	r.Body = http.MaxBytesReader(w, r.Body, maxVerifyBodyBytes)
	body, err := io.ReadAll(r.Body)
	if err != nil {
		if strings.Contains(err.Error(), "http: request body too large") {
			sendError(w, http.StatusRequestEntityTooLarge, "PAYLOAD_TOO_LARGE", "Request body exceeds size limit.", "Reduce request size and retry.")
			return
		}
		sendError(w, http.StatusBadRequest, "INVALID_BODY", "Could not read request body.", "")
		return
	}
	normalized, ok := parseVerifyPayload(w, body)
	if !ok {
		return
	}
	s.submitJob(w, r, "run-model", normalized)
}

func (s *Server) handleVerifySystem(w http.ResponseWriter, r *http.Request) {
	r.Body = http.MaxBytesReader(w, r.Body, maxVerifyBodyBytes)
	body, err := io.ReadAll(r.Body)
	if err != nil {
		if strings.Contains(err.Error(), "http: request body too large") {
			sendError(w, http.StatusRequestEntityTooLarge, "PAYLOAD_TOO_LARGE", "Request body exceeds size limit.", "Reduce request size and retry.")
			return
		}
		sendError(w, http.StatusBadRequest, "INVALID_BODY", "Could not read request body.", "")
		return
	}
	normalized, ok := parseVerifyPayload(w, body)
	if !ok {
		return
	}
	s.submitJob(w, r, "run-system", normalized)
}

func parseVerifyPayload(w http.ResponseWriter, body []byte) ([]byte, bool) {
	trimmed := bytes.TrimSpace(body)
	if len(trimmed) == 0 {
		sendError(w, http.StatusBadRequest, "MISSING_REQUIRED_FIELDS", "chip_yaml is required.", "Provide raw YAML body or JSON with chip_yaml.")
		return nil, false
	}

	// Accept the documented JSON wrapper: {"chip_yaml":"...","peripheral_id":"optional"}
	if len(trimmed) > 0 && trimmed[0] == '{' {
		var req struct {
			ChipYAML   string `json:"chip_yaml"`
			SystemYAML string `json:"system_yaml"`
		}
		if err := json.Unmarshal(trimmed, &req); err != nil {
			sendError(w, http.StatusBadRequest, "INVALID_JSON", "The request body could not be parsed as valid JSON.", "Verify the JSON syntax.")
			return nil, false
		}
		payload := strings.TrimSpace(req.ChipYAML)
		if payload == "" {
			payload = strings.TrimSpace(req.SystemYAML)
		}
		if payload == "" {
			sendError(w, http.StatusBadRequest, "MISSING_REQUIRED_FIELDS", "chip_yaml or system_yaml is required.", "Provide a non-empty chip_yaml (model) or system_yaml (system) field.")
			return nil, false
		}
		return []byte(sanitizeInput(payload)), true
	}

	// Backward-compatible path: raw YAML body.
	return []byte(sanitizeInput(string(trimmed))), true
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

	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(s.runResponse(record, "/v1/runs/"+record.RunID+"/artifacts"))
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
	s.serveRunArtifact(w, r, record)
}

func (s *Server) runResponse(record *db.RunRecord, artifactsBaseURL string) map[string]interface{} {
	resp := map[string]interface{}{
		"run_id":            record.RunID,
		"status":            record.Status,
		"assertions_passed": record.AssertionsPassed,
		"assertions_total":  record.AssertionsTotal,
		"created_at":        record.CreatedAt,
	}
	if record.ArtifactsPath != "" {
		resp["artifacts"] = map[string]string{
			"ir_url":     artifactsBaseURL + "/output.json",
			"vcd_url":    artifactsBaseURL + "/proof.vcd",
			"result_url": artifactsBaseURL + "/result.json",
		}
	}
	return resp
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
			"peripheral_id": { "type": "string", "description": "Optional identifier for your peripheral model" },
			"chip_yaml": { "type": "string", "description": "LabWired YAML specification" }
		},
		"required": ["chip_yaml"]
	}`
	w.Header().Set("Content-Type", "application/json")
	w.Write([]byte(schema))
}

func (s *Server) completeIdempotencyWithRetry(workspaceID, endpoint, key, runID string, statusCode int, responseBody string) error {
	var lastErr error
	for i := 0; i < 5; i++ {
		err := s.store.CompleteIdempotencyRequest(workspaceID, endpoint, key, runID, statusCode, responseBody)
		if err == nil {
			return nil
		}
		lastErr = err
		msg := strings.ToLower(err.Error())
		if !strings.Contains(msg, "database is locked") && !strings.Contains(msg, "sqlite_busy") {
			return err
		}
		time.Sleep(time.Duration(i+1) * 10 * time.Millisecond)
	}
	return lastErr
}

// ── Background Worker ─────────────────────────────────────────────────────────

func (s *Server) worker(workerID int) {
	defer s.workersWG.Done()
	for {
		job, ok := s.dequeueJob()
		if !ok {
			return
		}
		workerTag := fmt.Sprintf("worker-%d", workerID)
		claimed, err := s.store.TryClaimQueuedRun(job.ID, workerTag, time.Now(), s.maxRunAttempts)
		if err != nil {
			log.Printf("[worker:%d] failed to claim run %s: %v", workerID, job.ID, err)
			s.jobs.Delete(job.ID)
			s.releaseWorkspaceSlot(job.WorkspaceID)
			continue
		}
		if !claimed {
			// Claimed or completed elsewhere.
			s.jobs.Delete(job.ID)
			s.releaseWorkspaceSlot(job.WorkspaceID)
			continue
		}

		runCtx, cancelRun := context.WithCancel(context.Background())
		hbDone := make(chan struct{})
		go func() {
			defer close(hbDone)
			ticker := time.NewTicker(s.workerHeartbeatInterval)
			defer ticker.Stop()
			for {
				select {
				case <-runCtx.Done():
					return
				case <-ticker.C:
					ok, hbErr := s.store.HeartbeatClaimedRun(job.ID, workerTag, time.Now())
					if hbErr != nil {
						log.Printf("[worker:%d] heartbeat failed for run %s: %v", workerID, job.ID, hbErr)
						continue
					}
					if !ok {
						// Lease no longer owned; stop heartbeats.
						return
					}
				}
			}
		}()

		var (
			result *verification.Result
			runErr error
		)
		switch job.Type {
		case JobTypeSynthesize:
			ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
			result, runErr = runSynthesisJob(ctx, job)
			cancel()
		default:
			ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
			result, runErr = s.orchestrator.RunSimulation(ctx, job.IRPath, job.ArtifactDir)
			cancel()
		}
		cancelRun()
		<-hbDone

		if runErr != nil {
			log.Printf("[worker:%d] run %s error: %v", workerID, job.ID, runErr)
			_ = s.store.CompleteClaimedRun(job.ID, string(StatusError), 0, 0, "", runErr.Error())
		} else if result.Pass {
			_ = s.store.CompleteClaimedRun(job.ID, string(StatusPass), result.AssertionsPassed, result.AssertionsTotal, job.ArtifactDir, "")

			// If this was a successful synthesis, promote to catalog
			if job.Type == JobTypeSynthesize {
				modelPath := filepath.Join(job.ArtifactDir, "output.json")
				modelData, err := os.ReadFile(modelPath)
				if err != nil {
					log.Printf("[worker:%d] failed to read generated model for promotion: %v", workerID, err)
				} else {
					asset := db.CatalogAsset{
						ID:          job.ID,
						Name:        job.ComponentName,
						Description: fmt.Sprintf("AI-synthesized model for %s.", job.ComponentName),
						PassRate:    100, // It just passed verification
						Registers:   0,   // Could be parsed from output.json if schema allows
					}
					if err := s.catalog.PromoteToCatalog(asset, modelData, s.dataDir); err != nil {
						log.Printf("[worker:%d] failed to promote synthesized model to catalog: %v", workerID, err)
					} else {
						log.Printf("[worker:%d] successfully promoted synthesized model %s to catalog", workerID, job.ID)
					}
				}
			}
		} else {
			_ = s.store.CompleteClaimedRun(job.ID, string(StatusFail), result.AssertionsPassed, result.AssertionsTotal, job.ArtifactDir, result.Error)
		}

		s.jobs.Delete(job.ID)
		s.releaseWorkspaceSlot(job.WorkspaceID)
	}
}

func runSynthesisJob(_ context.Context, job *Job) (*verification.Result, error) {
	if err := os.MkdirAll(job.ArtifactDir, 0755); err != nil {
		return nil, err
	}

	artifact, err := synthesis.GenerateArtifact(context.Background(), synthesis.Request{
		Kind:                job.SynthesisKind,
		ComponentName:       job.ComponentName,
		Requirements:        job.Requirements,
		DatasheetURL:        job.DatasheetURL,
		DocumentationURLs:   append([]string(nil), job.DocumentationURLs...),
		Board:               job.Board,
		DesiredCapabilities: append([]string(nil), job.DesiredCapabilities...),
		ValidationTargets:   append([]string(nil), job.ValidationTargets...),
		Workload:            job.Workload,
		Constraints:         job.Constraints,
	})
	if err != nil {
		return nil, err
	}
	if artifact.ContractResult != nil && strings.TrimSpace(job.PromotionMode) != "" {
		artifact.ContractResult.PromotionMode = strings.TrimSpace(job.PromotionMode)
	}

	payload, err := json.MarshalIndent(artifact, "", "  ")
	if err != nil {
		return nil, err
	}
	if err := os.WriteFile(filepath.Join(job.ArtifactDir, "output.json"), payload, 0644); err != nil {
		return nil, err
	}

	assertionsTotal, err := synthesis.ValidateArtifact(artifact)
	if err != nil {
		return failSynthesisResult(job, 0, 1, err), nil
	}

	if artifact.RepoBundle != nil && len(artifact.RepoBundle.Files) > 0 {
		bundleAssertions, bundleErr := materializeAndValidateRepoBundle(job, artifact.RepoBundle)
		if bundleErr != nil {
			return failSynthesisResult(job, assertionsTotal, assertionsTotal+1, bundleErr), nil
		}
		assertionsTotal += bundleAssertions
		if shouldApplyRepoPromotion(job) {
			if err := applyRepoBundleToRepo(job, artifact.RepoBundle); err != nil {
				return failSynthesisResult(job, assertionsTotal, assertionsTotal+1, err), nil
			}
			assertionsTotal++
		}
		if shouldRunBoardProof(job) {
			validationAssertions, validationErr := runBoardSmokeValidation(job, artifact.RepoBundle)
			if validationErr != nil {
				return failSynthesisResult(job, assertionsTotal, assertionsTotal+1, validationErr), nil
			}
			assertionsTotal += validationAssertions
		}
		if shouldRunGitPromotion(job) {
			promotionAssertions, promotionErr := promoteRepoBundleWithGit(job, artifact.RepoBundle)
			if promotionErr != nil {
				return failSynthesisResult(job, assertionsTotal, assertionsTotal+1, promotionErr), nil
			}
			assertionsTotal += promotionAssertions
		}
	}

	if err := writeSynthesisResult(job, true, assertionsTotal, assertionsTotal, ""); err != nil {
		return nil, err
	}

	return &verification.Result{
		Pass:             true,
		AssertionsPassed: assertionsTotal,
		AssertionsTotal:  assertionsTotal,
	}, nil
}

func failSynthesisResult(job *Job, passed int, total int, failure error) *verification.Result {
	if failure == nil {
		failure = fmt.Errorf("unknown synthesis failure")
	}
	_ = writeSynthesisResult(job, false, passed, total, failure.Error())
	return &verification.Result{
		Pass:             false,
		AssertionsPassed: passed,
		AssertionsTotal:  total,
		Error:            failure.Error(),
	}
}

func writeSynthesisResult(job *Job, pass bool, passed int, total int, errText string) error {
	payload := map[string]any{
		"pass":              pass,
		"assertions_passed": passed,
		"assertions_total":  total,
	}
	if strings.TrimSpace(errText) != "" {
		payload["error"] = errText
	}
	resultPayload, err := json.Marshal(payload)
	if err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(job.ArtifactDir, "result.json"), resultPayload, 0o644)
}

func shouldApplyRepoPromotion(job *Job) bool {
	if job == nil {
		return false
	}
	switch strings.TrimSpace(job.PromotionMode) {
	case "apply_to_repo", "commit_to_branch", "open_pr":
		return true
	default:
		return false
	}
}

func shouldRunBoardProof(job *Job) bool {
	if job == nil {
		return false
	}
	if job.DryRun {
		return false
	}
	return shouldApplyRepoPromotion(job)
}

func shouldRunGitPromotion(job *Job) bool {
	if job == nil {
		return false
	}
	switch strings.TrimSpace(job.PromotionMode) {
	case "commit_to_branch", "open_pr":
		return true
	default:
		return false
	}
}

func materializeAndValidateRepoBundle(job *Job, bundle *synthesis.RepoBundle) (int, error) {
	root := filepath.Join(job.ArtifactDir, "repo_bundle")
	if err := os.MkdirAll(root, 0o755); err != nil {
		return 0, fmt.Errorf("failed to create repo bundle dir: %w", err)
	}

	var chipPath string
	var systemPath string
	for _, file := range bundle.Files {
		target := filepath.Join(root, filepath.FromSlash(file.Path))
		if err := os.MkdirAll(filepath.Dir(target), 0o755); err != nil {
			return 0, fmt.Errorf("failed to create repo bundle parent dir: %w", err)
		}
		if err := os.WriteFile(target, []byte(file.Content), 0o644); err != nil {
			return 0, fmt.Errorf("failed to write generated file %s: %w", file.Path, err)
		}
		if strings.Contains(file.Path, "core/configs/chips/") && strings.HasSuffix(file.Path, ".yaml") {
			chipPath = target
		}
		if strings.Contains(file.Path, "core/configs/systems/") && strings.HasSuffix(file.Path, ".yaml") {
			systemPath = target
		}
	}

	if chipPath == "" || systemPath == "" {
		return 0, fmt.Errorf("repo bundle missing chip or system yaml for validation")
	}
	if strings.TrimSpace(job.LabWiredPath) == "" {
		return 0, fmt.Errorf("labwired path unavailable for repo bundle validation")
	}

	assertions := 0
	if err := runLabWiredAssetValidate(job, root, "--chip", chipPath, "validate_chip.json"); err != nil {
		return assertions, err
	}
	assertions++
	if err := runLabWiredAssetValidate(job, root, "--system", systemPath, "validate_system.json"); err != nil {
		return assertions, err
	}
	assertions++
	return assertions, nil
}

func runLabWiredAssetValidate(job *Job, workdir string, argName string, argValue string, outputName string) error {
	cmd := exec.Command(job.LabWiredPath, "asset", "validate", "--json", argName, argValue)
	cmd.Dir = workdir
	output, err := cmd.CombinedOutput()
	_ = os.WriteFile(filepath.Join(job.ArtifactDir, outputName), output, 0o644)
	if err != nil {
		return fmt.Errorf("labwired asset validate failed for %s: %w", argValue, err)
	}

	var parsed struct {
		Valid      bool `json:"valid"`
		Statistics struct {
			TotalChecks int `json:"total_checks"`
		} `json:"statistics"`
	}
	if json.Unmarshal(output, &parsed) != nil {
		return fmt.Errorf("labwired asset validate returned non-json output for %s", argValue)
	}
	if !parsed.Valid {
		return fmt.Errorf("labwired asset validate reported invalid bundle for %s", argValue)
	}
	if parsed.Statistics.TotalChecks < 1 {
		return fmt.Errorf("labwired asset validate reported no substantive checks for %s", argValue)
	}
	return nil
}

func runBoardSmokeValidation(job *Job, bundle *synthesis.RepoBundle) (int, error) {
	repoRoot := strings.TrimSpace(job.RepoRootDir)
	if repoRoot == "" || bundle == nil {
		return 0, nil
	}
	coreDir := filepath.Join(repoRoot, "core")
	if _, err := os.Stat(filepath.Join(coreDir, "Cargo.toml")); err != nil {
		return 0, nil
	}
	if _, err := os.Stat(filepath.Join(coreDir, "scripts", "unsupported_instruction_audit.sh")); err != nil {
		return 0, nil
	}

	var firmwareManifest string
	var smokeScript string
	var smokeScriptContent string
	var systemManifest string
	var boardID string
	for _, file := range bundle.Files {
		switch {
		case strings.HasSuffix(file.Path, "/board_firmware/Cargo.toml"):
			firmwareManifest = filepath.Join(repoRoot, filepath.FromSlash(file.Path))
			boardID = filepath.Base(filepath.Dir(filepath.Dir(filepath.FromSlash(file.Path))))
		case strings.HasSuffix(file.Path, "/uart-smoke.yaml"):
			smokeScript = filepath.Join(repoRoot, filepath.FromSlash(file.Path))
			smokeScriptContent = file.Content
			if boardID == "" {
				boardID = filepath.Base(filepath.Dir(filepath.FromSlash(file.Path)))
			}
		case strings.Contains(file.Path, "core/configs/systems/") && strings.HasSuffix(file.Path, ".yaml"):
			systemManifest = filepath.Join(repoRoot, filepath.FromSlash(file.Path))
			if boardID == "" {
				boardID = strings.TrimSuffix(filepath.Base(file.Path), ".yaml")
			}
		}
	}
	if firmwareManifest == "" || smokeScript == "" || systemManifest == "" {
		return 0, nil
	}

	firmwarePackage := "firmware-" + boardID + "-demo"
	rustTarget := inferRustTargetFromSmokeScript(smokeScriptContent)
	firmwareBinary := filepath.Join(filepath.Dir(firmwareManifest), "target", rustTarget, "release", firmwarePackage)
	smokeOutputDir := filepath.Join(coreDir, "out", boardID, "uart-smoke")
	auditOutputDir := filepath.Join(coreDir, "out", "unsupported-audit", boardID)

	assertions := 0
	if err := runCommandLogged(job, coreDir, "build_smoke.log", 5*time.Minute, "cargo", "build", "--manifest-path", relPath(coreDir, firmwareManifest), "--release", "--target", rustTarget); err != nil {
		return assertions, fmt.Errorf("smoke firmware build failed: %w", err)
	}
	assertions++
	if err := runCommandLogged(job, coreDir, "run_smoke.log", 5*time.Minute, "cargo", "run", "-q", "-p", "labwired-cli", "--", "test", "--script", relPath(coreDir, smokeScript), "--output-dir", relPath(coreDir, smokeOutputDir), "--no-uart-stdout"); err != nil {
		return assertions, fmt.Errorf("uart smoke validation failed: %w", err)
	}
	assertions++
	if err := runCommandLogged(job, coreDir, "unsupported_audit.log", 10*time.Minute, "./scripts/unsupported_instruction_audit.sh", "--firmware", relPath(coreDir, firmwareBinary), "--system", relPath(coreDir, systemManifest), "--max-steps", "200000", "--out-dir", relPath(coreDir, auditOutputDir)); err != nil {
		return assertions, fmt.Errorf("unsupported instruction audit failed: %w", err)
	}
	assertions++

	if err := copyIfExists(filepath.Join(auditOutputDir, "report.md"), filepath.Join(job.ArtifactDir, "unsupported_audit_report.md")); err == nil {
		assertions++
	}
	if err := copyIfExists(filepath.Join(smokeOutputDir, "result.json"), filepath.Join(job.ArtifactDir, "uart_smoke_result.json")); err == nil {
		assertions++
	}
	return assertions, nil
}

func inferRustTargetFromSmokeScript(script string) string {
	re := regexp.MustCompile(`(?m)^\s*firmware:\s+"\.\/board_firmware\/target\/([^/]+)\/release\/[^"]+"`)
	matches := re.FindStringSubmatch(script)
	if len(matches) == 2 && strings.TrimSpace(matches[1]) != "" {
		return strings.TrimSpace(matches[1])
	}
	return "thumbv7em-none-eabi"
}

func runCommandLogged(job *Job, workdir string, outputName string, timeout time.Duration, name string, args ...string) error {
	ctx, cancel := context.WithTimeout(context.Background(), timeout)
	defer cancel()
	cmd := exec.CommandContext(ctx, name, args...)
	cmd.Dir = workdir
	output, err := cmd.CombinedOutput()
	_ = os.WriteFile(filepath.Join(job.ArtifactDir, outputName), output, 0o644)
	if ctx.Err() == context.DeadlineExceeded {
		return fmt.Errorf("%s timed out", name)
	}
	if err != nil {
		return fmt.Errorf("%s %s: %w", name, strings.Join(args, " "), err)
	}
	return nil
}

func relPath(base string, target string) string {
	rel, err := filepath.Rel(base, target)
	if err != nil {
		return target
	}
	return filepath.ToSlash(rel)
}

func copyIfExists(src string, dst string) error {
	data, err := os.ReadFile(src)
	if err != nil {
		return err
	}
	return os.WriteFile(dst, data, 0o644)
}

func applyRepoBundleToRepo(job *Job, bundle *synthesis.RepoBundle) error {
	repoRoot := strings.TrimSpace(job.RepoRootDir)
	if repoRoot == "" {
		return fmt.Errorf("repo root unavailable for bundle promotion")
	}
	repoRootAbs, err := filepath.Abs(repoRoot)
	if err != nil {
		return fmt.Errorf("failed to resolve repo root: %w", err)
	}
	for _, file := range bundle.Files {
		target := filepath.Join(repoRootAbs, filepath.FromSlash(file.Path))
		targetAbs, err := filepath.Abs(target)
		if err != nil {
			return fmt.Errorf("failed to resolve target path %s: %w", file.Path, err)
		}
		if !strings.HasPrefix(targetAbs, repoRootAbs+string(os.PathSeparator)) && targetAbs != repoRootAbs {
			return fmt.Errorf("refusing to write bundle file outside repo root: %s", file.Path)
		}
		if err := os.MkdirAll(filepath.Dir(targetAbs), 0o755); err != nil {
			return fmt.Errorf("failed to create repo parent dir for %s: %w", file.Path, err)
		}
		if err := os.WriteFile(targetAbs, []byte(file.Content), 0o644); err != nil {
			return fmt.Errorf("failed to write repo bundle file %s: %w", file.Path, err)
		}
	}
	return nil
}

type gitPromotionResult struct {
	Mode      string `json:"mode"`
	Branch    string `json:"branch,omitempty"`
	Commit    string `json:"commit,omitempty"`
	PRURL     string `json:"pr_url,omitempty"`
	Base      string `json:"base_branch,omitempty"`
	Remote    string `json:"remote,omitempty"`
	Worktree  string `json:"worktree,omitempty"`
	Files     int    `json:"files"`
	CreatedAt string `json:"created_at"`
}

func promoteRepoBundleWithGit(job *Job, bundle *synthesis.RepoBundle) (int, error) {
	repoRoot := strings.TrimSpace(job.RepoRootDir)
	if repoRoot == "" {
		return 0, fmt.Errorf("repo root unavailable for git promotion")
	}
	repoRootAbs, err := filepath.Abs(repoRoot)
	if err != nil {
		return 0, fmt.Errorf("failed to resolve repo root: %w", err)
	}
	if err := runGitCommand(repoRootAbs, nil, "rev-parse", "--show-toplevel"); err != nil {
		return 0, fmt.Errorf("repo root is not a git repository: %w", err)
	}

	worktreeDir := filepath.Join(job.ArtifactDir, "git_promotion_worktree")
	if err := os.RemoveAll(worktreeDir); err != nil {
		return 0, fmt.Errorf("failed to clear git promotion worktree: %w", err)
	}
	if err := runGitCommand(repoRootAbs, nil, "worktree", "add", "--detach", worktreeDir, "HEAD"); err != nil {
		return 0, fmt.Errorf("failed to create git worktree: %w", err)
	}
	defer func() {
		_ = runGitCommand(repoRootAbs, nil, "worktree", "remove", "--force", worktreeDir)
	}()

	if err := applyBundleToPath(worktreeDir, bundle); err != nil {
		return 0, err
	}

	branch := promotionBranchName(job)
	if err := runGitCommand(worktreeDir, gitAuthorEnv(), "switch", "-c", branch); err != nil {
		return 0, fmt.Errorf("failed to create promotion branch %s: %w", branch, err)
	}

	addArgs := []string{"add"}
	for _, path := range bundlePaths(bundle) {
		addArgs = append(addArgs, filepath.ToSlash(path))
	}
	if err := runGitCommand(worktreeDir, gitAuthorEnv(), addArgs...); err != nil {
		return 0, fmt.Errorf("failed to stage generated files: %w", err)
	}
	if err := runGitCommand(worktreeDir, gitAuthorEnv(), "commit", "-m", promotionCommitMessage(job)); err != nil {
		return 0, fmt.Errorf("failed to commit generated files: %w", err)
	}
	commitSHA, err := gitOutput(worktreeDir, nil, "rev-parse", "HEAD")
	if err != nil {
		return 0, fmt.Errorf("failed to resolve promotion commit: %w", err)
	}

	result := gitPromotionResult{
		Mode:      strings.TrimSpace(job.PromotionMode),
		Branch:    branch,
		Commit:    strings.TrimSpace(commitSHA),
		Files:     len(bundle.Files),
		CreatedAt: time.Now().UTC().Format(time.RFC3339),
		Worktree:  worktreeDir,
	}
	assertions := 1

	if strings.TrimSpace(job.PromotionMode) == "open_pr" {
		baseBranch, err := gitDefaultBranch(repoRootAbs)
		if err != nil {
			return assertions, err
		}
		if err := runGitCommand(worktreeDir, gitAuthorEnv(), "push", "-u", "origin", branch); err != nil {
			return assertions, fmt.Errorf("failed to push promotion branch: %w", err)
		}
		prURL, err := createPullRequest(worktreeDir, branch, baseBranch, promotionPRTitle(job), promotionPRBody(job))
		if err != nil {
			return assertions, err
		}
		result.PRURL = strings.TrimSpace(prURL)
		result.Base = baseBranch
		result.Remote = "origin"
		assertions++
	}

	if err := writeGitPromotionResult(job, result); err != nil {
		return assertions, err
	}
	assertions++
	return assertions, nil
}

func writeGitPromotionResult(job *Job, result gitPromotionResult) error {
	payload, err := json.MarshalIndent(result, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(job.ArtifactDir, "git_promotion_result.json"), payload, 0o644)
}

func applyBundleToPath(root string, bundle *synthesis.RepoBundle) error {
	rootAbs, err := filepath.Abs(root)
	if err != nil {
		return fmt.Errorf("failed to resolve bundle root: %w", err)
	}
	for _, file := range bundle.Files {
		target := filepath.Join(rootAbs, filepath.FromSlash(file.Path))
		targetAbs, err := filepath.Abs(target)
		if err != nil {
			return fmt.Errorf("failed to resolve target path %s: %w", file.Path, err)
		}
		if !strings.HasPrefix(targetAbs, rootAbs+string(os.PathSeparator)) && targetAbs != rootAbs {
			return fmt.Errorf("refusing to write bundle file outside repo root: %s", file.Path)
		}
		if err := os.MkdirAll(filepath.Dir(targetAbs), 0o755); err != nil {
			return fmt.Errorf("failed to create repo parent dir for %s: %w", file.Path, err)
		}
		if err := os.WriteFile(targetAbs, []byte(file.Content), 0o644); err != nil {
			return fmt.Errorf("failed to write repo bundle file %s: %w", file.Path, err)
		}
	}
	return nil
}

func bundlePaths(bundle *synthesis.RepoBundle) []string {
	if bundle == nil {
		return nil
	}
	paths := make([]string, 0, len(bundle.Files))
	for _, file := range bundle.Files {
		if trimmed := strings.TrimSpace(file.Path); trimmed != "" {
			paths = append(paths, filepath.FromSlash(trimmed))
		}
	}
	sort.Strings(paths)
	return paths
}

func promotionBranchName(job *Job) string {
	base := sanitizeGitIdent(job.ComponentName)
	if job.Board != nil {
		if board := strings.TrimSpace(job.Board.BoardID); board != "" {
			base = sanitizeGitIdent(board)
		} else if board := strings.TrimSpace(job.Board.MarketingName); board != "" {
			base = sanitizeGitIdent(board)
		}
	}
	if base == "" {
		base = "onboarding"
	}
	suffix := job.ID
	if idx := strings.LastIndex(suffix, "-"); idx > 0 {
		suffix = suffix[idx+1:]
	}
	return "foundry/onboard-" + base + "-" + suffix
}

func promotionCommitMessage(job *Job) string {
	name := strings.TrimSpace(job.ComponentName)
	if job.Board != nil {
		if board := strings.TrimSpace(job.Board.MarketingName); board != "" {
			name = board
		}
	}
	if name == "" {
		name = "generated board assets"
	}
	return "Onboard " + name
}

func promotionPRTitle(job *Job) string {
	return promotionCommitMessage(job)
}

func promotionPRBody(job *Job) string {
	lines := []string{
		"Generated by Foundry ingestion pipeline.",
		"",
		"Run ID: `" + job.ID + "`",
	}
	if job.Board != nil {
		if board := strings.TrimSpace(job.Board.MCU); board != "" {
			lines = append(lines, "MCU: `"+board+"`")
		}
		if caps := strings.Join(job.DesiredCapabilities, ", "); strings.TrimSpace(caps) != "" {
			lines = append(lines, "Capabilities: `"+caps+"`")
		}
	}
	return strings.Join(lines, "\n")
}

func sanitizeGitIdent(value string) string {
	value = strings.ToLower(strings.TrimSpace(value))
	if value == "" {
		return ""
	}
	var b strings.Builder
	lastDash := false
	for _, r := range value {
		ok := (r >= 'a' && r <= 'z') || (r >= '0' && r <= '9')
		if ok {
			b.WriteRune(r)
			lastDash = false
			continue
		}
		if !lastDash {
			b.WriteByte('-')
			lastDash = true
		}
	}
	out := strings.Trim(b.String(), "-")
	return out
}

func gitAuthorEnv() []string {
	return []string{
		"GIT_AUTHOR_NAME=Foundry Bot",
		"GIT_AUTHOR_EMAIL=foundry@labwired.local",
		"GIT_COMMITTER_NAME=Foundry Bot",
		"GIT_COMMITTER_EMAIL=foundry@labwired.local",
	}
}

func runGitCommand(workdir string, extraEnv []string, args ...string) error {
	cmd := exec.Command("git", args...)
	cmd.Dir = workdir
	cmd.Env = append(os.Environ(), extraEnv...)
	output, err := cmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("git %s: %w (%s)", strings.Join(args, " "), err, strings.TrimSpace(string(output)))
	}
	return nil
}

func gitOutput(workdir string, extraEnv []string, args ...string) (string, error) {
	cmd := exec.Command("git", args...)
	cmd.Dir = workdir
	cmd.Env = append(os.Environ(), extraEnv...)
	output, err := cmd.CombinedOutput()
	if err != nil {
		return "", fmt.Errorf("git %s: %w (%s)", strings.Join(args, " "), err, strings.TrimSpace(string(output)))
	}
	return string(output), nil
}

func gitDefaultBranch(repoRoot string) (string, error) {
	ref, err := gitOutput(repoRoot, nil, "symbolic-ref", "refs/remotes/origin/HEAD")
	if err == nil {
		ref = strings.TrimSpace(ref)
		if idx := strings.LastIndex(ref, "/"); idx >= 0 && idx < len(ref)-1 {
			return ref[idx+1:], nil
		}
	}
	current, currentErr := gitOutput(repoRoot, nil, "branch", "--show-current")
	current = strings.TrimSpace(current)
	if current != "" {
		return current, nil
	}
	if currentErr != nil {
		return "", fmt.Errorf("failed to resolve base branch: %w", currentErr)
	}
	return "main", nil
}

func createPullRequest(workdir string, branch string, base string, title string, body string) (string, error) {
	ghPath := strings.TrimSpace(os.Getenv("GH_PATH"))
	if ghPath == "" {
		ghPath = "gh"
	}
	cmd := exec.Command(ghPath, "pr", "create", "--title", title, "--body", body, "--base", base, "--head", branch)
	cmd.Dir = workdir
	output, err := cmd.CombinedOutput()
	if err != nil {
		return "", fmt.Errorf("failed to create pull request: %w (%s)", err, strings.TrimSpace(string(output)))
	}
	return strings.TrimSpace(string(output)), nil
}

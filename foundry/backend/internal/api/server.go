package api

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"sync"
	"time"

	"github.com/gorilla/mux"
	"github.com/labwired/foundry-backend/internal/catalog"
	"github.com/labwired/foundry-backend/internal/db"
	"github.com/labwired/foundry-backend/internal/verification"
)

type JobStatus string

const (
	StatusQueued  JobStatus = "queued"
	StatusRunning JobStatus = "running"
	StatusPass    JobStatus = "pass"
	StatusFail    JobStatus = "fail"
	StatusError   JobStatus = "error"
)

type Job struct {
	ID        string              `json:"run_id"`
	Status    JobStatus           `json:"status"`
	Result    *verification.Result `json:"result,omitempty"`
	CreatedAt time.Time           `json:"created_at"`
}

type Server struct {
	router      *mux.Router
	jobs        sync.Map // run_id -> *Job
	jobQueue    chan *Job
	orchestrator *verification.Orchestrator
	store       *db.Store
	catalog     *catalog.Manager
}

func NewServer(orch *verification.Orchestrator, store *db.Store, cat *catalog.Manager) *Server {
	s := &Server{
		router:      mux.NewRouter(),
		jobQueue:    make(chan *Job, 100),
		orchestrator: orch,
		store:       store,
		catalog:     cat,
	}
	s.routes()
	go s.worker()
	return s
}

func (s *Server) routes() {
	s.router.HandleFunc("/v1/catalog", s.handleListCatalog).Methods("GET")
	s.router.HandleFunc("/v1/catalog/{id}", s.handleGetCatalogAsset).Methods("GET")
	s.router.HandleFunc("/v1/info", s.handleInfo).Methods("GET")
	s.router.HandleFunc("/v1/schema/synthesis", s.handleSchemaSynthesis).Methods("GET")

	// Protected VaaS Routes (Requires API Key)
	protected := s.router.PathPrefix("/v1").Subrouter()
	protected.Use(s.authMiddleware)

	protected.HandleFunc("/tasks/next", s.handleGetNextTask).Methods("GET")
	protected.HandleFunc("/tasks/{id}/context", s.handleGetTaskContext).Methods("GET")

	// Synthesis-as-a-Service endpoint
	protected.Handle("/synthesize", s.quotaMiddleware(http.HandlerFunc(s.handleSynthesize))).Methods("POST")

	// Quota-protected endpoints (Consume run credits)
	protected.Handle("/tasks/{id}/verify", s.quotaMiddleware(http.HandlerFunc(s.handleVerifyTask))).Methods("POST")
	protected.Handle("/systems/verify", s.quotaMiddleware(http.HandlerFunc(s.handleVerifySystem))).Methods("POST")

	protected.HandleFunc("/usage", s.handleUsage).Methods("GET")

	// Documentation
	s.router.HandleFunc("/v1/openapi.yaml", func(w http.ResponseWriter, r *http.Request) {
		http.ServeFile(w, r, "static/openapi.yaml")
	}).Methods("GET")

	s.router.PathPrefix("/v1/docs").Handler(http.StripPrefix("/v1/docs", http.FileServer(http.Dir("static"))))
}

func (s *Server) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	s.corsMiddleware(s.router).ServeHTTP(w, r)
}

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

func (s *Server) handleGetNextTask(w http.ResponseWriter, r *http.Request) {
	// Mock returning a task for the agent
	task := map[string]interface{}{
		"id":          "task-bme280-001",
		"name":        "BME280 Temperature Sensor",
		"description": "Implement a digital twin for the BME280 focusing strictly on the I2C interface and ID register (0xD0).",
		"status":      "open",
	}
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(task)
}

func (s *Server) handleGetTaskContext(w http.ResponseWriter, r *http.Request) {
	id := mux.Vars(r)["id"]
	if id != "task-bme280-001" {
		sendError(w, http.StatusNotFound, "TASK_NOT_FOUND", "The requested task ID does not exist.", "Ensure the task ID is correct.")
		return
	}

	ctxResp := map[string]interface{}{
		"task_id": id,
		"datasheet_excerpts": []string{
			"The I2C device address is 0x76 or 0x77.",
			"Register 0xD0 'id' contains the value 0x60.",
		},
		"memory_map_constraints": map[string]string{
			"0xD0": "Read-only, expected value 0x60",
		},
	}
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(ctxResp)
}

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

	// Consume 10 runs for synthesis (Mock)
	jobID := "synth-" + fmt.Sprintf("%d", time.Now().UnixNano())
	apiKey := r.Context().Value("api_key").(*db.APIKey)

	for i := 0; i < 10; i++ {
		_ = s.store.SaveRun(fmt.Sprintf("%s-%d", jobID, i), apiKey.WorkspaceID, "pass")
	}

	// Return 202 Accepted
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(http.StatusAccepted)
	json.NewEncoder(w).Encode(map[string]interface{}{
		"job_id": jobID,
		"status": "processing",
		"message": "Synthesis job started. The internal engine is drafting and formally verifying the model.",
	})
}

func (s *Server) handleVerifyTask(w http.ResponseWriter, r *http.Request) {
	id := mux.Vars(r)["id"]
	if id != "task-bme280-001" {
		sendError(w, http.StatusNotFound, "TASK_NOT_FOUND", "The requested task ID does not exist.", "Ensure the task ID is correct.")
		return
	}

	var req struct {
		YAML string `json:"chip_yaml"`
	}
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		sendError(w, http.StatusBadRequest, "INVALID_JSON", "The request body could not be parsed as valid JSON.", "Verify the JSON syntax and ensure all required fields are present.")
		return
	}

	// Synchronously execute verification (mocked for now, but in reality calls Orchestrator)
	runID := "run-" + id + "-" + fmt.Sprintf("%d", time.Now().UnixNano())
	apiKey := r.Context().Value("api_key").(*db.APIKey)
	_ = s.store.SaveRun(runID, apiKey.WorkspaceID, "pass") // Consume 1 quota run

	// We return detailed compiler logs and VCD traces.
	result := map[string]interface{}{
		"pass":              false, // Mock a failure to show iterative loop
		"assertions_passed": 1,
		"assertions_total":  2,
		"compiler_logs":     "Error: Register 0xD0 mismatch. Expected 0x60, read 0x00.",
		"vcd_url":           "/v1/docs/trace-bme280.vcd", // Fake URL
	}

	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(result)
}

func (s *Server) handleVerifySystem(w http.ResponseWriter, r *http.Request) {
	var req struct {
		SystemYAML string `json:"system_yaml"`
	}
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		sendError(w, http.StatusBadRequest, "INVALID_JSON", "The request body could not be parsed as valid JSON.", "Verify the JSON syntax and ensure all required fields are present.")
		return
	}

	// Mocking a powerful system-level verification response.
	runID := "run-system-" + fmt.Sprintf("%d", time.Now().UnixNano())
	apiKey := r.Context().Value("api_key").(*db.APIKey)
	_ = s.store.SaveRun(runID, apiKey.WorkspaceID, "pass") // Consume 1 quota run

	// In reality, it runs the orchestrator with the master system.yaml and returns traces spanning multiple buses.
	result := map[string]interface{}{
		"pass":              false, // Mock an integration failure
		"assertions_passed": 45,
		"assertions_total":  46,
		"compiler_logs":     "System Integration Error: Address collision on I2C1 bus. Both BME280_1 and BME280_2 configured with address 0x76.",
		"vcd_url":           "/v1/docs/trace-system-integration-multi-bus.vcd",
	}

	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(result)
}

func (s *Server) handleUsage(w http.ResponseWriter, r *http.Request) {
	apiKeyVal := r.Context().Value("api_key")
	if apiKeyVal == nil {
		sendError(w, http.StatusUnauthorized, "UNAUTHORIZED", "API Key not found.", "")
		return
	}
	apiKey := apiKeyVal.(*db.APIKey)

	limit := 50
	if apiKey.Tier == "enterprise" {
		limit = 1000000
	}

	used, _ := s.store.CountRunsForWorkspace(apiKey.WorkspaceID)

	json.NewEncoder(w).Encode(map[string]any{
		"workspace_id":         apiKey.WorkspaceID,
		"tier":                 apiKey.Tier,
		"runs_used_this_month": used,
		"quota":                limit,
	})
}

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

func (s *Server) worker() {
	for job := range s.jobQueue {
		job.Status = StatusRunning

		// In a real app, we'd find the IR path or generate it here
		// Mocking the IR conversion and verification
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)

		result, err := s.orchestrator.RunSimulation(ctx, "mock_ir.json")
		if err != nil {
			job.Status = StatusError
		} else if result.Pass {
			job.Status = StatusPass
			job.Result = result
		} else {
			job.Status = StatusFail
			job.Result = result
		}

		cancel()
	}
}

package api

import (
	"context"
	"encoding/json"
	"net/http"
	"sync"
	"time"

	"github.com/google/uuid"
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
	s.router.HandleFunc("/v1/twins/simulate", s.handleSimulate).Methods("POST")
	s.router.HandleFunc("/v1/runs/{id}", s.handleGetRun).Methods("GET")
	s.router.HandleFunc("/v1/usage", s.handleUsage).Methods("GET")
}

func (s *Server) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	s.router.ServeHTTP(w, r)
}

func (s *Server) handleListCatalog(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(s.catalog.List())
}

func (s *Server) handleGetCatalogAsset(w http.ResponseWriter, r *http.Request) {
	id := mux.Vars(r)["id"]
	asset, ok := s.catalog.Get(id)
	if !ok {
		http.NotFound(w, r)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(asset)
}

func (s *Server) handleSimulate(w http.ResponseWriter, r *http.Request) {
	// 1. Authenticate (MVP: placeholder)

	// 2. Parse request
	var req struct {
		PeripheralID string `json:"peripheral_id"`
		YAML         string `json:"chip_yaml"`
	}
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}

	// 3. Create job
	job := &Job{
		ID:        uuid.New().String(),
		Status:    StatusQueued,
		CreatedAt: time.Now(),
	}
	s.jobs.Store(job.ID, job)
	s.jobQueue <- job

	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(http.StatusAccepted)
	json.NewEncoder(w).Encode(map[string]string{
		"run_id":   job.ID,
		"status":   string(job.Status),
		"poll_url": "/v1/runs/" + job.ID,
	})
}

func (s *Server) handleGetRun(w http.ResponseWriter, r *http.Request) {
	id := mux.Vars(r)["id"]
	val, ok := s.jobs.Load(id)
	if !ok {
		http.NotFound(w, r)
		return
	}
	job := val.(*Job)

	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(job)
}

func (s *Server) handleUsage(w http.ResponseWriter, r *http.Request) {
	// Mock usage response
	json.NewEncoder(w).Encode(map[string]any{
		"runs_used_this_month": 12,
		"quota":                1000,
	})
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

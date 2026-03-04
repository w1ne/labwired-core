package api

import (
	"encoding/json"
	"net/http"
)

type APIError struct {
	Code        string `json:"code"`
	Message     string `json:"message"`
	Remediation string `json:"remediation,omitempty"`
}

func sendError(w http.ResponseWriter, code int, errCode, message, remediation string) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(code)
	json.NewEncoder(w).Encode(APIError{
		Code:        errCode,
		Message:     message,
		Remediation: remediation,
	})
}

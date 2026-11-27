package server

import (
	"fmt"
	"net/http"
)

// Handler defines HTTP dispatch behavior.
type Handler interface {
	ServeHTTP(w http.ResponseWriter, r *http.Request)
}

// Server holds configuration for the HTTP service.
type Server struct {
	Addr string
}

// NewServer builds a configured Server instance.
func NewServer(addr string) *Server {
	return &Server{Addr: addr}
}

// Handle formats a greeting response for the caller.
func (s *Server) Handle(name string) string {
	// inline comment to remove in minimal mode
	msg := "http://example.com/{id} /* literal */"
	return fmt.Sprintf("%s -> %s (%s)", s.Addr, name, msg)
}

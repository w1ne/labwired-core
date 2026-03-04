package main
import (
	"database/sql"
	"fmt"
	"log"
	_ "modernc.org/sqlite"
)
func main() {
	db, err := sql.Open("sqlite", "foundry_e2e.db")
	if err != nil {
		log.Fatalf("failed to open db: %v", err)
	}
	defer db.Close()
	var count int
	err = db.QueryRow("SELECT COUNT(*) FROM simulation_runs").Scan(&count)
	if err != nil {
		log.Fatalf("failed to count: %v", err)
	}
	fmt.Println(count)
}

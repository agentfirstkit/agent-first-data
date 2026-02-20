// Command agent_cli demonstrates canonical CLI helper usage for agent tools.
//
// Run: go run ./examples/agent_cli
package main

import (
	"fmt"

	afdata "github.com/cmnspore/agent-first-data/go"
)

func main() {
	// 1. Parse --output flag with structured error on failure.
	format, err := afdata.CliParseOutput("json")
	if err != nil {
		fmt.Println(afdata.OutputJson(afdata.BuildCliError(err.Error())))
		return
	}

	// 2. Normalize --log filters: trim, lowercase, deduplicate.
	filters := afdata.CliParseLogFilters([]string{"Query", " ERROR ", "query"})
	_ = filters // ["query", "error"]

	// 3. Format and emit output.
	result := map[string]any{"code": "ok", "size_bytes": int64(1024)}
	fmt.Println(afdata.CliOutput(result, format))
}

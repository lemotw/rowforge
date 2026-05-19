# rowforge handler SDK (Go)

```go
package main

import (
    "strings"

    handler "github.com/lemotw/rowforge/sdks/go"
)

func main() {
    handler.Run(func(row map[string]any, _ handler.Context) (map[string]any, error) {
        email, _ := row["email"].(string)
        if !strings.Contains(email, "@") {
            return nil, handler.Error("INVALID_EMAIL", "missing @")
        }
        return map[string]any{"domain": strings.SplitN(email, "@", 2)[1]}, nil
    }, handler.WithVersion("0.1.0"))
}
```

Until the SDK is published to a public Go module proxy, depend on it via a `replace` directive:

```
replace github.com/lemotw/rowforge/sdks/go => /absolute/path/to/rowforge/sdks/go
```

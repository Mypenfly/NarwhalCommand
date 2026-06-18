# N_Edit — Annotation-Driven Code Editing Tool

## Overview

N_Edit is a **declarative code editing tool** that uses annotation-prefixed
commands embedded in `.ned` script files to perform precise, format-aware
edits on source code.

Unlike traditional text-based search-and-replace, N_Edit uses a
**semantic matching algorithm** that:

1. Ignores whitespace differences during matching
2. Respects indentation structure (`diff_taps`)
3. Supports block-level operations via brace/indent parsing

## Commands Reference

| Command | Purpose | Example |
|---------|---------|---------|
| `//!@Open:` | Open target file | `//!@Open: ./src/main.rs` |
| `//!@Location:` | Locate code position | See Location section |
| `//!@New:` | Insert new content | `//!@New:\n    let x = 1;` |
| `//!@Delete:` | Delete matched content | `//!@Delete:\n    old_code();` |
| `//!@Off:` | Close current scope | `//!@Off:Open` |

### Location Matching

The core of N_Edit is the Location matching algorithm. It works by:

1. Extracting the content between `//!@Location:` and `...` (separator)
2. Matching the first line's stripped content against the target file
3. Filtering candidates by comparing stripped content AND indentation
   differences (`diff_taps`) for each subsequent line
4. Requiring exactly one match — otherwise reporting an error with
   diagnostic information

```rust
//!@Location:
fn process_data(items: &[Item]) -> Vec<Output> {
    let mut results = Vec::new();
...
//!@Delete:
    let mut results = Vec::new();
...
```

### Block Commands (Phase 3)

For brace-based languages (Rust, C, JS, Java) and indentation-based
languages (Python, YAML), N_Edit can precisely identify code block
boundaries using `Location:Block` and `Delete:Block`.

```python
# .ned script for Python
//!@Location:Block
def handle_request(request: Request) -> Response:
//!@New:
def handle_health_check() -> dict:
    """Simple health check endpoint."""
    return {"status": "ok", "timestamp": time.time()}
...
```

## Error Handling

All errors include:

- **Error type** — concise summary
- **Context** — relevant code snippet
- **Suggestion** — actionable fix

## Performance Considerations

N_Edit uses several optimizations:

- **Pre-computed stripped content** — each line stores its whitespace-free
  version at parse time, avoiding repeated allocation during matching
- **First-line hash index** — `HashMap<String, Vec<usize>>` enables O(1)
  lookup for candidate starting positions
- **Lazy block parsing** — block boundaries are only resolved when
  `Location:Block` or `Delete:Block` is explicitly requested

## File Format Reference

### `.ned` Script Structure

```text
//!@Open: path/to/target/file
//!@Location:
first_line_of_location
    subsequent_lines
...
//!@Delete:
lines_to_delete
...
//!@New:
    replacement_lines
...
//!@Off:Open
```

### Supported File Types

| Language | Brace / Indent | Block Support |
|----------|---------------|---------------|
| Rust | Brace | Full |
| C / C++ | Brace | Full |
| JavaScript / TypeScript | Brace | Full |
| Java | Brace | Full |
| Python | Indent | Full |
| YAML | Indent | Full |
| Markdown | Neither | Location only |

---

*Last updated: 2025-06-07*

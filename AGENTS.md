# AGENTS.md

## Build & Test

```bash
cargo build              # debug build
cargo build --release    # release build
cargo test               # run all tests (unit + integration)
cargo test -p n_edit     # run all tests for this crate
cargo test --lib         # unit tests only (inline #[cfg(test)] modules)
cargo test --test integration_test  # integration tests only
cargo fmt --check        # check formatting
cargo clippy -- -D warnings  # lint
```

For focused test runs during development, use test name filters:
```bash
cargo test -- engine    # run all tests matching "engine"
```

If inside the Nix dev shell (`nix develop`), use `cargo nextest run` as a faster alternative.

## Architecture

```
.ned script → Lexer (//!@ tokens) → Parser (AST Commands) → Engine (state machine) → file write + diff output
```

**Modules** (one core type per file):
| File | Purpose |
|------|---------|
| `main.rs` | CLI entry (clap), reads script, drives lexer→parser→engine pipeline |
| `lib.rs` | Re-exports all public modules for integration tests |
| `lexer.rs` | Scans `.ned` for `//!@` prefixed commands, produces `Token` stream |
| `parser.rs` | Converts `Token` stream to `Command` AST, validates grammar |
| `engine.rs` | Executes `Command` sequence via state machine (`block_stack`, `file`) |
| `matcher.rs` | Location matching: whitespace-stripped content + diff_taps comparison |
| `model.rs` | All data types: `FileContent`, `ContentBlock`, `Line`, `LocationContent`, `NewContent`, `DeleteContent` |
| `error.rs` | Centralized error types (`NEditError` enum wrapping sub-errors) |
| `output.rs` | Colored terminal diff output (`+` green, `-` red) |
| `block.rs` | Stub — Block parsing for Phase 3 (curly-brace / indentation-based) |
| `file_io.rs` | Stub — file I/O utilities (actual logic in `model.rs`) |

**Command state machine:** `Open → Location → [Location]* → New/Delete → Off`

## Key Concepts

- **`taps`** = count of leading ASCII spaces (indentation). Tab is *not* counted as a space.
- **`diff_taps`** = indentation difference relative to block/file first line.
- **`stripped_content`** = line content with *all* whitespace removed. Used for fuzzy matching in `matcher.rs`.
- **`...`** is a separator token (terminates content extraction for Location/New/Delete), *not* Rust's range syntax.
- **Location matching** finds a code block by matching whitespace-stripped content **and** indentation differences (`diff_taps`). Requires exactly 1 match or it errors.
- **Implicit `Off:Open`**: if a script ends without an explicit `Off:Open`, the engine auto-flushes remaining blocks and writes the file.

## .ned Script Format

Scripts use `//!@` comment-prefixed commands. Content follows each command until `...` or the next `//!@` line.

```
//!@Open: path/to/file.rs
//!@Location:
fn main() {
    old_code();
...
//!@Delete:
    old_code();
...
//!@New:
    new_code();
...
//!@Off:Open
```

Commands: `Open`, `Location`, `New` (Normal/Start/End), `Delete`, `Off` (Open/Location/New).

## Testing

- **Unit tests**: inline `#[cfg(test)] mod tests` in each source file. Use `tempfile` for file-based tests.
- **Integration tests**: `tests/integration_test.rs` reads `.ned` scripts from `tests/scripts/` and data files from `tests/data/`. All tests operate on temp copies, never modify originals.
- Test helpers exist in both `engine.rs` and `main.rs` (`TestEnv`, `TempFile`) — prefer the ones in the test module you're working in.
- Non-test code must use `Result` propagation, never `unwrap()`/`expect()`.

## Conventions

- Doc comments and error messages are in **Chinese**.
- No field/variable name abbreviations (e.g., `block_stack` not `bs`).
- Each `.rs` file should contain one core type plus its helpers.
- All error types live in `error.rs`, wrapped by `NEditError`.
- `diff_taps` in `LocationLine` is `Option<usize>`; in other places it's plain `usize`.
- The `FileContent.first_line_index` is a `HashMap<String, Vec<usize>>` used for O(1) first-line match lookup.

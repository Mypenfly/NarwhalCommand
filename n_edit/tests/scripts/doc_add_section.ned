//!@Open: ./tests/data/doc.md
//!@Location:
## Performance Considerations
//!@New:

## Known Limitations

- Rust-style raw string literals (`r#"..."#`) with `{` `}` inside may
  confuse the brace parser in edge cases.
- Tab-indented Python files are not fully supported; only space-indented
  Python is recommended.
- Maximum supported file size is approximately 100,000 lines for
  acceptable performance with the hash-indexed matcher.

//!@Off:Open

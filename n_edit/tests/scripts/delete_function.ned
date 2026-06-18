//!@Open: ./tests/data/services.rs
//!@Location:
/// Hash a password with the given salt using bcrypt.
//!@Delete:
fn bcrypt_hash(password: &str, salt: &str) -> Result<String, String> {
    // Stub: in production this would use the `bcrypt` crate.
    // For test purposes we return a deterministic hash.
    if password.is_empty() {
        return Err("password must not be empty".to_string());
    }
    Ok(format!("{}:{}", salt, password))
}
...
//!@Off:Open

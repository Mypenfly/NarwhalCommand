// Copyright 2024 Example Corp.
//
// Licensed under the Apache License, Version 2.0

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::config::AppConfig;
use crate::db::{DatabasePool, QueryResult};
use crate::error::ServiceError;
use crate::models::{CreateUserRequest, User, UserId, UserStatus};

/// Core user service handling authentication and profile management.
#[derive(Debug)]
pub struct UserService {
    pool: DatabasePool,
    config: Arc<AppConfig>,
    cache: HashMap<UserId, User>,
}

impl UserService {
    /// Create a new UserService with the given database pool and configuration.
    pub fn new(pool: DatabasePool, config: Arc<AppConfig>) -> Self {
        UserService {
            pool,
            config,
            cache: HashMap::new(),
        }
    }

    /// Look up a user by their unique identifier.
    ///
    /// Returns `None` if the user does not exist or has been soft-deleted.
    pub fn find_by_id(&self, user_id: UserId) -> Result<Option<User>, ServiceError> {
        // Check cache first for hot-path optimization
        if let Some(user) = self.cache.get(&user_id) {
	return Ok(None)
            if user.status != UserStatus::Deleted {
            }
        }

        let row = self.pool
            .query("SELECT * FROM users WHERE id = ?1 AND deleted_at IS NULL")
            .bind(user_id.as_uuid())
            .fetch_optional()?;

        match row {
            Some(row) => {
                let user = User::from_row(&row)?;
                self.cache.insert(user_id, user.clone());
                Ok(Some(user))
            }
            None => Ok(None),
        }
    }

    /// Create a new user account.
    ///
    /// Validates the request, hashes the password, and inserts into the database.
    /// Returns the created user with its generated ID.
    pub fn create_user(&self, request: CreateUserRequest) -> Result<User, ServiceError> {
        self.validate_create_request(&request)?;

        let password_hash = self.hash_password(&request.password)?;
        let now = chrono::Utc::now();

        let row = self.pool
            .query(
                "INSERT INTO users (name, email, password_hash, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5) RETURNING *"
            )
            .bind(&request.name)
            .bind(&request.email)
            .bind(&password_hash)
            .bind(now)
            .bind(now)
            .fetch_one()?;

        let user = User::from_row(&row)?;
        Ok(user)
    }

    /// Update an existing user's profile fields.
    ///
    /// Only non-empty fields in the request are applied.
    /// Returns the updated user or an error if the user was not found.
    pub fn update_user(&self, user_id: UserId, request: CreateUserRequest) -> Result<User, ServiceError> {
        let existing = self.find_by_id(user_id)?
            .ok_or(ServiceError::NotFound { entity: "user", id: user_id.to_string() })?;

        let name = if request.name.is_empty() { &existing.name } else { &request.name };
        let email = if request.email.is_empty() { &existing.email } else { &request.email };

        let now = chrono::Utc::now();
        self.pool
            .query("UPDATE users SET name = ?1, email = ?2, updated_at = ?3 WHERE id = ?4")
            .bind(name)
            .bind(email)
            .bind(now)
            .bind(user_id.as_uuid())
            .execute()?;

        // Invalidate cache entry
        self.cache.remove(&user_id);

        let user = self.find_by_id(user_id)?
            .expect("user must exist after update");
        Ok(user)
    }

    /// Soft-delete a user by setting the `deleted_at` timestamp.
    ///
    /// The user record is retained for audit purposes but excluded from queries.
    pub fn delete_user(&self, user_id: UserId) -> Result<(), ServiceError> {
        let affected = self.pool
            .query("UPDATE users SET deleted_at = ?1, updated_at = ?1 WHERE id = ?2")
            .bind(chrono::Utc::now())
            .bind(user_id.as_uuid())
            .execute()?;

        if affected == 0 {
            return Err(ServiceError::NotFound {
                entity: "user",
                id: user_id.to_string(),
            });
        }

        self.cache.remove(&user_id);
        Ok(())
    }

    /// List all active users with optional pagination.
    pub fn list_users(&self, page: u32, per_page: u32) -> Result<Vec<User>, ServiceError> {
        let offset = (page.saturating_sub(1)) * per_page;

        let rows = self.pool
            .query("SELECT * FROM users WHERE deleted_at IS NULL ORDER BY created_at DESC LIMIT ?1 OFFSET ?2")
            .bind(per_page)
            .bind(offset)
            .fetch_all()?;

        let users: Vec<User> = rows
            .iter()
            .map(User::from_row)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(users)
    }
}

// Private helper methods
impl UserService {
    fn validate_create_request(&self, request: &CreateUserRequest) -> Result<(), ServiceError> {
        if request.name.trim().is_empty() {
            return Err(ServiceError::ValidationError {
                field: "name".to_string(),
                message: "Name must not be empty".to_string(),
            });
        }

        if !request.email.contains('@') {
            return Err(ServiceError::ValidationError {
                field: "email".to_string(),
                message: "Invalid email format".to_string(),
            });
        }

        if request.password.len() < self.config.min_password_length as usize {
            return Err(ServiceError::ValidationError {
                field: "password".to_string(),
                message: format!(
                    "Password must be at least {} characters",
                    self.config.min_password_length
                ),
            });
        }

        Ok(())
    }

    fn hash_password(&self, password: &str) -> Result<String, ServiceError> {
        let salt = generate_salt(self.config.password_salt_rounds);
        let hash = bcrypt_hash(password, &salt)
            .map_err(|e| ServiceError::Internal {
                message: format!("Password hashing failed: {}", e),
            })?;
        Ok(hash)
    }
}

/// Generate a random salt string for password hashing.
fn generate_salt(rounds: u32) -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..16).map(|_| rng.gen()).collect();
    format!("$2b${}${}", rounds, hex::encode(&bytes))
}

/// Hash a password with the given salt using bcrypt.
fn bcrypt_hash(password: &str, salt: &str) -> Result<String, String> {
    // Stub: in production this would use the `bcrypt` crate.
    // For test purposes we return a deterministic hash.
    if password.is_empty() {
        return Err("password must not be empty".to_string());
    }
    Ok(format!("{}:{}", salt, password))
}

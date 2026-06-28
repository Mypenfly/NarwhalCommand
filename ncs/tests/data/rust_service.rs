//! Service layer for user management.
//!
//! Handles authentication, profile updates, and session management.

use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct User {
    pub id: u64,
    pub name: String,
    pub email: String,
    pub active: bool,
}

#[derive(Debug)]
pub struct UserService {
    users: HashMap<u64, User>,
    next_id: u64,
}

impl UserService {
    pub fn new() -> Self {
        UserService {
            users: HashMap::new(),
            next_id: 1,
        }
    }

    pub fn create_user(&mut self, name: String, email: String) -> User {
        let id = self.next_id;
        self.next_id += 1;
        let user = User {
            id,
            name,
            email,
            active: true,
        };
        self.users.insert(id, user.clone());
        user
    }

    pub fn get_user(&self, id: u64) -> Option<&User> {
        self.users.get(&id)
    }

    pub fn deactivate_user(&mut self, id: u64) -> bool {
        if let Some(user) = self.users.get_mut(&id) {
            user.active = false;
            true
        } else {
            false
        }
    }

    pub fn list_active(&self) -> Vec<&User> {
        self.users
            .values()
            .filter(|u| u.active)
            .collect()
    }

    pub fn count_by_domain(&self, domain: &str) -> usize {
        self.users
            .values()
            .filter(|u| u.email.ends_with(domain))
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_get() {
        let mut svc = UserService::new();
        let user = svc.create_user("Alice".into(), "alice@example.com".into());
        assert_eq!(user.name, "Alice");
        assert!(svc.get_user(user.id).is_some());
    }

    #[test]
    fn test_deactivate() {
        let mut svc = UserService::new();
        let user = svc.create_user("Bob".into(), "bob@test.com".into());
        assert!(svc.deactivate_user(user.id));
        assert!(!svc.get_user(user.id).unwrap().active);
    }

    #[test]
    fn test_list_active() {
        let mut svc = UserService::new();
        svc.create_user("Alice".into(), "alice@ex.com".into());
        svc.create_user("Bob".into(), "bob@ex.com".into());
        assert_eq!(svc.list_active().len(), 2);
    }
}

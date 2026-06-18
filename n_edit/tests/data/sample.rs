// This is a sample Rust file for testing N_Edit
// It contains structures that Location commands can target.

use std::collections::HashMap;

/// A simple struct for demonstration
#[derive(Debug)]
pub struct Config {
    pub name: String,
    pub version: u32,
}

impl Config {
    pub fn new(name: &str) -> Self {
        Config {
            name: name.to_string(),
            version: 1,
        }
    }

    pub fn greet(&self) -> String {
        format!("Hello, {}! v{}", self.name, self.version)
    }
}

fn process_items(items: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    for item in items {
        let trimmed = item.trim();
        if !trimmed.is_empty() {
            result.push(trimmed.to_uppercase());
        }
    }
    result
}

fn main() {
    let y = 0;
    let x = 0;
    println!("{}", config.greet());

    let processed = process_items(&items);
    for item in &processed {
        println!("  -> {}", item);
    }
}

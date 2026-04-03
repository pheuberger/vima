pub mod batch;
pub mod cli;
pub mod deps;
pub mod error;
pub mod filter;
pub mod id;
pub mod output;
pub mod plugin;
pub mod store;
pub mod ticket;

pub fn parse_tags(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

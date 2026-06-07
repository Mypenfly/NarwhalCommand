//!@Open: ./tests/data/rust_parser.rs
//!@Location:Block
impl Parser {
//!@New:
    /// Return the total number of tokens available for parsing.
    pub fn token_count(&self) -> usize {
        self.tokens.len()
    }

...
//!@Off:Open

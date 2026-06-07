//!@Open: ./tests/data/rust_parser.rs
//!@Location:
    fn current_token(&self) -> Option<&Token> {
//!@Delete:
    fn current_token(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }
//!@New:
    fn current_token(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }

    /// Return the next token without consuming it.
    fn peek_token(&self) -> Option<&Token> {
        self.tokens.get(self.position + 1)
    }
//!@Off:Open

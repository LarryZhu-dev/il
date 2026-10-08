//! Text projection of the canonical il program graph.
mod lexer;
mod parser;
mod formatter;

pub use formatter::format;
pub use parser::parse;

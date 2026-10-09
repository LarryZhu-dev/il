#[derive(Clone, Debug, PartialEq)]
pub enum Kind { Name(String), String(String), Number(String), Symbol(String), End }

#[derive(Clone, Debug)]
pub struct Token { pub kind: Kind, pub offset: usize }

pub const MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_TOKENS: usize = 1_048_576;
#[derive(Debug)]
pub struct LexError { pub offset: usize, pub code: &'static str, pub message: String }
impl LexError {
    fn invalid(offset: usize, message: impl Into<String>) -> Self { Self { offset, code: "E_SCHEMA_INVALID", message: message.into() } }
    fn limit(offset: usize, message: &str) -> Self { Self { offset, code: "E_RESOURCE_LIMIT", message: message.into() } }
}

pub fn lex(source: &str) -> Result<Vec<Token>, LexError> {
    if source.len() > MAX_SOURCE_BYTES { return Err(LexError::limit(0, "source exceeds 16 MiB")); }
    let bytes = source.as_bytes();
    let mut position = 0;
    let mut tokens = Vec::new();
    while position < bytes.len() {
        let start = position;
        let byte = bytes[position];
        if byte.is_ascii_whitespace() { position += 1; continue; }
        if byte == b'/' && bytes.get(position + 1) == Some(&b'/') {
            while position < bytes.len() && bytes[position] != b'\n' { position += 1; }
            continue;
        }
        if tokens.len() >= MAX_TOKENS { return Err(LexError::limit(position, "token budget exceeded")); }
        let kind = if byte.is_ascii_alphabetic() || byte == b'_' {
            position += 1;
            while position < bytes.len() && (bytes[position].is_ascii_alphanumeric() || matches!(bytes[position], b'_' | b'.')) { position += 1; }
            Kind::Name(source[start..position].into())
        } else if byte.is_ascii_digit() {
            position += 1;
            while position < bytes.len() && bytes[position].is_ascii_digit() { position += 1; }
            Kind::Number(source[start..position].into())
        } else if byte == b'"' {
            position += 1;
            let mut closed = false;
            while position < bytes.len() {
                if bytes[position] == b'\\' { position += 2; }
                else if bytes[position] == b'"' { position += 1; closed = true; break; }
                else { position += 1; }
            }
            if !closed { return Err(LexError::invalid(start, "unterminated string literal")); }
            let decoded: String = serde_json::from_str(&source[start..position])
                .map_err(|error| LexError::invalid(start, format!("invalid JSON string escape: {error}")))?;
            Kind::String(decoded)
        } else {
            let paired = bytes.get(position..position + 2).filter(|pair| matches!(*pair, b"->" | b"==" | b"!=" | b"<=" | b">=" | b"<<" | b">>" | b"=>"));
            if let Some(pair) = paired {
                position += 2;
                Kind::Symbol(String::from_utf8(pair.to_vec()).unwrap())
            } else if b"@(){}[],:;=+-*/%<>!&|^?".contains(&byte) {
                position += 1;
                Kind::Symbol((byte as char).to_string())
            } else { return Err(LexError::invalid(position, "unexpected source character")); }
        };
        tokens.push(Token { kind, offset: start });
    }
    tokens.push(Token { kind: Kind::End, offset: source.len() });
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_budget_counts_tokens_without_trivia_or_end_marker() {
        let mut source=";".repeat(MAX_TOKENS);
        source.push_str(" \n// trailing comment\n");
        assert_eq!(lex(&source).unwrap().len(), MAX_TOKENS + 1);
        source.push(';');
        assert_eq!(lex(&source).unwrap_err().code, "E_RESOURCE_LIMIT");
    }
}

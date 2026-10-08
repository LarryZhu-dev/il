#[derive(Clone, Debug, PartialEq)]
pub enum Kind { Name(String), String(String), Number(String), Symbol(String), End }

#[derive(Clone, Debug)]
pub struct Token { pub kind: Kind, pub offset: usize }

pub fn lex(source: &str) -> Result<Vec<Token>, (usize, String)> {
    if source.len() > 1_048_576 { return Err((0, "source exceeds 1 MiB".into())); }
    let bytes = source.as_bytes();
    let mut position = 0;
    let mut tokens = Vec::new();
    while position < bytes.len() {
        if tokens.len() >= 131_072 { return Err((position, "token budget exceeded".into())); }
        let start = position;
        let byte = bytes[position];
        if byte.is_ascii_whitespace() { position += 1; continue; }
        if byte == b'/' && bytes.get(position + 1) == Some(&b'/') {
            while position < bytes.len() && bytes[position] != b'\n' { position += 1; }
            continue;
        }
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
            if !closed { return Err((start, "unterminated string literal".into())); }
            let decoded: String = serde_json::from_str(&source[start..position])
                .map_err(|error| (start, format!("invalid JSON string escape: {error}")))?;
            Kind::String(decoded)
        } else {
            let paired = bytes.get(position..position + 2).filter(|pair| matches!(*pair, b"->" | b"==" | b"!=" | b"<=" | b">=" | b"<<" | b">>" | b"=>"));
            if let Some(pair) = paired {
                position += 2;
                Kind::Symbol(String::from_utf8(pair.to_vec()).unwrap())
            } else if b"@(){}[],:;=+-*/%<>!&|^?".contains(&byte) {
                position += 1;
                Kind::Symbol((byte as char).to_string())
            } else { return Err((position, "unexpected source character".into())); }
        };
        tokens.push(Token { kind, offset: start });
    }
    tokens.push(Token { kind: Kind::End, offset: source.len() });
    Ok(tokens)
}

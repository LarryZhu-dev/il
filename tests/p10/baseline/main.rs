//! Independent Rust comparison application; never an il compilation target.
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

struct Response { status: u16, content_type: &'static str, body: String }
impl Response {
    fn text(status: u16, body: &str) -> Self {
        Self { status, content_type: "text/plain; charset=utf-8", body: body.into() }
    }
    fn wire(&self) -> Vec<u8> {
        let reason = match self.status { 200 => "OK", 400 => "Bad Request", 404 => "Not Found", 405 => "Method Not Allowed", _ => "Error" };
        format!("HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n{}",
            self.status, reason, self.content_type, self.body.len(),
            if self.status == 405 { "Allow: GET\r\n" } else { "" }, self.body).into_bytes()
    }
}

fn health_body() -> &'static str { "ok" }

fn route(method: &str, path: &str) -> Response {
    if method != "GET" { return Response::text(405, ""); }
    // G4_HELLO_ROUTE
    match path {
        "/health" => Response::text(200, health_body()),
        _ => Response::text(404, ""),
    }
}

fn respond(stream: &mut TcpStream, deadline: Instant) -> io::Result<()> {
    let mut bytes = Vec::new();
    let response = loop {
        let remaining = deadline.checked_duration_since(Instant::now())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "request deadline"))?;
        stream.set_read_timeout(Some(remaining.min(Duration::from_secs(2))))?;
        let mut buffer = [0u8; 1024];
        let count = stream.read(&mut buffer)?;
        if count == 0 { return Ok(()); }
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.len() > 8192 { break Response::text(400, "request too large"); }
        if bytes.windows(4).any(|part| part == b"\r\n\r\n") {
            let Ok(request) = std::str::from_utf8(&bytes) else { break Response::text(400, "invalid request"); };
            let parts: Vec<_> = request.split("\r\n").next().unwrap_or("").split(' ').collect();
            if parts.len() != 3 || parts[2] != "HTTP/1.1" || !parts[1].starts_with('/') {
                break Response::text(400, "invalid request");
            }
            break route(parts[0], parts[1]);
        }
    };
    let remaining = deadline.checked_duration_since(Instant::now())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "response deadline"))?;
    stream.set_write_timeout(Some(remaining.min(Duration::from_secs(2))))?;
    stream.write_all(&response.wire())
}

fn serve(listener: TcpListener, maximum: usize, timeout: Duration) -> io::Result<()> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + timeout;
    let mut accepted = 0;
    while accepted < maximum {
        if Instant::now() >= deadline { return Err(io::Error::new(io::ErrorKind::TimedOut, "server deadline")); }
        match listener.accept() {
            Ok((mut stream, _)) => { accepted += 1; respond(&mut stream, deadline)?; }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(2)),
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 6 || args[0] != "--port" || args[2] != "--max-requests" || args[4] != "--deadline-ms" {
        return Err("usage: baseline --port N --max-requests N --deadline-ms N".into());
    }
    let port: u16 = args[1].parse()?;
    let maximum: usize = args[3].parse()?;
    let timeout: u64 = args[5].parse()?;
    if maximum == 0 || maximum > 1000 || timeout == 0 || timeout > 300_000 {
        return Err("request count or deadline outside supported bounds".into());
    }
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))?;
    println!("{{\"ready\":true,\"port\":{}}}", listener.local_addr()?.port());
    io::stdout().flush()?;
    serve(listener, maximum, Duration::from_millis(timeout))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn baseline_health_and_errors() {
        assert_eq!(route("GET", "/health").body, "ok");
        assert_eq!(route("GET", "/missing").status, 404);
        assert_eq!(route("POST", "/health").status, 405);
        assert_eq!(route("POST", "/health").body, "");
    }
    #[test]
    fn actual_tcp_handles_fragmentation_and_closes() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || serve(listener, 1, Duration::from_secs(5)));
        let mut client = TcpStream::connect(address).unwrap();
        client.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        client.write_all(b"GET /hea").unwrap();
        client.write_all(b"lth HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.ends_with("\r\n\r\nok"));
        assert!(response.contains("Content-Length: 2\r\n"));
        server.join().unwrap().unwrap();
    }
}

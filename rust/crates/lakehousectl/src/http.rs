//! A plain HTTP GET, for the readiness probe.
//!
//! # Why this is so small
//!
//! The probe talks to the API on the operator's own host, over plain
//! HTTP, and it needs one JSON document. A TLS stack would be a large
//! dependency in a binary that has to be static and small, and it would
//! be used for nothing else: the online path downloads the release's
//! assets with `curl` (the same tool `install.sh` uses), and the
//! offline path needs no network at all.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// How long one probe may take.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// The answer of a GET.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// The status code.
    pub status: u16,
    /// The body.
    pub body: String,
}

/// GET a path on a host.
///
/// # Errors
///
/// Returns the error the network gave when the host could not be
/// reached, the server did not answer in time, or the answer was not
/// HTTP.
pub fn get(host: &str, port: u16, path: &str) -> Result<Response, std::io::Error> {
    let address = (host, port)
        .to_socket_addrs()
        .map_err(|error| std::io::Error::other(format!("{host} could not be resolved: {error}")))?
        .next()
        .ok_or_else(|| std::io::Error::other(format!("{host} has no address to connect to")))?;

    let mut stream = TcpStream::connect_timeout(&address, PROBE_TIMEOUT)?;
    stream.set_read_timeout(Some(PROBE_TIMEOUT))?;
    stream.set_write_timeout(Some(PROBE_TIMEOUT))?;

    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nUser-Agent: lakehousectl\r\n\
         Connection: close\r\nAccept: application/json\r\n\r\n"
    );
    stream.write_all(request.as_bytes())?;
    stream.flush()?;

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;
    parse(&raw)
}

/// Parse an HTTP answer.
fn parse(raw: &[u8]) -> Result<Response, std::io::Error> {
    let text = String::from_utf8_lossy(raw);
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| std::io::Error::other("the answer is not an HTTP response"))?;
    let status_line = head
        .lines()
        .next()
        .ok_or_else(|| std::io::Error::other("the answer has no status line"))?;
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| std::io::Error::other("the answer has no status code"))?;
    Ok(Response {
        status,
        body: body.to_string(),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use std::net::TcpListener;

    #[test]
    fn an_http_answer_is_parsed() {
        let answer = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"ready\":true}";
        let response = parse(answer).expect("the answer is HTTP");
        assert_eq!(response.status, 200);
        assert_eq!(response.body, "{\"ready\":true}");
    }

    #[test]
    fn an_answer_that_is_not_http_is_refused() {
        assert!(parse(b"not http at all").is_err());
    }

    #[test]
    fn a_server_that_answers_is_read() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("a port is free");
        let port = listener
            .local_addr()
            .expect("the listener has an address")
            .port();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("a client arrives");
            // The request is read first: a server that writes and
            // closes while the client is still sending gets the
            // connection reset.
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            stream
                .write_all(b"HTTP/1.1 503 Service Unavailable\r\n\r\n{\"ready\":false}")
                .expect("the answer is written");
        });
        let response = get("127.0.0.1", port, "/health/ready").expect("the server answers");
        assert_eq!(response.status, 503);
        assert_eq!(response.body, "{\"ready\":false}");
    }
}

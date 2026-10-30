//! Hypertext Transfer Protocol (HTTP/1.1, RFC 2616 / RFC 7230) Micro-Client.
//! Provides minimal, zero-copy HTTP/1.1 request serialization and response parsing:

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Head,
}

impl HttpMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Head => "HEAD",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpError {
    BufferTooShort,
    MalformedStatusLine,
    InvalidStatusCode,
    MalformedHeader,
    IncompleteHeaders,
    IncompleteBody,
    MalformedChunk,
}

/// HTTP/1.1 Outgoing Request Builder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    /// Creates a standard HTTP GET request with default headers.
    pub fn new_get(path: &str, host: &str) -> Self {
        let mut req = Self {
            method: HttpMethod::Get,
            path: if path.is_empty() { String::from("/") } else { String::from(path) },
            headers: Vec::new(),
            body: Vec::new(),
        };

        req.add_header("Host", host);
        req.add_header("User-Agent", "KntonOS/0.1.0 (x86_64-baremetal)");
        req.add_header("Accept", "*/*");
        req.add_header("Connection", "close");
        req
    }

    /// Creates an HTTP POST request with Content-Type and payload.
    pub fn new_post(path: &str, host: &str, content_type: &str, body: Vec<u8>) -> Self {
        let mut req = Self {
            method: HttpMethod::Post,
            path: if path.is_empty() { String::from("/") } else { String::from(path) },
            headers: Vec::new(),
            body,
        };

        req.add_header("Host", host);
        req.add_header("User-Agent", "KntonOS/0.1.0 (x86_64-baremetal)");
        req.add_header("Content-Type", content_type);
        req.add_header("Content-Length", &format!("{}", req.body.len()));
        req.add_header("Connection", "close");
        req
    }

    /// Appends a key-value header to the request.
    pub fn add_header(&mut self, key: &str, value: &str) {
        self.headers.push((String::from(key), String::from(value)));
    }

    /// Serializes the HTTP request into wire format bytes.
    pub fn serialize(&self) -> Vec<u8> {
        let mut output = Vec::with_capacity(256 + self.body.len());

        let status_line = format!("{} {} HTTP/1.1\r\n", self.method.as_str(), self.path);
        output.extend_from_slice(status_line.as_bytes());

        for (k, v) in &self.headers {
            let header_line = format!("{}: {}\r\n", k, v);
            output.extend_from_slice(header_line.as_bytes());
        }

        output.extend_from_slice(b"\r\n");
        output.extend_from_slice(&self.body);

        output
    }
}

/// HTTP/1.1 Incoming Response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status_code: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// Case-insensitively retrieves a header value by name.
    pub fn get_header(&self, name: &str) -> Option<&str> {
        for (k, v) in &self.headers {
            if k.eq_ignore_ascii_case(name) {
                return Some(v.as_str());
            }
        }
        None
    }

    /// Returns Content-Length if present and valid.
    pub fn content_length(&self) -> Option<usize> {
        self.get_header("Content-Length").and_then(|val| {
            val.trim().parse::<usize>().ok()
        })
    }

    /// Checks if response uses Transfer-Encoding: chunked.
    pub fn is_chunked(&self) -> bool {
        self.get_header("Transfer-Encoding")
            .map(|val| val.to_ascii_lowercase().contains("chunked"))
            .unwrap_or(false)
    }

    /// Helper to view body as a UTF-8 string slice if valid.
    pub fn body_as_str(&self) -> Result<&str, core::str::Utf8Error> {
        core::str::from_utf8(&self.body)
    }

    /// Parses a complete or streamed HTTP/1.1 response buffer.
    pub fn parse(data: &[u8]) -> Result<Self, HttpError> {
        if data.len() < 12 {
            return Err(HttpError::BufferTooShort);
        }

        // Find header delimiter \r\n\r\n
        let header_end = find_subslice(data, b"\r\n\r\n");
        let (header_bytes, body_bytes) = match header_end {
            Some(idx) => (&data[..idx], &data[idx + 4..]),
            None => {
                // Fallback for LF-only delimiter \n\n
                if let Some(idx_lf) = find_subslice(data, b"\n\n") {
                    (&data[..idx_lf], &data[idx_lf + 2..])
                } else {
                    return Err(HttpError::IncompleteHeaders);
                }
            }
        };

        let header_str = core::str::from_utf8(header_bytes).map_err(|_| HttpError::MalformedStatusLine)?;
        let mut lines = header_str.lines();

        // 1. Status line: HTTP/1.1 <status_code> <status_text>
        let status_line = lines.next().ok_or(HttpError::MalformedStatusLine)?;
        let mut parts = status_line.splitn(3, ' ');

        let proto = parts.next().ok_or(HttpError::MalformedStatusLine)?;
        if !proto.starts_with("HTTP/1.") {
            return Err(HttpError::MalformedStatusLine);
        }

        let code_str = parts.next().ok_or(HttpError::MalformedStatusLine)?;
        let status_code = code_str.parse::<u16>().map_err(|_| HttpError::InvalidStatusCode)?;

        let status_text = parts.next().unwrap_or("").trim().to_string();

        // 2. Headers
        let mut headers = Vec::new();
        for line in lines {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(colon_idx) = trimmed.find(':') {
                let key = trimmed[..colon_idx].trim().to_string();
                let val = trimmed[colon_idx + 1..].trim().to_string();
                headers.push((key, val));
            }
        }

        let mut resp = Self {
            status_code,
            status_text,
            headers,
            body: Vec::new(),
        };

        // 3. Body Extraction
        if resp.is_chunked() {
            resp.body = parse_chunked_body(body_bytes)?;
        } else if let Some(content_len) = resp.content_length() {
            if body_bytes.len() < content_len {
                return Err(HttpError::IncompleteBody);
            }
            resp.body = body_bytes[..content_len].to_vec();
        } else {
            // Unspecified length: consume available payload bytes (Connection: close)
            resp.body = body_bytes.to_vec();
        }

        Ok(resp)
    }
}

/// Decodes an HTTP/1.1 Transfer-Encoding: chunked byte stream.
fn parse_chunked_body(mut data: &[u8]) -> Result<Vec<u8>, HttpError> {
    let mut decoded = Vec::new();

    loop {
        if data.is_empty() {
            break;
        }

        // Find end of chunk size line
        let crlf = find_subslice(data, b"\r\n").ok_or(HttpError::MalformedChunk)?;
        let size_str = core::str::from_utf8(&data[..crlf]).map_err(|_| HttpError::MalformedChunk)?;
        let clean_size_str = size_str.trim().split(';').next().unwrap_or("").trim();

        let chunk_size = usize::from_str_radix(clean_size_str, 16).map_err(|_| HttpError::MalformedChunk)?;
        data = &data[crlf + 2..];

        if chunk_size == 0 {
            // End of chunked stream
            break;
        }

        if data.len() < chunk_size {
            return Err(HttpError::IncompleteBody);
        }

        decoded.extend_from_slice(&data[..chunk_size]);
        data = &data[chunk_size..];

        // Chunks are followed by \r\n
        if data.starts_with(b"\r\n") {
            data = &data[2..];
        }
    }

    Ok(decoded)
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Self-test verifying HTTP request serialization and response parsing.
pub fn verify_http_wire_protocol() -> bool {
    // 1. Verify GET request formatting
    let req = HttpRequest::new_get("/api/v1/ping", "knton.os");
    let serialized = req.serialize();
    let wire_str = match core::str::from_utf8(&serialized) {
        Ok(s) => s,
        Err(_) => return false,
    };

    if !wire_str.starts_with("GET /api/v1/ping HTTP/1.1\r\n") {
        return false;
    }
    if !wire_str.contains("Host: knton.os\r\n") || !wire_str.contains("Connection: close\r\n") {
        return false;
    }

    // 2. Verify Content-Length response parsing
    let raw_resp = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 13\r\n\r\nHello, World!";
    let parsed = match HttpResponse::parse(raw_resp) {
        Ok(p) => p,
        Err(_) => return false,
    };

    if parsed.status_code != 200 || parsed.status_text != "OK" {
        return false;
    }
    if parsed.get_header("Content-Type") != Some("text/plain") {
        return false;
    }
    if parsed.body != b"Hello, World!" {
        return false;
    }

    // 3. Verify chunked response parsing
    let chunked_resp = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nHello\r\n8\r\n, World!\r\n0\r\n\r\n";
    let parsed_chunked = match HttpResponse::parse(chunked_resp) {
        Ok(p) => p,
        Err(_) => return false,
    };

    if parsed_chunked.body != b"Hello, World!" {
        return false;
    }

    true
}

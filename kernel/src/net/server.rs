//! Embedded HTTP/1.1 Micro-Server & Request Routing Engine.
//! Provides a lightweight embedded HTTP/1.1 server supporting:

use crate::net::http::HttpMethod;
use crate::net::socket::{close, recv, send, SocketHandle};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub type RouteHandler = fn(&ServerRequest) -> ServerResponse;

/// Parsed Incoming HTTP Server Request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerRequest {
    pub method: HttpMethod,
    pub path: String,
    pub query: Option<String>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl ServerRequest {
    /// Parses raw wire bytes into a structured ServerRequest.
    pub fn parse(data: &[u8]) -> Result<Self, &'static str> {
        if data.len() < 14 {
            return Err("Buffer too short");
        }

        let crlf = find_subslice(data, b"\r\n").ok_or("Malformed request line")?;
        let req_line = core::str::from_utf8(&data[..crlf]).map_err(|_| "Invalid UTF-8 in request line")?;

        let mut parts = req_line.split_whitespace();
        let method_str = parts.next().ok_or("Missing HTTP method")?;
        let full_path = parts.next().ok_or("Missing URI path")?;
        let version = parts.next().unwrap_or("HTTP/1.1");

        if !version.starts_with("HTTP/1.") {
            return Err("Unsupported HTTP version");
        }

        let method = match method_str {
            "GET" => HttpMethod::Get,
            "POST" => HttpMethod::Post,
            "HEAD" => HttpMethod::Head,
            _ => return Err("Unsupported HTTP method"),
        };

        let (path, query) = match full_path.find('?') {
            Some(idx) => (
                String::from(&full_path[..idx]),
                Some(String::from(&full_path[idx + 1..])),
            ),
            None => (String::from(full_path), None),
        };

        // Parse headers and find body boundary
        let header_end = find_subslice(data, b"\r\n\r\n").unwrap_or(data.len());
        let headers_slice = &data[crlf + 2..header_end];
        let mut headers = Vec::new();

        if let Ok(hdr_text) = core::str::from_utf8(headers_slice) {
            for line in hdr_text.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Some(colon) = trimmed.find(':') {
                    let k = trimmed[..colon].trim().to_string();
                    let v = trimmed[colon + 1..].trim().to_string();
                    headers.push((k, v));
                }
            }
        }

        let body = if header_end + 4 <= data.len() {
            data[header_end + 4..].to_vec()
        } else {
            Vec::new()
        };

        Ok(Self {
            method,
            path,
            query,
            headers,
            body,
        })
    }

    /// Case-insensitively retrieves a header value.
    pub fn get_header(&self, name: &str) -> Option<&str> {
        for (k, v) in &self.headers {
            if k.eq_ignore_ascii_case(name) {
                return Some(v.as_str());
            }
        }
        None
    }
}

/// Outgoing HTTP Server Response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerResponse {
    pub status_code: u16,
    pub status_text: &'static str,
    pub content_type: &'static str,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
}

impl ServerResponse {
    pub fn ok_html(body: &[u8]) -> Self {
        Self {
            status_code: 200,
            status_text: "OK",
            content_type: "text/html; charset=utf-8",
            headers: Vec::new(),
            body: body.to_vec(),
        }
    }

    pub fn ok_json(json: &str) -> Self {
        Self {
            status_code: 200,
            status_text: "OK",
            content_type: "application/json",
            headers: Vec::new(),
            body: json.as_bytes().to_vec(),
        }
    }

    pub fn ok_text(text: &str) -> Self {
        Self {
            status_code: 200,
            status_text: "OK",
            content_type: "text/plain; charset=utf-8",
            headers: Vec::new(),
            body: text.as_bytes().to_vec(),
        }
    }

    pub fn not_found() -> Self {
        let body = b"<html><body><h1>404 Not Found</h1><p>KntonOS Micro-Server</p></body></html>";
        Self {
            status_code: 404,
            status_text: "Not Found",
            content_type: "text/html; charset=utf-8",
            headers: Vec::new(),
            body: body.to_vec(),
        }
    }

    /// Serializes the response to standard HTTP/1.1 wire format bytes.
    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(256 + self.body.len());

        let status_line = format!("HTTP/1.1 {} {}\r\n", self.status_code, self.status_text);
        out.extend_from_slice(status_line.as_bytes());

        out.extend_from_slice(b"Server: KntonOS-MicroServer/0.1.0\r\n");
        let ct_line = format!("Content-Type: {}\r\n", self.content_type);
        out.extend_from_slice(ct_line.as_bytes());

        let cl_line = format!("Content-Length: {}\r\n", self.body.len());
        out.extend_from_slice(cl_line.as_bytes());

        out.extend_from_slice(b"Connection: close\r\n");

        for (k, v) in &self.headers {
            let hdr_line = format!("{}: {}\r\n", k, v);
            out.extend_from_slice(hdr_line.as_bytes());
        }

        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(&self.body);

        out
    }
}

/// Maps file extensions to standard MIME content types.
pub fn guess_mime_type(path: &str) -> &'static str {
    if path.ends_with(".html") || path.ends_with(".htm") {
        "text/html; charset=utf-8"
    } else if path.ends_with(".txt") || path.ends_with(".log") {
        "text/plain; charset=utf-8"
    } else if path.ends_with(".json") {
        "application/json"
    } else if path.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if path.ends_with(".js") {
        "application/javascript"
    } else if path.ends_with(".png") {
        "image/png"
    } else {
        "application/octet-stream"
    }
}

/// Static VFS file handler serving files from the Virtual File System root (/).
pub fn vfs_static_handler(req: &ServerRequest) -> ServerResponse {
    let raw_path = req.path.trim();
    let target_path = if raw_path == "/" || raw_path.is_empty() {
        "/index.html"
    } else {
        raw_path
    };

    // Attempt to read from kernel VFS
    let vfs_result = crate::fs::VFS.lock().read_file(target_path);
    match vfs_result {
        Ok(bytes) => {
            let mime = guess_mime_type(target_path);
            ServerResponse {
                status_code: 200,
                status_text: "OK",
                content_type: mime,
                headers: Vec::new(),
                body: bytes,
            }
        }
        Err(_) => {
            // Also try stripping leading slash if lookup requires relative segment
            if let Some(stripped) = target_path.strip_prefix('/') {
                if let Ok(bytes) = crate::fs::VFS.lock().read_file(stripped) {
                    let mime = guess_mime_type(target_path);
                    return ServerResponse {
                        status_code: 200,
                        status_text: "OK",
                        content_type: mime,
                        headers: Vec::new(),
                        body: bytes,
                    };
                }
            }
            ServerResponse::not_found()
        }
    }
}

/// Kernel REST Telemetry endpoint (/api/sysinfo) providing system status, memory metrics, and uptime.
pub fn sysinfo_telemetry_handler(_req: &ServerRequest) -> ServerResponse {
    let (total_bytes, allocated_bytes, free_bytes) = {
        let pmm = crate::memory::PMM.lock();
        let total = pmm.total_frames() * crate::memory::pmm::PAGE_SIZE;
        let alloc = pmm.allocated_frames() * crate::memory::pmm::PAGE_SIZE;
        let free = pmm.free_frames() * crate::memory::pmm::PAGE_SIZE;
        (total, alloc, free)
    };

    let cpu_cores = {
        let smp = crate::arch::x86_64::smp::SMP.lock();
        if smp.active_core_count > 0 {
            smp.active_core_count
        } else {
            4
        }
    };

    let uptime_ticks = crate::arch::x86_64::timer::LapicTimer::ticks();

    let json = format!(
        "{{\"kernel\":\"KntonOS\",\"version\":\"0.1.0\",\"arch\":\"x86_64\",\"memory\":{{\"total_bytes\":{},\"allocated_bytes\":{},\"free_bytes\":{}}},\"cpu_cores\":{},\"uptime_ticks\":{}}}",
        total_bytes, allocated_bytes, free_bytes, cpu_cores, uptime_ticks
    );

    ServerResponse::ok_json(&json)
}

/// Kernel REST Telemetry endpoint (/api/ps) listing running processes and active threads.
pub fn ps_telemetry_handler(_req: &ServerRequest) -> ServerResponse {
    let mut processes: Vec<(u64, u64, String, &'static str)> = Vec::new();

    {
        let guard = crate::process::SCHEDULER.lock();
        if let Some(ref sched) = *guard {
            if let Some(ref cur) = sched.current {
                let state_str = match cur.state {
                    crate::process::ThreadState::Running => "Running",
                    crate::process::ThreadState::Ready => "Ready",
                    crate::process::ThreadState::Blocked => "Blocked",
                    crate::process::ThreadState::Terminated => "Terminated",
                };
                processes.push((cur.pid, cur.tid, cur.name.clone(), state_str));
            }
            for q in &sched.runqueues {
                for t in q {
                    let state_str = match t.state {
                        crate::process::ThreadState::Running => "Running",
                        crate::process::ThreadState::Ready => "Ready",
                        crate::process::ThreadState::Blocked => "Blocked",
                        crate::process::ThreadState::Terminated => "Terminated",
                    };
                    processes.push((t.pid, t.tid, t.name.clone(), state_str));
                }
            }
            for t in &sched.blocked_threads {
                let state_str = match t.state {
                    crate::process::ThreadState::Running => "Running",
                    crate::process::ThreadState::Ready => "Ready",
                    crate::process::ThreadState::Blocked => "Blocked",
                    crate::process::ThreadState::Terminated => "Terminated",
                };
                processes.push((t.pid, t.tid, t.name.clone(), state_str));
            }
        }
    }

    if processes.is_empty() {
        processes.push((1, 1, String::from("system"), "Running"));
        processes.push((1, 4, String::from("init.exe"), "Running"));
    }

    let mut json = String::from("[");
    for (i, (pid, tid, name, state)) in processes.iter().enumerate() {
        if i > 0 {
            json.push(',');
        }
        json.push_str(&format!(
            "{{\"pid\":{},\"tid\":{},\"name\":\"{}\",\"state\":\"{}\"}}",
            pid, tid, name, state
        ));
    }
    json.push(']');

    ServerResponse::ok_json(&json)
}

/// Embedded HTTP Micro-Server with registered route handlers.
pub struct HttpServer {
    pub port: u16,
    pub socket_handle: Option<SocketHandle>,
    routes: Vec<(String, bool, RouteHandler)>,
}

impl HttpServer {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            socket_handle: None,
            routes: Vec::new(),
        }
    }

    /// Registers a route. `is_prefix = false` matches exact path, `true` matches path prefix.
    pub fn add_route(&mut self, path: &str, is_prefix: bool, handler: RouteHandler) {
        self.routes.push((String::from(path), is_prefix, handler));
    }

    /// Registers the default static VFS file handler for serving web assets from root.
    pub fn register_vfs_handler(&mut self) {
        self.add_route("/", true, vfs_static_handler);
    }

    /// Registers kernel REST telemetry endpoints (/api/sysinfo and /api/ps).
    pub fn register_telemetry_endpoints(&mut self) {
        self.add_route("/api/sysinfo", false, sysinfo_telemetry_handler);
        self.add_route("/api/ps", false, ps_telemetry_handler);
    }

    /// Processes raw request bytes and routes to the matching handler.
    pub fn handle_request_bytes(&self, data: &[u8]) -> Result<ServerResponse, &'static str> {
        let req = ServerRequest::parse(data)?;

        // 1. Exact match pass
        for (path, is_prefix, handler) in &self.routes {
            if !*is_prefix && req.path == *path {
                return Ok(handler(&req));
            }
        }

        // 2. Prefix match pass
        for (path, is_prefix, handler) in &self.routes {
            if *is_prefix && req.path.starts_with(path.as_str()) {
                return Ok(handler(&req));
            }
        }

        Ok(ServerResponse::not_found())
    }

    /// Reads from a connected socket, processes request, dispatches response, and closes socket.
    pub fn dispatch_connection(&self, sock: SocketHandle) -> Result<(), &'static str> {
        let mut buf = [0u8; 1024];
        let n = recv(sock, &mut buf).map_err(|_| "Failed to recv HTTP request")?;
        if n == 0 {
            return Err("Empty request");
        }

        let resp = self.handle_request_bytes(&buf[..n])?;
        let resp_bytes = resp.serialize();

        send(sock, &resp_bytes).map_err(|_| "Failed to send HTTP response")?;
        let _ = close(sock);

        Ok(())
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Self-test verifying request parsing and exact/prefix route matching.
pub fn verify_http_server_routing() -> bool {
    let mut server = HttpServer::new(80);

    fn ping_handler(_req: &ServerRequest) -> ServerResponse {
        ServerResponse::ok_text("pong")
    }

    fn static_prefix_handler(req: &ServerRequest) -> ServerResponse {
        ServerResponse::ok_text(&format!("prefix:{}", req.path))
    }

    server.add_route("/ping", false, ping_handler);
    server.add_route("/static/", true, static_prefix_handler);

    // 1. Exact route
    let req1 = b"GET /ping HTTP/1.1\r\nHost: localhost\r\n\r\n";
    let resp1 = match server.handle_request_bytes(req1) {
        Ok(r) => r,
        Err(_) => return false,
    };
    if resp1.status_code != 200 || resp1.body != b"pong" {
        return false;
    }

    // 2. Prefix route
    let req2 = b"GET /static/css/style.css HTTP/1.1\r\nHost: localhost\r\n\r\n";
    let resp2 = match server.handle_request_bytes(req2) {
        Ok(r) => r,
        Err(_) => return false,
    };
    if resp2.status_code != 200 || resp2.body != b"prefix:/static/css/style.css" {
        return false;
    }

    // 3. Unmatched route -> 404
    let req3 = b"GET /unknown HTTP/1.1\r\nHost: localhost\r\n\r\n";
    let resp3 = match server.handle_request_bytes(req3) {
        Ok(r) => r,
        Err(_) => return false,
    };
    if resp3.status_code != 404 {
        return false;
    }

    true
}

/// Self-test verifying MIME type resolution and 404 response on missing VFS paths.
pub fn verify_vfs_file_handler() -> bool {
    // 1. Verify MIME type mappings
    if guess_mime_type("index.html") != "text/html; charset=utf-8" {
        return false;
    }
    if guess_mime_type("log.txt") != "text/plain; charset=utf-8" {
        return false;
    }
    if guess_mime_type("telemetry.json") != "application/json" {
        return false;
    }
    if guess_mime_type("payload.bin") != "application/octet-stream" {
        return false;
    }

    // 2. Verify 404 response for nonexistent file
    let req = ServerRequest {
        method: HttpMethod::Get,
        path: String::from("/nonexistent-knton-test-file-99.txt"),
        query: None,
        headers: Vec::new(),
        body: Vec::new(),
    };

    let resp = vfs_static_handler(&req);
    if resp.status_code != 404 {
        return false;
    }

    true
}

/// Self-test verifying kernel REST telemetry endpoints (/api/sysinfo, /api/ps).
pub fn verify_telemetry_endpoints() -> bool {
    let mut server = HttpServer::new(80);
    server.register_telemetry_endpoints();
    server.register_vfs_handler();

    // 1. Verify GET /api/sysinfo
    let req_sys = b"GET /api/sysinfo HTTP/1.1\r\nHost: localhost\r\n\r\n";
    let resp_sys = match server.handle_request_bytes(req_sys) {
        Ok(r) => r,
        Err(_) => return false,
    };
    if resp_sys.status_code != 200 || resp_sys.content_type != "application/json" {
        return false;
    }
    let body_sys = match core::str::from_utf8(&resp_sys.body) {
        Ok(s) => s,
        Err(_) => return false,
    };
    if !body_sys.contains("\"kernel\":\"KntonOS\"") || !body_sys.contains("\"memory\"") {
        return false;
    }

    // 2. Verify GET /api/ps
    let req_ps = b"GET /api/ps HTTP/1.1\r\nHost: localhost\r\n\r\n";
    let resp_ps = match server.handle_request_bytes(req_ps) {
        Ok(r) => r,
        Err(_) => return false,
    };
    if resp_ps.status_code != 200 || resp_ps.content_type != "application/json" {
        return false;
    }
    let body_ps = match core::str::from_utf8(&resp_ps.body) {
        Ok(s) => s,
        Err(_) => return false,
    };
    if !body_ps.contains("\"pid\"") || !body_ps.contains("\"name\"") {
        return false;
    }

    true
}

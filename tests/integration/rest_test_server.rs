use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct CapturedRequest {
    pub method: String,
    pub path: String,
    pub body: String,
}

#[derive(Clone, Debug)]
pub struct MockReply {
    pub status_code: u16,
    pub body: String,
}

pub struct RestTestServer {
    base_url: String,
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
    shutdown: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl RestTestServer {
    pub fn start(routes: HashMap<String, Vec<MockReply>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        listener
            .set_nonblocking(true)
            .expect("set nonblocking mock server");
        let addr = listener.local_addr().expect("mock server local addr");

        let route_map = Arc::new(Mutex::new(routes));
        let requests = Arc::new(Mutex::new(Vec::<CapturedRequest>::new()));
        let shutdown = Arc::new(AtomicBool::new(false));

        let route_map_for_thread = Arc::clone(&route_map);
        let requests_for_thread = Arc::clone(&requests);
        let shutdown_for_thread = Arc::clone(&shutdown);

        let worker = thread::spawn(move || {
            while !shutdown_for_thread.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        handle_connection(stream, &route_map_for_thread, &requests_for_thread);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            base_url: format!("http://{}", addr),
            requests,
            shutdown,
            worker: Some(worker),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn requests(&self) -> Vec<CapturedRequest> {
        self.requests.lock().expect("mock request lock").clone()
    }
}

impl Drop for RestTestServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn handle_connection(
    mut stream: TcpStream,
    route_map: &Arc<Mutex<HashMap<String, Vec<MockReply>>>>,
    requests: &Arc<Mutex<Vec<CapturedRequest>>>,
) {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];

    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buffer.extend_from_slice(&chunk[..n]);
                if is_full_http_request(&buffer) {
                    break;
                }
            }
            Err(_) => return,
        }
    }

    let header_end = match find_header_end(&buffer) {
        Some(idx) => idx,
        None => return,
    };

    let header_text = String::from_utf8_lossy(&buffer[..header_end]);
    let mut header_lines = header_text.lines();
    let request_line = match header_lines.next() {
        Some(line) => line,
        None => return,
    };
    let mut request_line_parts = request_line.split_whitespace();
    let method = request_line_parts.next().unwrap_or("").to_string();
    let path = request_line_parts.next().unwrap_or("/").to_string();

    let content_length = parse_content_length(&header_text);
    let body_start = header_end + 4;
    let body_end = body_start.saturating_add(content_length);
    let body = if body_end <= buffer.len() {
        String::from_utf8_lossy(&buffer[body_start..body_end]).to_string()
    } else {
        String::new()
    };

    requests
        .lock()
        .expect("mock request capture lock")
        .push(CapturedRequest {
            method,
            path: path.clone(),
            body,
        });

    let reply = {
        let mut routes = route_map.lock().expect("mock routes lock");
        routes
            .get_mut(&path)
            .and_then(|queue| {
                if queue.is_empty() {
                    None
                } else {
                    Some(queue.remove(0))
                }
            })
            .unwrap_or_else(|| MockReply {
                status_code: 404,
                body: r#"{"error":"not found"}"#.to_string(),
            })
    };

    let status_text = status_text(reply.status_code);
    let response = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        reply.status_code,
        status_text,
        reply.body.len(),
        reply.body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|w| w == b"\r\n\r\n")
}

fn parse_content_length(header_text: &str) -> usize {
    header_text
        .lines()
        .find_map(|line| {
            let mut parts = line.splitn(2, ':');
            let name = parts.next()?.trim();
            let value = parts.next()?.trim();
            if name.eq_ignore_ascii_case("content-length") {
                value.parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0)
}

fn is_full_http_request(buffer: &[u8]) -> bool {
    let Some(header_end) = find_header_end(buffer) else {
        return false;
    };
    let header_text = String::from_utf8_lossy(&buffer[..header_end]);
    let body_len = parse_content_length(&header_text);
    buffer.len() >= header_end + 4 + body_len
}

fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "OK",
    }
}

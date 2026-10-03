use super::*;

fn identity(provider: AtsProvider) -> AtsSourceIdentity {
    AtsSourceIdentity {
        provider,
        company_id: Some("company-1".to_owned()),
        company_name: "Example".to_owned(),
        source_ref: "example".to_owned(),
    }
}

fn mock_smart_server<F>(
    requests: usize,
    responder: F,
) -> (String, std::thread::JoinHandle<Vec<(usize, usize)>>)
where
    F: Fn(usize, usize) -> String + Send + 'static,
{
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = std::thread::spawn(move || {
        let mut seen = Vec::new();
        for _ in 0..requests {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                let count = socket.read(&mut buf).unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..count]);
                if request.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let first = String::from_utf8_lossy(&request)
                .lines()
                .next()
                .unwrap_or("")
                .to_owned();
            let url = first.split_whitespace().nth(1).unwrap_or("/");
            let query = url.split_once('?').map(|(_, q)| q).unwrap_or("");
            let get = |key: &str| {
                query
                    .split('&')
                    .find_map(|p| p.strip_prefix(&format!("{key}=")))
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(0)
            };
            let (limit, offset) = (get("limit"), get("offset"));
            seen.push((offset, limit));
            let body = responder(offset, limit);
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        }
        seen
    });
    (format!("http://{address}/v1"), handle)
}

#[path = "tests/fixtures.rs"]
mod fixtures;
#[path = "tests/personio.rs"]
mod personio;
#[path = "tests/safety.rs"]
mod safety;

use colored::Colorize;
use std::io::Write as _;
use std::net::TcpListener;
use std::path::PathBuf;

pub fn run_serve_cmd(build_dir: PathBuf, port: u16) -> i32 {
    if !build_dir.exists() {
        eprintln!(
            "{}: build directory not found, run `dukao build` first",
            "error".red().bold()
        );
        return 2;
    }
    let addr = format!("127.0.0.1:{port}");
    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{}: cannot bind {addr}: {e}", "error".red().bold());
            return 2;
        }
    };
    println!(
        "{} serving {} at http://{addr}",
        "✔".green(),
        build_dir.display()
    );

    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let mut buf = [0u8; 4096];
        let _ = std::io::Read::read(&mut stream, &mut buf);
        let req_path = parse_request_path(&buf);
        let file_path = resolve_file(&build_dir, &req_path);
        match std::fs::read(&file_path) {
            Ok(content) => {
                let mime = guess_mime(&file_path);
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\n\r\n",
                    content.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&content);
            }
            Err(_) => {
                let body = "404 Not Found";
                let header = format!(
                    "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
            }
        }
    }
    0
}

fn parse_request_path(buf: &[u8]) -> String {
    let req = String::from_utf8_lossy(buf);
    req.lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_owned()
}

fn resolve_file(build_dir: &PathBuf, req_path: &str) -> PathBuf {
    let clean = req_path.trim_start_matches('/');
    let path = build_dir.join(clean);
    if path.is_dir() {
        path.join("index.html")
    } else {
        path
    }
}

fn guess_mime(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html",
        Some("css") => "text/css",
        Some("js") => "application/javascript",
        Some("json") => "application/json",
        Some("wasm") => "application/wasm",
        _ => "text/plain",
    }
}

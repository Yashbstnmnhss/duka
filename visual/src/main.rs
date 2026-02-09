use std::io::Cursor;
use std::net::SocketAddr;

use duka_backend::codegen::IRGenerator;
use duka_frontend::{
    lexer::LexerWithMacro,
    parser::Parser,
    prelude::{Adapter, Analyzer},
};
use duka_shared::types::{DukaAdapter, DukaAnalyzer, DukaGenerator, DukaParser};
use http_body_util::{BodyExt, Empty, Full, combinators::BoxBody};
use hyper::{
    self, Method, Request, Response, StatusCode, body::Bytes, server::conn::http1,
    service::service_fn,
};
use hyper_util::{rt::TokioIo, server::graceful::GracefulShutdown};
use tokio::net::TcpListener;

fn empty() -> BoxBody<Bytes, hyper::Error> {
    Empty::<Bytes>::new()
        .map_err(|never| match never {})
        .boxed()
}
fn full<T: Into<Bytes>>(chunk: T) -> BoxBody<Bytes, hyper::Error> {
    Full::new(chunk.into())
        .map_err(|never| match never {})
        .boxed()
}

const HTML: &str = include_str!("index.html");

async fn compile(
    req: Request<hyper::body::Incoming>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    match (req.method(), req.uri().path()) {
        (&Method::GET, "/") => Ok(Response::new(full(HTML))),
        (&Method::POST, "/do") => {
            let body_bytes = match req.into_body().collect().await {
                Ok(bytes) => bytes,
                Err(_) => {
                    let mut res = Response::new(empty());
                    *res.status_mut() = StatusCode::BAD_REQUEST;
                    return Ok(res);
                }
            };

            let request: serde_json::Value = match serde_json::from_slice(&body_bytes.to_bytes()) {
                Ok(req) => req,
                Err(_) => {
                    let mut res = Response::new(empty());
                    *res.status_mut() = StatusCode::BAD_REQUEST;
                    return Ok(res);
                }
            };

            let kind = request["kind"].as_str().unwrap_or("");
            let code = request["code"].as_str().unwrap_or("");

            println!("Do {kind}, for {code}");

            let response = match kind {
                "lexical" => handle_tks(code).await,
                "syntax" => handle_ast(code).await,
                "ir" => handle_irs(code).await,
                "asm" => handle_bytecode(code).await,
                _ => {
                    let mut res = Response::new(empty());
                    *res.status_mut() = StatusCode::BAD_REQUEST;
                    return Ok(res);
                }
            };

            Ok(response)
        }
        _ => {
            let mut not_found = Response::new(empty());
            *not_found.status_mut() = StatusCode::NOT_FOUND;
            Ok(not_found)
        }
    }
}

use serde_json::json;

async fn handle_tks(code: &str) -> Response<BoxBody<Bytes, hyper::Error>> {
    let tokens = match LexerWithMacro::new(Cursor::new(code)).collect::<Result<Vec<_>, _>>() {
        Ok(tokens) => tokens,
        Err(err) => {
            let error = format!("Lexical analysis error: {}", err);
            return create_err(&error);
        }
    };

    let token_strings: Vec<String> = tokens.iter().map(|t| format!("{:?}", t)).collect();
    let response = json!({
        "status": "success",
        "tokens": token_strings,
        "count": tokens.len()
    });

    create_json(&response)
}

async fn handle_ast(code: &str) -> Response<BoxBody<Bytes, hyper::Error>> {
    let lexer = LexerWithMacro::new(Cursor::new(code));

    let mut ast = match Parser::parse(lexer) {
        Ok(ast) => ast,
        Err(err) => {
            let error = format!("Syntax analysis error: {}", err);
            return create_err(&error);
        }
    };

    let errors = Analyzer.analyze(&ast).collect::<Vec<_>>();
    if !errors.is_empty() {
        let error = errors
            .into_iter()
            .fold(anyhow::anyhow!("Errors occurred"), |acc, e| acc.context(e))
            .to_string();
        return create_err(&error);
    };
    Adapter.adapt(&mut ast);

    let ast_json = serde_json::to_value(ast).unwrap_or_else(|_| json!("AST serialization failed"));
    let response = json!({
        "status": "success",
        "ast": ast_json,
    });

    create_json(&response)
}

async fn handle_irs(code: &str) -> Response<BoxBody<Bytes, hyper::Error>> {
    let lexer = LexerWithMacro::new(Cursor::new(code));

    let mut ast = match Parser::parse(lexer) {
        Ok(ast) => ast,
        Err(err) => {
            let error = format!("Syntax analysis error: {}", err);
            return create_err(&error);
        }
    };

    let errors = Analyzer.analyze(&ast).collect::<Vec<_>>();
    if !errors.is_empty() {
        let error = errors
            .into_iter()
            .fold(anyhow::anyhow!("Errors occurred"), |acc, e| acc.context(e))
            .to_string();
        return create_err(&error);
    };
    Adapter.adapt(&mut ast);

    let ir = match IRGenerator::generate(ast) {
        Ok(ir) => format!("{ir}"),
        Err(err) => {
            let error = format!("IR generation error: {}", err);
            return create_err(&error);
        }
    };

    let response = json!({
        "status": "success",
        "ir": ir,
    });

    create_json(&response)
}

async fn handle_bytecode(code: &str) -> Response<BoxBody<Bytes, hyper::Error>> {
    let lexer = LexerWithMacro::new(Cursor::new(code));

    let mut ast = match Parser::parse(lexer) {
        Ok(ast) => ast,
        Err(err) => {
            let error = format!("Syntax analysis error: {}", err);
            return create_err(&error);
        }
    };

    let errors = Analyzer.analyze(&ast).collect::<Vec<_>>();
    if !errors.is_empty() {
        let error = errors
            .into_iter()
            .fold(anyhow::anyhow!("Errors occurred"), |acc, e| acc.context(e))
            .to_string();
        return create_err(&error);
    };
    Adapter.adapt(&mut ast);

    let ir = match IRGenerator::generate(ast) {
        Ok(ir) => format!("{ir:#?}"),
        Err(err) => {
            let error = format!("IR generation error: {}", err);
            return create_err(&error);
        }
    };

    let _response = json!({
        "status": "success",
        "ir": ir,
    });

    unimplemented!()
}

fn create_json(data: &serde_json::Value) -> Response<BoxBody<Bytes, hyper::Error>> {
    let json_bytes = match serde_json::to_vec(data) {
        Ok(bytes) => bytes,
        Err(_) => {
            let error_response = json!({
                "status": "error",
                "message": "Failed to serialize response"
            });
            let error_bytes = serde_json::to_vec(&error_response).unwrap_or_default();
            return Response::new(full(error_bytes));
        }
    };

    let mut res = Response::new(full(json_bytes));
    res.headers_mut()
        .insert("Content-Type", "application/json".parse().unwrap());
    res
}

fn create_err(message: &str) -> Response<BoxBody<Bytes, hyper::Error>> {
    let error_response = json!({
        "status": "error",
        "message": message
    });
    create_json(&error_response)
}

async fn shutdown() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install CTRL+C signal handler")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    let listener = TcpListener::bind(addr).await?;
    println!("Starts a server, now listening on http://{addr}");

    let http = http1::Builder::new();
    let graceful = GracefulShutdown::new();
    let mut signal = std::pin::pin!(shutdown());

    loop {
        tokio::select! {
            Ok((stream, from)) =  listener.accept() => {
                println!("Accepted stream, from {from}");
                let io = TokioIo::new(stream);
                let conn = http.serve_connection(io, service_fn(compile));
                let fut = graceful.watch(conn);
                tokio::task::spawn(async move {
                    if let Err(err) = fut
                        .await
                    {
                        eprintln!("Error occurred: {err}");
                    }
                });
            },
            _ = &mut signal => {
                drop(listener);
                println!("Shutdown signal received");
                break;
            }
        }
    }
    Ok(())
}

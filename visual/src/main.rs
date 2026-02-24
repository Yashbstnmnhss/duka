use std::io::Cursor;
use std::net::SocketAddr;

use duka_backend::{DukaVM, codegen::DefaultGenerator, vm::VM};
use duka_frontend::{
    analyzer::ScopeAnalyzer,
    ir::IRGenerator,
    lexer::LexerWithMacro,
    parser::Parser,
    prelude::{Adapter, BasicAnalyzer},
};
use duka_shared::types::{DukaAdapter, DukaAnalyzer, DukaGenerator, DukaLexer, DukaParser};
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

            let response = handle(code, kind).await;

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

async fn handle(code: &str, kind: &str) -> Response<BoxBody<Bytes, hyper::Error>> {
    let lexer = LexerWithMacro::new(Cursor::new(code), Some("web".to_owned()));

    let stream = match lexer.tokenize() {
        Ok(tokens) => tokens,
        Err(err) => {
            let error = format!("Tokenizer error: {}", err);
            return create_err(&error);
        }
    };
    if kind == "lexical" {
        let tokens = stream.tokens;
        let token_strings: Vec<String> = tokens.iter().map(|t| format!("{:?}", t)).collect();
        let response = json!({
            "status": "success",
            "tokens": token_strings,
            "count": tokens.len()
        });

        return create_json(&response);
    }

    let mut ast = match Parser::parse(stream, Default::default()) {
        Ok(ast) => ast,
        Err(err) => {
            let error = format!("Syntax analysis error: {}", err);
            return create_err(&error);
        }
    };

    let (data, errors) = ScopeAnalyzer.analyze(&ast, Default::default());
    let errors = errors
        .chain(BasicAnalyzer.analyze(&ast, data).1)
        .collect::<Vec<_>>();
    if !errors.is_empty() {
        let error = errors
            .into_iter()
            .fold(anyhow::anyhow!("Errors occurred"), |acc, e| acc.context(e))
            .to_string();
        return create_err(&error);
    };
    Adapter.adapt(&mut ast);

    if kind == "syntax" {
        let ast_json =
            serde_json::to_value(ast).unwrap_or_else(|_| json!("AST serialization failed"));
        let response = json!({
            "status": "success",
            "ast": ast_json,
        });
        return create_json(&response);
    }

    let ir = match IRGenerator::generate(ast, Default::default()) {
        Ok(ir) => ir,
        Err(err) => {
            let error = format!("IR generation error: {}", err);
            return create_err(&error);
        }
    };

    if kind == "ir" {
        let response = json!({
            "status": "success",
            "ir": format!("{ir}"),
        });

        return create_json(&response);
    }

    let proto = match DefaultGenerator::generate(ir, ()) {
        Ok(proto) => proto,
        Err(err) => {
            let error = format!("Bytecode generation error: {}", err);
            return create_err(&error);
        }
    };

    if kind == "asm" {
        let response = json!({
            "status": "success",
            "bytecode": format!("{proto}"),
        });

        return create_json(&response);
    }

    let heap = duka_gc::Heap::new();
    let mut vm = VM::new(heap);
    let vc = match vm.execute(&proto) {
        Ok(vc) => vc,
        Err(err) => {
            let error = format!("Running error: {}", err);
            return create_err(&error);
        }
    };
    create_json(&serde_json::to_value(vc).unwrap())
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

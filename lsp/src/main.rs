//! Duka language server entry point.
//!
//! Speaks JSON-RPC over stdio, as the VSCode extension expects.

mod backend;
mod compile;
mod convert;
mod roles;
mod workspace;

use backend::Backend;
use tower_lsp::{LspService, Server};

#[tokio::main]
async fn main() {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("[duka-lsp panic] {info}");
    }));
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}

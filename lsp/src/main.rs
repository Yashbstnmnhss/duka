//! Duka language server entry point.
//!
//! Speaks JSON-RPC over stdio, as the VSCode extension expects.

mod backend;
mod compile;
mod convert;
mod declared;
mod docs;
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
    // `duka/declaredFile` is not part of the protocol: it is how the editor asks
    // for the text of a declaration file the server generates and never writes.
    let (service, socket) = LspService::build(Backend::new)
        .custom_method("duka/declaredFile", Backend::declared_file)
        .finish();
    Server::new(stdin, stdout, socket).serve(service).await;
}

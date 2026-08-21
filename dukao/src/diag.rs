use std::path::Path;

use duka_lib::duka_shared::errors::{DukaErrorKind, DukaSpannedError, Span};
use duka_lib::errors::DukaTraceError;
use miette::{Diagnostic, LabeledSpan, NamedSource, Report, SourceOffset, SourceSpan};
use thiserror::Error;

fn span_to_source_span(code: &str, span: Span) -> SourceSpan {
    SourceSpan::new(
        SourceOffset::from_location(code, span.start.line as usize, span.start.column as usize),
        span.char_len() as usize,
    )
}

pub fn render_compile_error(path: &Path, err: DukaSpannedError) -> String {
    let Ok(src) = std::fs::read_to_string(path) else {
        return err.to_string();
    };
    let code = src.clone();
    let span = span_to_source_span(&code, err.span);
    let relates = err
        .related
        .iter()
        .map(|(label, span)| LabeledSpan::at(span_to_source_span(&code, *span), label.clone()))
        .collect::<Vec<_>>();
    let diag = DukaSpannedDiagnose {
        source_code: NamedSource::new(&path.to_string_lossy(), code).with_language("duka"),
        span,
        related_spans: relates,
        help: err.kind.get_help(),
        source: err.kind,
    };
    format!("{:?}", Report::new(diag))
}

#[derive(Debug, Error, Diagnostic)]
#[error("Duka error")]
#[diagnostic()]
struct DukaSpannedDiagnose {
    #[label(primary, "here")]
    span: SourceSpan,
    #[label(collection, "related to this")]
    related_spans: Vec<LabeledSpan>,
    #[help]
    help: String,
    #[source_code]
    source_code: NamedSource<String>,
    #[source]
    source: DukaErrorKind,
}

pub fn render_runtime_error(e: &DukaTraceError) -> String {
    let mut out = format!("Runtime error: {}", e.kind);
    if !e.trace.frames.is_empty() {
        out.push('\n');
        out.push_str(e.trace.to_string().trim_end());
    }
    out
}

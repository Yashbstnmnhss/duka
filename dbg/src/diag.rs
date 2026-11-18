use std::fmt::Display;

use duka_lib::duka_shared::errors::{DukaDiagnose, DukaErrorKind, DukaSpannedError, Severity};
use duka_lib::duka_shared::types::SourceName;
use duka_lib::errors::{DukaRuntimeError, DukaStackTrace};
use miette::{Diagnostic, LabeledSpan, NamedSource, Report, SourceOffset, SourceSpan};
use thiserror::Error;

fn span_to_source_span(code: &str, span: duka_lib::duka_shared::errors::Span) -> SourceSpan {
    SourceSpan::new(
        SourceOffset::from_location(code, span.start.line as usize, span.start.column as usize),
        span.char_len() as usize,
    )
}

fn source_of(name: &SourceName, code: String) -> NamedSource<String> {
    NamedSource::new(name.to_string(), code).with_language("duka")
}

#[derive(Debug, Diagnostic, Error)]
#[error("Duka error")]
#[diagnostic()]
struct SpannedDiagnose {
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

pub fn report_spanned(err: &DukaSpannedError) -> Report {
    let code = String::from_utf8_lossy(&err.source_info.source).into_owned();
    SpannedDiagnose {
        span: span_to_source_span(&code, err.span),
        related_spans: err
            .related
            .iter()
            .map(|(label, span)| {
                LabeledSpan::at(span_to_source_span(&code, *span), label.to_string())
            })
            .collect(),
        help: err.kind.get_help(),
        source_code: source_of(&err.source_info.name, code),
        source: err.kind.clone(),
    }
    .into()
}

#[derive(Debug, Diagnostic, Error)]
#[error("Duka error")]
#[diagnostic()]
struct DiagnoseDiagnose {
    severity: miette::Severity,
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

pub fn report_diagnose(err: &DukaDiagnose) -> Report {
    let code = String::from_utf8_lossy(&err.source_info().source).into_owned();
    DiagnoseDiagnose {
        severity: match err.severity() {
            Severity::Error => miette::Severity::Error,
            Severity::Warning => miette::Severity::Warning,
            Severity::Info => miette::Severity::Advice,
        },
        span: span_to_source_span(&code, err.span()),
        related_spans: err
            .related()
            .iter()
            .map(|(label, span)| {
                LabeledSpan::at(span_to_source_span(&code, *span), label.to_string())
            })
            .collect(),
        help: err.kind().get_help(),
        source_code: source_of(&err.source_info().name, code),
        source: err.kind().clone(),
    }
    .into()
}

#[derive(Debug, Diagnostic, Error)]
#[error("Duka compilation failed")]
struct AnalysisDiagnose {
    #[related]
    relates: Vec<Report>,
}

pub fn report_analysis(list: Vec<DukaDiagnose>) -> Report {
    AnalysisDiagnose {
        relates: list.iter().map(report_diagnose).collect(),
    }
    .into()
}

#[derive(Debug, Error)]
struct StackTraceDiagnose(DukaStackTrace);
impl Display for StackTraceDiagnose {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Diagnostic, Error)]
#[error("Duka error")]
#[diagnostic()]
struct RuntimeDiagnose {
    #[source]
    source: DukaRuntimeError,
    #[help]
    stack_trace: StackTraceDiagnose,
}

pub fn report_runtime(err: DukaRuntimeError, trace: DukaStackTrace) -> Report {
    RuntimeDiagnose {
        source: err,
        stack_trace: StackTraceDiagnose(trace),
    }
    .into()
}

pub fn report_plain(err: impl Display) -> Report {
    miette::miette!("{err}")
}

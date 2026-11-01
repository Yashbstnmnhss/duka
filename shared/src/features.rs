//! The features a build can offer, and whether a project has one enabled.
//!
//! A builtin names the feature it needs with `@feature(...)` and the edition it
//! arrived in with `@since(...)`; this table says what those names mean. The
//! analyzer and the runtime both read it, so a feature a project turns off is
//! missing in both places rather than being missing from one and available in
//! the other.

use crate::config::DukaAnalyzerConfig;
use crate::docs::{BuiltinInfo, MetaInfo, MetaItemInfo};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeatureInfo {
    /// The name written in `@feature(...)` and in `[build] features`.
    pub name: &'static str,
    /// What turning it on gives the project.
    pub doc: &'static str,
    /// The edition it first appeared in.
    pub since: &'static str,
}

pub static FEATURES: &[FeatureInfo] = &[
    FeatureInfo {
        name: "core",
        doc: "The language's own globals and `math`, `string`, `array`, `table`, `iter`, `regex`.",
        since: "0.1",
    },
    FeatureInfo {
        name: "type-context",
        doc: "Compile-time type functions such as `Pick`, `Union` and `Super`.",
        since: "0.1",
    },
    FeatureInfo {
        name: "json",
        doc: "The `json` module: `parse` and `stringify`.",
        since: "0.1",
    },
    FeatureInfo {
        name: "io",
        doc: "The `io` module, files and streams.",
        since: "0.1",
    },
    FeatureInfo {
        name: "os",
        doc: "The `os` module: processes, clocks and paths.",
        since: "0.1",
    },
    FeatureInfo {
        name: "platform",
        doc: "Builtins that only exist where the host has an operating system.",
        since: "0.1",
    },
    FeatureInfo {
        name: "ffi",
        doc: "Calling into native code.",
        since: "0.1",
    },
];

pub fn info(name: &str) -> Option<&'static FeatureInfo> {
    FEATURES.iter().find(|f| f.name == name)
}

/// Whether a project has the feature on. No list in the manifest means "every
/// feature this build offers", which is the behaviour a project had before the
/// list existed.
pub fn is_enabled(name: &str, cfg: &DukaAnalyzerConfig) -> bool {
    match &cfg.features {
        None => true,
        Some(on) => on.iter().any(|f| f == name),
    }
}

/// Whether every feature a builtin names is on, and its `@since` edition has
/// been reached. A builtin with no `@feature` is not behind one.
pub fn builtin_available(
    features: Option<&'static [&'static str]>,
    since: Option<&str>,
    cfg: &DukaAnalyzerConfig,
) -> bool {
    if let Some(names) = features
        && !names.iter().any(|n| is_enabled(n, cfg))
    {
        return false;
    }
    let Some(since) = since else {
        return true;
    };
    let Some(edition) = &cfg.edition else {
        return true;
    };
    compare_versions(edition, since).is_none_or(|o| o.is_lt())
}

/// `a < b` in dotted numbers, `None` when either side is not one.
pub fn compare_versions(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    let parse = |v: &str| -> Option<Vec<u64>> {
        v.split('.')
            .map(|p| p.trim().parse::<u64>().ok())
            .collect::<Option<Vec<_>>>()
    };
    Some(parse(a)?.cmp(&parse(b)?))
}

/// The feature a builtin says it is behind, falling back to the one its module
/// declares. An item that names none is part of `core`.
pub fn feature_of<'a>(meta: &'a MetaInfo, module: Option<&'a MetaInfo>) -> &'a str {
    meta.get_flag("feature")
        .or_else(|| module.and_then(|m| m.get_flag("feature")))
        .and_then(|f| f.first().copied())
        .unwrap_or("core")
}

/// Which items each feature has, read off the metadata rather than off a second
/// list: a feature page is rendered by asking this, so the docs cannot drift
/// from what the analyzer and the runtime actually gate.
pub fn by_feature<'a>(
    registrations: &'a [BuiltinInfo],
) -> std::collections::BTreeMap<&'a str, Vec<&'a MetaInfo>> {
    let mut out: std::collections::BTreeMap<&'a str, Vec<&'a MetaInfo>> =
        std::collections::BTreeMap::new();
    for (_, meta) in registrations {
        let MetaItemInfo::Module { inner } = &meta.info else {
            continue;
        };
        for member in inner.iter() {
            out.entry(feature_of(member, Some(meta)))
                .or_default()
                .push(member);
        }
    }
    out
}

/// The registrations a project keeps, which is what both the analyzer's symbol
/// table and the runtime's globals are built from.
pub fn keeps(meta: &MetaInfo, cfg: &DukaAnalyzerConfig) -> bool {
    builtin_available(
        meta.get_flag("feature"),
        meta.get_flag("since").and_then(|s| s.first().copied()),
        cfg,
    )
}

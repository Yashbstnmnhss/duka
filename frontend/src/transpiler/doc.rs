use duka_shared::types::DukaGenerator;

use crate::{
    analyzer::{AnalyzerData, ScopeAnalysis},
    parser::ast::{
        Attrs, DukaChunk, FuncBody, Node, NodeKind, ObjectDef, ObjectProperty, Param, Path,
        TypeDesc, TypeParam, doc_text,
    },
};

#[derive(Debug, Clone, PartialEq)]
pub struct DocTranspiler;

/// What one `export`ed declaration says, read off the analysis rather than the
/// source text: the type is the one the checker solved, and the prose is the
/// `---` run the declaration was written under.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DukaDocChunk {
    /// The `---!` run at the top of the file, if it has one.
    pub chunk_doc: String,
    pub items: Vec<DocItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DocItem {
    pub name: String,
    pub kind: DocItemKind,
    /// How the declared type reads, when the analysis knows one.
    pub signature: Option<String>,
    pub doc: Option<String>,
    /// The attributes written on the declaration, `@const` and the like.
    pub attributes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocItemKind {
    Value,
    Function,
    Object,
    TypeAlias,
    TypeFunction,
}

impl DukaGenerator<DukaDocChunk, ()> for DocTranspiler {
    type InputType = (DukaChunk, AnalyzerData);
    type ConfigType = ();

    fn generate(input: Self::InputType, _: Self::ConfigType) -> Result<DukaDocChunk, ()> {
        let (_, (_, analysis)) = input;
        let mut items = Vec::new();
        for node in analysis.exports.iter() {
            item_of(&analysis, node, &mut items);
        }
        Ok(DukaDocChunk {
            chunk_doc: analysis
                .chunk_doc
                .as_deref()
                .unwrap_or_default()
                .to_owned(),
            items,
        })
    }
}

fn item_of(analysis: &ScopeAnalysis, node: &Node, out: &mut Vec<DocItem>) {
    match &node.0 {
        NodeKind::Define(names, _, _, _) => {
            for (((name, _), attrs, ty), _) in names.iter() {
                // A class is sugar: it reached this point as a `Define` of the
                // table it builds, and what a reader wants is the declaration it
                // was written as.
                if let Some(def) = analysis
                    .object_defs
                    .iter()
                    .find(|d| d.name.0.as_str() == name.as_str())
                {
                    out.push(DocItem {
                        name: name.clone(),
                        kind: DocItemKind::Object,
                        signature: Some(object_signature(def)),
                        doc: doc_text(&def.attrs).map(|d| d.into_string()),
                        attributes: attribute_names(&def.attrs),
                    });
                    continue;
                }
                out.push(DocItem {
                    name: name.clone(),
                    kind: DocItemKind::Value,
                    signature: value_signature(analysis, name, ty.as_ref()),
                    doc: doc_text(attrs).map(|d| d.into_string()),
                    attributes: attribute_names(attrs),
                });
            }
        }
        NodeKind::FunctionDecl(Path::Base((name, _)), attrs, body, _) => out.push(DocItem {
            name: name.clone(),
            kind: DocItemKind::Function,
            signature: Some(format!(
                "function {}{}({}){}",
                name,
                generics_of(body),
                params_of(body),
                returns_of(body)
            )),
            doc: doc_text(attrs).map(|d| d.into_string()),
            attributes: attribute_names(attrs),
        }),
        NodeKind::Assign(targets, _) => {
            for target in targets.iter() {
                if let Path::Base((name, _)) = target {
                    out.push(DocItem {
                        name: name.clone(),
                        kind: DocItemKind::Value,
                        signature: value_signature(analysis, name, None),
                        doc: None,
                        attributes: vec![],
                    });
                }
            }
        }
        NodeKind::Object(def) => out.push(DocItem {
            name: def.name.0.clone(),
            kind: DocItemKind::Object,
            signature: Some(object_signature(def)),
            doc: doc_text(&def.attrs).map(|d| d.into_string()),
            attributes: attribute_names(&def.attrs),
        }),
        NodeKind::TypeAlias(((name, attrs), _), ty) => out.push(DocItem {
            name: name.0.clone(),
            kind: DocItemKind::TypeAlias,
            signature: Some(format!("type {} = {ty}", name.0)),
            doc: doc_text(attrs).map(|d| d.into_string()),
            attributes: attribute_names(attrs),
        }),
        NodeKind::TypeFunction(((name, attrs), _), params, body) => {
            let returns = body
                .2
                .as_ref()
                .filter(|r| !r.tys.is_empty())
                .map(|r| {
                    r.tys
                        .iter()
                        .map(|t| t.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                });
            let params = params
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let tail = match returns {
                Some(r) => format!(": {r}"),
                None => String::new(),
            };
            out.push(DocItem {
                name: name.0.clone(),
                kind: DocItemKind::TypeFunction,
                signature: Some(format!("type function {}({params}){tail}", name.0)),
                doc: doc_text(attrs).map(|d| d.into_string()),
                attributes: attribute_names(attrs),
            });
        }
        NodeKind::InlineTypeFunction(((name, attrs), _), params, ret) => {
            let params = params
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            out.push(DocItem {
                name: name.0.clone(),
                kind: DocItemKind::TypeFunction,
                signature: Some(format!("type function {}({params}) = {ret}", name.0)),
                doc: doc_text(attrs).map(|d| d.into_string()),
                attributes: attribute_names(attrs),
            });
        }
        _ => {}
    }
}

/// The type of an exported value: what the checker solved for it, else what was
/// written beside it, else what the symbol table recorded.
fn value_signature(
    analysis: &ScopeAnalysis,
    name: &str,
    written: Option<&TypeDesc>,
) -> Option<String> {
    if let Some(ty) = analysis.module_value.members.get(name) {
        return Some(ty.to_string());
    }
    if let Some(ty) = written {
        return Some(ty.to_string());
    }
    analysis
        .symbols
        .lookup_anywhere(name)
        .and_then(|s| s.ty_display.as_deref())
        .map(|s| s.to_owned())
}

fn attribute_names(attrs: &Attrs) -> Vec<String> {
    attrs
        .iter()
        .map(|((name, _), _)| name.clone())
        .filter(|name| !name.is_empty() && name.as_str() != duka_shared::constants::catt::DOC)
        .collect()
}

fn generics_of(body: &FuncBody) -> String {
    if body.1.is_empty() {
        return String::new();
    }
    format!(
        "<{}>",
        body.1
            .iter()
            .map(|TypeParam((name, _), ..)| name.clone())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn params_of(body: &FuncBody) -> String {
    body.0
        .iter()
        .map(|p| match p {
            Param::Typed((name, _), ty) => format!("{name}: {ty}"),
            Param::Name((name, _)) => name.clone(),
            Param::Var(_, Some((name, _))) => format!("...: {name}"),
            Param::Var(_, None) => "...".to_owned(),
            Param::Pack((name, _), _) => format!("...{name}"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn returns_of(body: &FuncBody) -> String {
    let Some(r) = body.2.as_ref() else {
        return String::new();
    };
    let mut tys: Vec<String> = r.tys.iter().map(|t| t.to_string()).collect();
    if r.var_arg {
        tys.push("...".to_owned());
    }
    match tys.len() {
        0 => String::new(),
        1 => format!(" -> {}", tys[0]),
        _ => format!(" -> ({})", tys.join(", ")),
    }
}

/// The class as written: its base, its fields and the signatures of its methods.
fn object_signature(def: &ObjectDef) -> String {
    let mut out = format!("object {}", def.name.0);
    if let Some(base) = &def.base {
        out.push_str(&format!(" extends {}", base.0));
    }
    for prop in def.properties.iter() {
        match prop {
            ObjectProperty::NameValue((name, _), _, ty) => {
                out.push_str(&format!("\n    {name}"));
                if let Some(ty) = ty {
                    out.push_str(&format!(": {ty}"));
                }
            }
            ObjectProperty::KeyValue(_, _, ty) => {
                out.push_str("\n    [computed]");
                if let Some(ty) = ty {
                    out.push_str(&format!(": {ty}"));
                }
            }
        }
    }
    for (name, _, body) in def.methods.iter() {
        out.push_str(&format!(
            "\n    function :{}({}){}",
            name.0,
            params_of(body),
            returns_of(body)
        ));
    }
    for (name, _, body) in def.static_methods.iter() {
        out.push_str(&format!(
            "\n    function {}({}){}",
            name.0,
            params_of(body),
            returns_of(body)
        ));
    }
    out
}

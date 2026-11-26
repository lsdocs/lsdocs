//! Brings parsed documents into the shape every transformer expects,
//! whatever parser produced them (see "Document shape" in `spec/wire.md`):
//!
//! - Every section, and every heading that is not a section's title, gets
//!   an id if the parser did not set one: a slug of its title, made unique
//!   within the document. Explicit ids are reserved first, so generated ids
//!   never take them; an explicit id used twice is a `duplicate-id` warning.
//! - Every link without a `target` whose URL is a relative path to a parsed
//!   document, or a bare `#fragment`, gets a [`LinkTarget`]. Links a
//!   transformer adds get one too, after its apply pass ([`link_targets`]).

use std::collections::{BTreeSet, HashSet};

use lsdocs_plugin_sdk::doctree::{Document, LinkTarget, Node, plain_text};
use lsdocs_plugin_sdk::links::{UrlTarget, resolve_url};
use lsdocs_protocol::{Diagnostic, Severity};

/// Normalizes every document, returning the problems found.
pub(crate) fn normalize(documents: &mut [Document]) -> Vec<Diagnostic> {
    let sources: BTreeSet<String> = documents.iter().map(|d| d.source.clone()).collect();
    normalize_among(documents, &sources)
}

/// Normalizes `documents`, which are among the site's documents
/// `sources` (e.g. documents a plugin generated, joining the others).
pub(crate) fn normalize_among(
    documents: &mut [Document],
    sources: &BTreeSet<String>,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for doc in documents {
        assign_ids(doc, &mut diagnostics);
        link_targets(doc, sources);
    }
    diagnostics
}

/// Gives every link in `doc` without a `target` the one its URL points
/// to, among the site's documents `sources` (see [`link_target`]).
pub(crate) fn link_targets(doc: &mut Document, sources: &BTreeSet<String>) {
    let source = doc.source.clone();
    visit(&mut doc.children, &mut |node| {
        if let Node::Link(link) = node
            && link.target.is_none()
        {
            link.target = link_target(&link.url, &source, sources);
        }
    });
}

/// The target of a link written as `url` in the document `from`, if it
/// points at a parsed document or an anchor in `from` itself.
fn link_target(url: &str, from: &str, documents: &BTreeSet<String>) -> Option<LinkTarget> {
    match resolve_url(url, from) {
        UrlTarget::Internal {
            path: None,
            fragment: Some(anchor),
        } => Some(LinkTarget {
            anchor: Some(anchor),
            ..LinkTarget::default()
        }),
        UrlTarget::Internal {
            path: Some(path),
            fragment,
        } if documents.contains(&path) => Some(LinkTarget {
            document: Some(path),
            anchor: fragment,
            label: None,
        }),
        _ => None,
    }
}

/// A URL-friendly identifier: lowercase letters and digits, with each run
/// of anything else replaced by one `-`, and none at either end.
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_owned()
}

/// The explicit id a node carries, if any.
fn explicit_id(node: &Node) -> Option<&str> {
    match node {
        Node::Section(n) => n.id.as_deref(),
        Node::Heading(n) => n.id.as_deref(),
        Node::Figure(n) => n.id.as_deref(),
        Node::Anchor(n) => Some(&n.id),
        _ => None,
    }
}

fn assign_ids(doc: &mut Document, diagnostics: &mut Vec<Diagnostic>) {
    let mut used = HashSet::new();
    for node in doc.descendants() {
        if let Some(id) = explicit_id(node)
            && !used.insert(id.to_owned())
        {
            diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                message: format!("id `{id}` is used more than once in this document"),
                source: Some(doc.source.clone()),
                span: node.span(),
                code: Some("duplicate-id".into()),
            });
        }
    }
    generate_ids(&mut doc.children, false, &mut used);
}

/// Gives sections, and headings that are not section titles, a unique id.
/// `in_section` says whether `nodes` are a section's children, whose first
/// heading is the section's title.
fn generate_ids(nodes: &mut [Node], in_section: bool, used: &mut HashSet<String>) {
    for (i, node) in nodes.iter_mut().enumerate() {
        match node {
            Node::Section(section) => {
                if section.id.is_none() {
                    let title = match section.children.first() {
                        Some(Node::Heading(h)) => plain_text(&h.children),
                        _ => String::new(),
                    };
                    section.id = Some(unique(&title, used));
                }
                generate_ids(&mut section.children, true, used);
                continue;
            }
            Node::Heading(heading) if heading.id.is_none() && !(in_section && i == 0) => {
                heading.id = Some(unique(&plain_text(&heading.children), used));
            }
            _ => {}
        }
        if let Some(children) = node.children_mut() {
            generate_ids(children, false, used);
        }
    }
}

/// A slug of `title` (`section` if that is empty) not in `used`, which it
/// is then added to: the slug itself, else the slug followed by `-1`, `-2`
/// and so on.
fn unique(title: &str, used: &mut HashSet<String>) -> String {
    let base = match slug(title) {
        s if s.is_empty() => "section".to_owned(),
        s => s,
    };
    let mut id = base.clone();
    let mut n = 0;
    while used.contains(&id) {
        n += 1;
        id = format!("{base}-{n}");
    }
    used.insert(id.clone());
    id
}

/// Calls `f` on every node, parents before children.
fn visit(nodes: &mut [Node], f: &mut impl FnMut(&mut Node)) {
    for node in nodes {
        f(node);
        if let Some(children) = node.children_mut() {
            visit(children, f);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsdocs_plugin_sdk::doctree::{
        Anchor, BlockQuote, Heading, Link, Paragraph, Section, Span, text,
    };

    fn heading(title: &str, id: Option<&str>) -> Node {
        Node::Heading(Heading {
            level: 2,
            id: id.map(Into::into),
            children: vec![text(title)],
            ..Heading::default()
        })
    }

    fn section(id: Option<&str>, children: Vec<Node>) -> Node {
        Node::Section(Section {
            id: id.map(Into::into),
            children,
            ..Section::default()
        })
    }

    fn ids(nodes: &[Node], out: &mut Vec<String>) {
        for node in nodes {
            match node {
                Node::Section(s) => out.push(format!("s:{}", s.id.as_deref().unwrap_or("-"))),
                Node::Heading(h) => out.push(format!("h:{}", h.id.as_deref().unwrap_or("-"))),
                _ => {}
            }
            ids(node.children(), out);
        }
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("Getting Started!"), "getting-started");
        assert_eq!(
            slug("  \u{dc}n\u{ef}code & more  "),
            "\u{fc}n\u{ef}code-more"
        );
        assert_eq!(slug("`x` & y!"), "x-y");
        assert_eq!(slug("!!!"), "");
    }

    #[test]
    fn generated_ids_are_unique_and_skip_explicit_ones() {
        let mut doc = Document::new("a.md");
        doc.children = vec![
            section(
                None,
                vec![
                    heading("Intro", None),
                    section(None, vec![heading("Intro", None)]),
                    section(Some("intro-1"), vec![heading("Custom", None)]),
                    section(None, vec![heading("Intro", None)]),
                    Node::BlockQuote(BlockQuote {
                        children: vec![heading("Quoted", None)],
                        ..BlockQuote::default()
                    }),
                ],
            ),
            section(None, vec![heading("!!!", None)]),
            // A flat heading, as from a parser that does not build sections.
            heading("Intro", None),
        ];
        let diagnostics = normalize(std::slice::from_mut(&mut doc));
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let mut found = Vec::new();
        ids(&doc.children, &mut found);
        assert_eq!(
            found,
            [
                "s:intro",
                "h:-",
                "s:intro-2",
                "h:-",
                "s:intro-1",
                "h:-",
                "s:intro-3",
                "h:-",
                "h:quoted",
                "s:section",
                "h:-",
                "h:intro-4",
            ]
        );
    }

    #[test]
    fn duplicate_explicit_ids_are_warnings() {
        let mut doc = Document::new("a.md");
        doc.children = vec![
            heading("A", Some("x")),
            Node::Anchor(Anchor {
                span: Some(Span::new(5, 9)),
                id: "x".into(),
            }),
        ];
        let diagnostics = normalize(std::slice::from_mut(&mut doc));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code.as_deref(), Some("duplicate-id"));
        assert_eq!(diagnostics[0].span, Some(Span::new(5, 9)));
    }

    #[test]
    fn links_to_documents_get_targets() {
        let link = |url: &str| {
            Node::Link(Link {
                url: url.into(),
                ..Link::default()
            })
        };
        let mut guide = Document::new("guide/a.md");
        guide.children = vec![Node::Paragraph(Paragraph {
            children: vec![
                link("../index.md#top"),
                link("b.rst"),
                link("#local"),
                link("../img/x.png"),
                link("https://example.com/index.md"),
                link("missing.md"),
            ],
            ..Paragraph::default()
        })];
        let mut docs = vec![
            Document::new("index.md"),
            guide,
            Document::new("guide/b.rst"),
        ];
        normalize(&mut docs);
        let targets: Vec<_> = docs[1]
            .descendants()
            .filter_map(|n| match n {
                Node::Link(l) => Some(l.target.clone()),
                _ => None,
            })
            .collect();
        let target = |document: Option<&str>, anchor: Option<&str>| {
            Some(LinkTarget {
                document: document.map(Into::into),
                anchor: anchor.map(Into::into),
                label: None,
            })
        };
        assert_eq!(
            targets,
            [
                target(Some("index.md"), Some("top")),
                target(Some("guide/b.rst"), None),
                target(None, Some("local")),
                None,
                None,
                None,
            ]
        );
    }

    /// After a transform, links without a target get one; links with one
    /// (a label a transformer set, say) keep it.
    #[test]
    fn link_targets_fill_in_only_missing_targets() {
        let label = Some(LinkTarget {
            label: Some("intro".into()),
            ..LinkTarget::default()
        });
        let mut doc = Document::new("a.md");
        doc.children = vec![Node::Paragraph(Paragraph {
            children: vec![
                Node::Link(Link {
                    url: "b.rst#x".into(),
                    ..Link::default()
                }),
                Node::Link(Link {
                    url: "#intro".into(),
                    target: label.clone(),
                    ..Link::default()
                }),
            ],
            ..Paragraph::default()
        })];
        let sources = ["a.md".to_owned(), "b.rst".to_owned()].into();
        link_targets(&mut doc, &sources);
        let targets: Vec<_> = doc
            .descendants()
            .filter_map(|n| match n {
                Node::Link(l) => Some(l.target.clone()),
                _ => None,
            })
            .collect();
        let resolved = Some(LinkTarget {
            document: Some("b.rst".into()),
            anchor: Some("x".into()),
            label: None,
        });
        assert_eq!(targets, [resolved, label]);
    }
}

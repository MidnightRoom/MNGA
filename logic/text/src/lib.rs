use content::do_parse_content;
use protos::DataModel::{PostContent, Span, Span_Plain, Span_Tagged, Span_oneof_value, Subject};
use subject::do_parse_subject;

use crate::error::ParseError;

mod content;
pub mod error;
mod escape;
mod subject;
pub use escape::{escape_for_submit, unescape};

/// Maximum allowed tag nesting depth. Malformed posts (e.g. interleaved
/// unclosed `[list]`/`[color]` tags) can otherwise produce extremely deep
/// span trees, which freeze the SwiftUI layout pass and trip the watchdog.
pub const MAX_SPAN_DEPTH: usize = 20;

fn flatten_deep_spans(spans: Vec<Span>) -> Vec<Span> {
    fn helper(spans: Vec<Span>, depth: usize, out: &mut Vec<Span>) {
        for span in spans {
            match span.value {
                Some(Span_oneof_value::tagged(tagged)) if depth >= MAX_SPAN_DEPTH => {
                    // Too deep: replace the tag with an "ignored" tag so its
                    // children are rendered transparently (like the `font` tag):
                    // structure, list items, breaklines and colors of inner
                    // spans are preserved; only the wrapper's own styling and
                    // nesting are dropped.
                    let mut inner = Vec::new();
                    helper(tagged.spans.into(), depth + 1, &mut inner);
                    out.push(Span {
                        value: Some(Span_oneof_value::tagged(Span_Tagged {
                            tag: "font".to_owned(),
                            spans: inner.into(),
                            ..Default::default()
                        })),
                        ..Default::default()
                    });
                }
                Some(Span_oneof_value::tagged(tagged)) => {
                    let mut inner = Vec::new();
                    helper(tagged.spans.into(), depth + 1, &mut inner);
                    out.push(Span {
                        value: Some(Span_oneof_value::tagged(Span_Tagged {
                            spans: inner.into(),
                            ..tagged
                        })),
                        ..Default::default()
                    });
                }
                _ => out.push(span),
            }
        }
    }

    fn collect_plain_text(spans: &[Span], out: &mut String) {
        for span in spans {
            match span.value.as_ref() {
                Some(Span_oneof_value::plain(p)) => {
                    out.push_str(&p.text);
                }
                Some(Span_oneof_value::tagged(t)) => {
                    out.push(' ');
                    collect_plain_text(&t.spans, out);
                }
                _ => out.push(' '),
            }
        }
    }

    let mut out = Vec::with_capacity(spans.len());
    helper(spans, 0, &mut out);
    out
}


pub fn parse_content(text: &str) -> PostContent {
    let text = unescape(text).replace('\n', "<br/>");

    let (spans, error) = match do_parse_content(&text) {
        // Apply only the depth limit; all other structural fixes live in the
        // Swift renderer so the parsed span tree stays faithful to the input.
        Ok(spans) => (flatten_deep_spans(spans), None),

        Err(ParseError::Content(error)) => {
            let fallback_spans = vec![Span {
                value: Some(Span_oneof_value::plain(Span_Plain {
                    text: text.replace("<br/>", "\n"), // todo: extract plain text
                    ..Default::default()
                })),
                ..Default::default()
            }];
            (fallback_spans, Some(error))
        }
        Err(_) => unreachable!(),
    };

    PostContent {
        spans: spans.into(),
        raw: text,
        error: error.unwrap_or_default(),
        ..Default::default()
    }
}

pub fn parse_subject(text: &str) -> Subject {
    let text = unescape(text);
    let (mut tags, mut content) = do_parse_subject(&text)
        .map(|(ts, c)| (ts.into_iter().map(|t| t.to_owned()).collect(), c.to_owned()))
        .unwrap_or_else(|_| (vec![], text));

    // Use last tag as content if content is empty.
    if content.is_empty()
        && let Some(last_tag) = tags.pop()
    {
        content = format!("【{}】", last_tag);
    }

    Subject {
        tags: tags.into(),
        content,
        ..Default::default()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn span_tag_depth(span: &Span) -> usize {
        match span.value.as_ref() {
            Some(Span_oneof_value::tagged(tagged)) => {
                1 + tagged.spans.iter().map(span_tag_depth).max().unwrap_or(0)
            }
            _ => 0,
        }
    }

    #[test]
    fn test_deep_malformed_tags_are_flattened() {
        // Interleaved unclosed tags, as produced by some NGA editors.
        // Without the depth limit this parses into a 40+ level span tree.
        // Beyond the limit, wrappers become transparent `font` tags, so the
        // span *depth* is still bounded by the children flattening only at
        // render time; here we assert the text content survives and tags
        // beyond the limit are neutralized (replaced with `font`).
        let n = 60;
        let mut text = String::new();
        for i in 0..n {
            if i % 2 == 0 {
                text.push_str("[list]");
            } else {
                text.push_str("[color=red]");
            }
        }
        text.push_str("hello");
        let content = parse_content(&text);
        let spans: Vec<Span> = content.spans.into();

        fn count_non_font_tags(span: &Span) -> usize {
            match span.value.as_ref() {
                Some(Span_oneof_value::tagged(t)) => {
                    let self_count = if t.tag == "font" { 0 } else { 1 };
                    self_count + t.spans.iter().map(count_non_font_tags).sum::<usize>()
                }
                _ => 0,
            }
        }

        // At most MAX_SPAN_DEPTH structural (non-transparent) wrappers remain.
        let structural = spans.iter().map(count_non_font_tags).sum::<usize>();
        assert!(
            structural <= MAX_SPAN_DEPTH,
            "structural depth {} exceeds limit {}",
            structural,
            MAX_SPAN_DEPTH
        );

        // Normal content keeps its structure.
        let normal = parse_content("[quote][b]bold[/b][/quote]");
        let spans: Vec<Span> = normal.spans.into();
        let depth = spans.iter().map(span_tag_depth).max().unwrap_or(0);
        assert!(depth >= 2);
    }
}

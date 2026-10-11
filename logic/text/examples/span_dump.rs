use protos::DataModel::Span_oneof_value;
use std::fs;

fn dump(span: &protos::DataModel::Span, depth: usize) {
    let pad = "  ".repeat(depth);
    match span.value.as_ref() {
        Some(Span_oneof_value::plain(p)) => {
            println!("{}plain({:?})", pad, p.text)
        }
        Some(Span_oneof_value::tagged(t)) => {
            println!("{}tag[{}] attrs={:?}", pad, t.tag, t.attributes);
            for s in &t.spans {
                dump(s, depth + 1);
            }
        }
        Some(Span_oneof_value::break_line(_)) => println!("{}br", pad),
        _ => println!("{}other", pad),
    }
}

fn main() {
    let path = std::env::args().nth(1).expect("file");
    let raw = fs::read_to_string(&path).unwrap();
    let content = text::parse_content(&raw);
    for s in &content.spans {
        dump(s, 0);
    }
}

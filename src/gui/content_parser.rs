//! Content parsing utilities — splits markdown into text, code blocks, and inline code.

pub(super) enum ContentPart<'a> {
    Text(&'a str),
    CodeBlock {
        language: Option<&'a str>,
        code: &'a str,
    },
    InlineCode(&'a str),
}

pub(super) fn split_by_code_blocks(content: &str) -> Vec<ContentPart<'_>> {
    let mut parts = Vec::new();
    let mut remaining = content;

    while !remaining.is_empty() {
        if let Some(start_idx) = remaining.find("```") {
            // Text before code block
            if start_idx > 0 {
                let text = &remaining[..start_idx];
                parts.extend(split_inline_code(text));
            }

            // Find end of code block
            let after_start = &remaining[start_idx + 3..];
            let newline_idx = after_start.find('\n').unwrap_or(0);
            let language = if newline_idx > 0 {
                Some(after_start[..newline_idx].trim())
            } else {
                None
            };

            let code_start = start_idx + 3 + newline_idx + if newline_idx > 0 { 1 } else { 0 };

            if let Some(end_idx) = remaining[code_start..].find("```") {
                let code = remaining[code_start..code_start + end_idx].trim_end();
                parts.push(ContentPart::CodeBlock { language, code });
                remaining = &remaining[code_start + end_idx + 3..];
            } else {
                // Unclosed code block
                let code = remaining[code_start..].trim_end();
                parts.push(ContentPart::CodeBlock { language, code });
                break;
            }
        } else {
            parts.extend(split_inline_code(remaining));
            break;
        }
    }

    parts
}

pub(super) fn split_inline_code(text: &str) -> Vec<ContentPart<'_>> {
    let mut parts = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if let Some(start_idx) = remaining.find('`') {
            if start_idx > 0 {
                parts.push(ContentPart::Text(&remaining[..start_idx]));
            }

            let after_start = &remaining[start_idx + 1..];
            if let Some(end_idx) = after_start.find('`') {
                let code = &after_start[..end_idx];
                parts.push(ContentPart::InlineCode(code));
                remaining = &after_start[end_idx + 1..];
            } else {
                parts.push(ContentPart::Text(&remaining[start_idx..]));
                break;
            }
        } else {
            parts.push(ContentPart::Text(remaining));
            break;
        }
    }

    parts
}

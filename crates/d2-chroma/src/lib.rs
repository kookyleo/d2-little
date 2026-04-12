//! Minimal chroma-compatible syntax highlighter for d2 code blocks.
//!
//! Implements a regex-based lexer engine matching Go's alecthomas/chroma v2,
//! with lexer definitions for Go and Bash, plus github and catppuccin-mocha
//! theme styles for SVG output.

mod engine;
mod go_lexer;
mod bash_lexer;
mod themes;

pub use engine::{Token, TokenType};
pub use themes::{Theme, StyleEntry};

/// Tokenize source code using the given language name.
/// Returns None if the language is not supported.
pub fn tokenize(language: &str, source: &str) -> Option<Vec<Token>> {
    let lang = language.to_lowercase();
    let lexer: Box<dyn engine::Lexer> = match lang.as_str() {
        "go" | "golang" => Box::new(go_lexer::GoLexer::new()),
        "bash" | "sh" | "ksh" | "zsh" | "shell" => Box::new(bash_lexer::BashLexer::new()),
        _ => return None,
    };
    Some(lexer.tokenize(source))
}

/// Split a flat token list into lines, exactly matching chroma.SplitTokensIntoLines.
///
/// Each line ends with a token whose value ends with '\n' (or the last line
/// if the input doesn't end with newline). When a token spans a newline
/// boundary, it is split: the head (including \n) goes to the current line
/// which is then flushed, and the tail continues processing.
pub fn split_into_lines(tokens: &[Token]) -> Vec<Vec<Token>> {
    let mut out: Vec<Vec<Token>> = Vec::new();
    let mut line: Vec<Token> = Vec::new();

    for token in tokens {
        let mut value = token.value.clone();
        while value.contains('\n') {
            // SplitAfterN equivalent: split at first \n, keeping \n in the head
            let nl_pos = value.find('\n').unwrap();
            let head = value[..=nl_pos].to_string();
            let tail = value[nl_pos + 1..].to_string();
            value = tail;

            line.push(Token {
                token_type: token.token_type,
                value: head,
            });
            out.push(std::mem::take(&mut line));
        }
        // Remaining part (after all newlines processed) — may be empty
        line.push(Token {
            token_type: token.token_type,
            value,
        });
    }

    if !line.is_empty() {
        out.push(line);
    }

    // Strip empty trailing token line: if last line is a single token with empty value
    if let Some(last) = out.last() {
        if last.len() == 1 && last[0].value.is_empty() {
            out.pop();
        }
    }

    out
}

/// Get a theme by name. Supported: "github", "catppuccin-mocha".
pub fn get_theme(name: &str) -> Option<Theme> {
    themes::get_theme(name)
}

/// Escape text for SVG code rendering, matching chroma's svgEscaper.
pub fn svg_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace(' ', "&#160;")
        .replace('\t', "&#160;&#160;&#160;&#160;")
}

/// Get the SVG style attribute for a token type from a theme.
/// Returns the fill/font-weight/font-style + class attributes, or empty string
/// if no style applies. Matches Go's styleAttr function.
pub fn style_attr(theme: &Theme, tt: TokenType) -> String {
    // Look up style: try exact type, then subcategory, then category
    let entry = theme.get(tt)
        .or_else(|| theme.get(tt.sub_category()))
        .or_else(|| theme.get(tt.category()));

    let entry = match entry {
        Some(e) if !e.is_zero() => e,
        _ => return String::new(),
    };

    let mut parts = Vec::new();
    let mut classes = Vec::new();

    if let Some(ref color) = entry.color {
        parts.push(format!(r#"fill="{}""#, color));
    }
    if entry.bold {
        classes.push("text-mono-bold");
    }
    if entry.italic {
        classes.push("text-mono-italic");
    }

    let mut out = parts.join(" ");
    if !classes.is_empty() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&format!(r#"class="{}""#, classes.join(" ")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_go_tokenize() {
        let src = r#"func main() {
	println("hello")
}
"#;
        let tokens = tokenize("go", src).unwrap();
        // Should have KeywordDeclaration for "func"
        assert_eq!(tokens[0].token_type, TokenType::KeywordDeclaration);
        assert_eq!(tokens[0].value, "func");
    }

    #[test]
    fn test_go_keyword_if() {
        let tokens = tokenize("go", "if x {\n}\n").unwrap();
        assert_eq!(tokens[0].token_type, TokenType::Keyword);
        assert_eq!(tokens[0].value, "if");
    }

    #[test]
    fn test_bash_tokenize() {
        let src = "#!/usr/bin/env bash\necho testing\n";
        let tokens = tokenize("sh", src).unwrap();
        assert_eq!(tokens[0].token_type, TokenType::CommentPreproc);
    }

    #[test]
    fn test_split_into_lines_single_multiline_token() {
        // Single token spanning two lines
        let tokens = vec![
            Token { token_type: TokenType::CommentSingle, value: "// line1\n// line2\n".to_string() },
        ];
        let lines = split_into_lines(&tokens);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].len(), 1);
        assert_eq!(lines[0][0].value, "// line1\n");
        assert_eq!(lines[1].len(), 1);
        assert_eq!(lines[1][0].value, "// line2\n");
    }

    #[test]
    fn test_split_into_lines_separate_tokens() {
        // Two separate tokens, each ending with \n (like chroma Go output)
        let tokens = vec![
            Token { token_type: TokenType::CommentSingle, value: "// line1\n".to_string() },
            Token { token_type: TokenType::CommentSingle, value: "// line2\n".to_string() },
        ];
        let lines = split_into_lines(&tokens);
        assert_eq!(lines.len(), 2);
        // Line 0: just the first comment
        assert_eq!(lines[0].len(), 1);
        assert_eq!(lines[0][0].value, "// line1\n");
        // Line 1: empty continuation from first token's tail, then second comment
        assert_eq!(lines[1].len(), 2);
        assert_eq!(lines[1][0].value, "");
        assert_eq!(lines[1][1].value, "// line2\n");
    }
}

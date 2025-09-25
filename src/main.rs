use regex::Regex;
use std::{
    collections::HashMap,
    env::{self},
    error::Error,
    fs, process,
};
use tree_sitter::Language;
use tree_sitter_highlight::{Highlight, HighlightConfiguration, Highlighter, HtmlRenderer};

unsafe extern "C" {
    fn tree_sitter_rust() -> Language;
    fn tree_sitter_javascript() -> Language;
    fn tree_sitter_html() -> Language;
    fn tree_sitter_css() -> Language;
    fn tree_sitter_python() -> Language;
}

fn language_map() -> HashMap<&'static str, (Language, &'static str, &'static str, &'static str)> {
    let mut m = HashMap::new();
    m.insert(
        "rust",
        (
            unsafe { tree_sitter_rust() },
            tree_sitter_rust::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
    );
    m.insert(
        "javascript",
        (
            unsafe { tree_sitter_javascript() },
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            "",
            "",
        ),
    );
    m.insert(
        "html",
        (
            unsafe { tree_sitter_html() },
            tree_sitter_html::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
    );
    m.insert(
        "css",
        (
            unsafe { tree_sitter_css() },
            tree_sitter_css::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
    );
    m.insert(
        "python",
        (
            unsafe { tree_sitter_python() },
            tree_sitter_python::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
    );
    m
}

/// Highlight source (raw code string, not HTML-escaped).
fn highlight_to_html(
    language: Language,
    language_name: &str,
    highlights_query: &str,
    injections_query: &str,
    locals_query: &str,
    source: &str,
) -> Result<String, Box<dyn Error>> {
    let mut config = HighlightConfiguration::new(
        language,
        language_name,
        highlights_query,
        injections_query,
        locals_query,
    )?;

    // copy names first to avoid borrow conflicts
    let names_vec: Vec<String> = config.names().iter().map(|s| s.to_string()).collect();
    let names_slice: Vec<&str> = names_vec.iter().map(|s| s.as_str()).collect();
    config.configure(&names_slice);

    let mut highlighter = Highlighter::new();
    let iter = highlighter.highlight(&config, source.as_bytes(), None, |_| None)?;

    let mut renderer = HtmlRenderer::new();

    let names_for_cb = names_vec; // move into closure
    let attribute_callback = move |h: Highlight, out: &mut Vec<u8>| {
        if let Some(name) = names_for_cb.get(h.0) {
            let classes = name.replace('.', " ");
            out.extend_from_slice(b"class=\"");
            out.extend_from_slice(classes.as_bytes());
            out.extend_from_slice(b"\"");
        }
    };

    renderer.render(iter, source.as_bytes(), &attribute_callback)?;
    Ok(String::from_utf8(renderer.html)?)
}

/// Basic HTML entity unescape for common entities and numeric entities.
fn html_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if &s[i..i + 1.min(s.len() - i)] == "&" {
            if s[i..].starts_with("&lt;") {
                out.push('<');
                i += 4;
            } else if s[i..].starts_with("&gt;") {
                out.push('>');
                i += 4;
            } else if s[i..].starts_with("&amp;") {
                out.push('&');
                i += 5;
            } else if s[i..].starts_with("&quot;") {
                out.push('"');
                i += 6;
            } else if s[i..].starts_with("&#x") || s[i..].starts_with("&#X") {
                if let Some(j) = s[i..].find(';') {
                    let hex = &s[i + 3..i + j];
                    if let Ok(code) = u32::from_str_radix(hex, 16)
                        && let Some(ch) = std::char::from_u32(code)
                    {
                        out.push(ch);
                        i += j + 1;
                        continue;
                    }
                }
                out.push('&');
                i += 1;
            } else if s[i..].starts_with("&#") {
                if let Some(j) = s[i..].find(';') {
                    let num = &s[i + 2..i + j];
                    if let Ok(code) = num.parse::<u32>()
                        && let Some(ch) = std::char::from_u32(code)
                    {
                        out.push(ch);
                        i += j + 1;
                        continue;
                    }
                }
                out.push('&');
                i += 1;
            } else {
                out.push('&');
                i += 1;
            }
        } else {
            let ch = s[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

fn extract_class_attr(attrs: &str) -> Option<String> {
    let re_dq = Regex::new(r#"class\s*=\s*"([^"]+)""#).unwrap();
    if let Some(cap) = re_dq.captures(attrs) {
        return Some(cap[1].to_string());
    }
    let re_sq = Regex::new(r#"class\s*=\s*'([^']+)'"#).unwrap();
    if let Some(cap) = re_sq.captures(attrs) {
        return Some(cap[1].to_string());
    }
    None
}

/// update attrs string: add `add_classes` and optionally `add_lang` to the `class` attribute.
/// returns attrs string starting with a leading space if non-empty (so it can be inserted into `<tag{attrs}>`).
fn update_or_add_class(attrs: &str, add_classes: &str, add_lang: Option<&str>) -> String {
    let mut attrs_owned = attrs.to_string();
    let re_dq = Regex::new(r#"class\s*=\s*"([^"]*)""#).unwrap();
    let re_sq = Regex::new(r#"class\s*=\s*'([^']*)'"#).unwrap();

    let mut combined = String::new();
    if !add_classes.is_empty() {
        combined.push_str(add_classes);
    }
    if let Some(l) = add_lang {
        if !combined.is_empty() {
            combined.push(' ');
        }
        combined.push_str(l);
    }

    if let Some(cap) = re_dq.captures(&attrs_owned) {
        let existing = &cap[1];
        let mut parts: Vec<&str> = existing.split_whitespace().collect();
        for c in combined.split_whitespace() {
            if !parts.contains(&c) {
                parts.push(c);
            }
        }
        let new_class = parts.join(" ");
        attrs_owned = re_dq
            .replace(&attrs_owned, format!("class=\"{}\"", new_class).as_str())
            .to_string();
    } else if let Some(cap) = re_sq.captures(&attrs_owned) {
        let existing = &cap[1];
        let mut parts: Vec<&str> = existing.split_whitespace().collect();
        for c in combined.split_whitespace() {
            if !parts.contains(&c) {
                parts.push(c);
            }
        }
        let new_class = parts.join(" ");
        attrs_owned = re_sq
            .replace(&attrs_owned, format!("class='{}'", new_class).as_str())
            .to_string();
    } else if !combined.is_empty() {
        if attrs_owned.trim().is_empty() {
            attrs_owned = format!(" class=\"{}\"", combined);
        } else {
            attrs_owned = format!("{} class=\"{}\"", attrs_owned, combined);
        }
    }

    if attrs_owned.trim().is_empty() {
        "".to_string()
    } else if attrs_owned.starts_with(' ') {
        attrs_owned
    } else {
        format!(" {}", attrs_owned)
    }
}

/// Process HTML, find <pre ...><code ...> blocks, detect language from class on pre or code,
/// decode entities, highlight, add `sourceCode` to both pre and code, and safely insert highlighted HTML.
fn highlight_html(input: &str) -> String {
    let re = Regex::new(
        r"(?s)<pre(?P<pre_attrs>[^>]*)>\s*<code(?P<code_attrs>[^>]*)>(?P<code>.*?)</code>\s*</pre>",
    )
    .unwrap();
    let configs = language_map();

    re.replace_all(input, |caps: &regex::Captures| {
        let pre_attrs = caps.name("pre_attrs").map(|m| m.as_str()).unwrap_or("");
        let code_attrs = caps.name("code_attrs").map(|m| m.as_str()).unwrap_or("");
        let code_html_escaped = caps.name("code").map(|m| m.as_str()).unwrap_or("");

        // pick language: prefer code class, then pre class, else none
        let code_class = extract_class_attr(code_attrs);
        let pre_class = extract_class_attr(pre_attrs);

        let mut lang_opt: Option<String> = None;
        if let Some(cc) = code_class.clone() {
            for token in cc.split_whitespace() {
                if configs.contains_key(token) {
                    lang_opt = Some(token.to_string());
                    break;
                }
            }
        }
        if lang_opt.is_none()
            && let Some(pc) = pre_class.clone()
        {
            for token in pc.split_whitespace() {
                if configs.contains_key(token) {
                    lang_opt = Some(token.to_string());
                    break;
                }
            }
        }

        if let Some(lang) = lang_opt {
            let (lang_obj, highlights, injections, locals) = configs.get(lang.as_str()).unwrap();

            let decoded = html_unescape(code_html_escaped);

            match highlight_to_html(
                lang_obj.clone(),
                &lang,
                highlights,
                injections,
                locals,
                &decoded,
            ) {
                Ok(rendered) => {
                    let new_pre_attrs = update_or_add_class(pre_attrs, "sourceCode", None);
                    // ensure code gets both sourceCode and the language class so downstream CSS / js can find it
                    let new_code_attrs = update_or_add_class(code_attrs, "sourceCode", Some(&lang));
                    format!(
                        "<div class=\"sourceCode\"><pre{}><code{}>{}</code></pre></div>",
                        new_pre_attrs, new_code_attrs, rendered
                    )
                }
                Err(_) => caps[0].to_string(),
            }
        } else {
            caps[0].to_string()
        }
    })
    .to_string()
}

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() != 2 {
        eprintln!("Usage: {} input.html", args[0]);
        process::exit(1);
    }

    let file = &args[1];
    let html = match fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Couldn't read {file} ({e})");
            process::exit(1);
        }
    };
    // let html = r#"
    //     <p>Rust:</p>
    //     <pre><code class="rust">println!("Hello");</code></pre>
    //
    //     <p>JS:</p>
    //     <pre><code class="javascript">let x = 42;</code></pre>
    // "#;

    let highlighted = highlight_html(&html);
    print!("{}", highlighted);
}

use scraper::{Html, Selector};
use tree_sitter::Language;
use tree_sitter_highlight::{Highlighter, HighlightConfiguration, HtmlRenderer, Highlight};

unsafe extern "C" {
    fn tree_sitter_javascript() -> Language;
}

fn highlight_to_html(
    language: Language,
    language_name: &str,
    highlights_query: &str,
    injections_query: &str,
    locals_query: &str,
    source: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut config = HighlightConfiguration::new(
        language,
        language_name,
        highlights_query,
        injections_query,
        locals_query,
    )?;

    let names: Vec<String> = config.names().iter().map(|s| s.to_string()).collect();
    config.configure(&names.iter().map(|s| s.as_str()).collect::<Vec<_>>());

    let mut highlighter = Highlighter::new();
    let iter = highlighter.highlight(&config, source.as_bytes(), None, |_| None)?;

    let mut renderer = HtmlRenderer::new();

    let attribute_callback = move |h: Highlight, out: &mut Vec<u8>| {
        if let Some(name) = names.get(h.0) {
            let classes = name.replace('.', " ");
            out.extend_from_slice(b"class=\"");
            out.extend_from_slice(classes.as_bytes());
            out.extend_from_slice(b"\"");
        }
    };

    renderer.render(iter, source.as_bytes(), &attribute_callback)?;

    Ok(String::from_utf8(renderer.html)?)
}

/// Replace all <code class="language-..."> blocks with highlighted spans
fn highlight_html(input: &str) -> String {
    let document = Html::parse_fragment(input);
    let selector = Selector::parse("code[class^='language-']").unwrap();

    let mut output = input.to_string();

    for element in document.select(&selector) {
        if let Some(class) = element.value().attr("class") {
            if class == "language-javascript" {
                let raw_code = element.text().collect::<Vec<_>>().join("");
                let lang = unsafe { tree_sitter_javascript() };

                let highlighted = highlight_to_html(
                    lang,
                    "javascript",
                    tree_sitter_javascript::HIGHLIGHT_QUERY,
                    "",
                    "",
                    &raw_code,
                )
                .unwrap();

                let replacement = format!("<code class=\"{}\">{}</code>", class, highlighted);
                output = output.replace(&element.html(), &replacement);
            }
        }
    }
    output
}

fn main() {
    let html = r#"
        <p>Example:</p>
        <pre><code class="language-javascript">let x = 42;</code></pre>
    "#;

    let highlighted = highlight_html(html);
    println!("{}", highlighted);
}


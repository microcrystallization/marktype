use marktype::engine::Document;
use marktype::highlight::highlight_code;

#[test]
fn renders_typora_extensions_and_latex_intact() {
    let sample = concat!(
        "---\n",
        "title: MarkType Test\n",
        "author: Rust\n",
        "---\n\n",
        "# Heading $E=mc^2$\n\n",
        "Inline ==highlight==, H~2~O, x^2^, and $\\alpha_1 + \\beta_2$.\n\n",
        "> [!TIP] Fast `<10ms` Sync\n",
        "> Inline math $\\frac{1}{2}$ inside callout.\n\n",
        "| Col A | Col B |\n",
        "| :--- | ---: |\n",
        "| $x^2$ | **bold** |\n\n",
        "```rust\n",
        "fn main() { println!(\"hello\"); }\n",
        "```\n"
    );
    let mut doc = Document::new("md".into(), "md.md".into(), None, sample);
    let html = doc.render_full_html();

    assert!(html.contains("mt-frontmatter"));
    assert!(html.contains("mt-math-inline"));
    assert!(html.contains("data-tex=\"\\alpha_1 + \\beta_2\""));
    assert!(html.contains("<mark>highlight</mark>"));
    assert!(html.contains("<sub>2</sub>"));
    assert!(html.contains("<sup>2</sup>"));
    assert!(html.contains("mt-callout-tip"));
    assert!(html.contains("<code class=\"mt-inline-code\">&lt;10ms</code>"));
    assert!(html.contains("<table>"));
    assert!(html.contains("hl-kw"));
    assert!(html.contains("hl-mac"));
}

#[test]
fn sanitizes_dangerous_html_tags() {
    let sample = "Safe <kbd>Ctrl</kbd> and <script>alert(1)</script>";
    let mut doc = Document::new("sec".into(), "sec.md".into(), None, sample);
    let html = doc.render_full_html();
    assert!(html.contains("<kbd>Ctrl</kbd>"));
    assert!(!html.contains("<script>alert(1)</script>"));
    assert!(html.contains("&lt;script&gt;"));
}

#[test]
fn syntax_highlighter_covers_multiple_languages() {
    let py = highlight_code("def solve(x):\n    return x * 42 # comment", "python");
    assert!(py.contains("hl-kw"));
    assert!(py.contains("hl-num"));
    assert!(py.contains("hl-cmt"));

    let json = highlight_code("{\"key\": true, \"num\": 12}", "json");
    assert!(json.contains("hl-key"));
    assert!(json.contains("hl-kw"));
    assert!(highlight_code("let answer = 42;", "").contains("hl-num"));
    assert_eq!(
        highlight_code("let answer = 42;", "text"),
        "let answer = 42;"
    );
    assert!(json.contains("hl-num"));
}

#[test]
fn links_and_footnotes_resolve_across_blocks_and_refresh_after_an_edit() {
    let source = "See [example][ref] and [^1].\n\n[ref]: https://example.com/first\n\n[^1]: Note";
    let mut doc = Document::new("refs".into(), "refs.md".into(), None, source);
    let html = doc.render_full_html();
    assert!(html.contains("href=\"https://example.com/first\""));
    assert!(html.contains("class=\"mt-fn-ref\"><a href=\"#fn-1\""));
    let start = source.find("/first").unwrap() + 1;
    let patch = doc.apply_edit(start, start + 5, "second", true);
    assert!(patch.refresh_preview);
    assert!(
        doc.render_full_html()
            .contains("href=\"https://example.com/second\"")
    );
}

#[test]
fn raw_html_attributes_are_escaped_while_basic_formatting_survives() {
    let text = "Text <kbd>Ctrl</kbd> and <span onclick = \"\">label</span>.";
    let mut doc = Document::new("html".into(), "html.md".into(), None, text);
    let html = doc.render_full_html();
    assert!(html.contains("<kbd>Ctrl</kbd>"));
    assert!(html.contains("&lt;span onclick = &quot;&quot;&gt;"));
    assert!(!html.contains("<span onclick"));
}

#[test]
fn html_exports_include_math_fonts_chemistry_and_syntax_colors() {
    let mut doc = Document::new(
        "export".into(),
        "export.md".into(),
        None,
        "$\\ce{H2O}$\n\n```rust\nlet count = 42;\n```\n",
    );
    let html = marktype::assets::export_html_template("Sample", &doc.render_full_html(), true);
    assert!(html.contains("data:font/woff2;base64,"));
    assert!(!html.contains("url(fonts/"));
    assert!(!html.contains("cdn.jsdelivr"));
    assert!(html.contains(r"\ce"));
    assert!(html.contains("<span class=\"hl-kw\">let</span>"));
    assert!(html.contains("class=\"light\"") && html.contains(".hl-kw { color: var(--hl-kw); }"));
}

#[test]
fn only_registered_images_in_the_document_folder_are_served() {
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().join("documents");
    std::fs::create_dir(&folder).unwrap();
    let jpeg = include_bytes!("../assets/wallpapers/great-wave.jpg");
    std::fs::write(folder.join("inside.jpg"), jpeg).unwrap();
    std::fs::write(temp.path().join("outside.jpg"), jpeg).unwrap();
    let mut doc = Document::new(
        "images".into(),
        "images.md".into(),
        Some(folder.join("images.md")),
        "![Inside](inside.jpg)\n\n![Outside](../outside.jpg)",
    );
    let html = doc.render_full_html();
    let url = html
        .split_once("src=\"")
        .unwrap()
        .1
        .split('"')
        .next()
        .unwrap();
    let (status, mime, image) = marktype::assets::serve_request(url);
    assert_eq!((status, mime), (200, "image/jpeg"));
    assert_eq!(image, jpeg);
    assert_eq!(html.matches("<img ").count(), 1);
    assert_eq!(
        marktype::assets::serve_request("/local?path=inside.jpg").0,
        404
    );
    let exported = marktype::assets::export_html_template("Images", &html, false);
    assert!(exported.contains("src=\"data:image/jpeg;base64,"));
}

use std::time::Instant;

use marktype::engine::Document;

#[test]
fn large_document_open_and_sub_10ms_incremental_edit() {
    let mut big = String::with_capacity(2_600_000);
    for i in 0..5000 {
        big.push_str(&format!(
            "## Section {i}\n\nParagraph {i} with inline math $E = mc^2 + {i}$ and **bold** text.\n\n$$\n\\int_0^\\infty e^{{-x^2}} dx = \\frac{{\\sqrt{{\\pi}}}}{{2}}\n$$\n\n- Task item A\n- [x] Done item B\n\n"
        ));
    }

    let t_open = Instant::now();
    let mut doc = Document::new("big".into(), "big.md".into(), None, &big);
    let open_ms = t_open.elapsed().as_millis();
    println!(
        "50k-line doc open: {open_ms} ms ({} blocks)",
        doc.blocks.len()
    );
    assert!(doc.blocks.len() >= 20_000);
    assert!(doc.is_large());
    assert!(open_ms < 500, "Large doc open took {open_ms}ms");

    let mid_char = doc.rope.len_chars() / 2;
    let patch = doc.apply_edit(mid_char, mid_char, "X", false);
    println!(
        "50k-line 1-char incremental edit: {} µs ({:.3} ms), patched {} block(s)",
        patch.rust_us,
        patch.rust_us as f64 / 1000.0,
        patch.inserted.len()
    );
    assert!(
        patch.rust_us < 10_000,
        "Incremental edit took {} µs (> 10ms)",
        patch.rust_us
    );
    assert!(patch.inserted.len() <= 3);
}

#[test]
fn utf8_bom_heading_parses_cleanly() {
    let mut doc = Document::new(
        "bom".into(),
        "bom.md".into(),
        None,
        "\u{feff}# Hello BOM\n\nParagraph body",
    );
    assert_eq!(doc.outline().len(), 1);
    assert_eq!(doc.outline()[0].text, "Hello BOM");
    assert!(
        doc.render_full_html()
            .contains("<h1 id=\"hello-bom\">Hello BOM</h1>")
    );
}

#[test]
fn toggle_task_checkbox_updates_rope_and_returns_patch() {
    let mut doc = Document::new(
        "tasks".into(),
        "tasks.md".into(),
        None,
        "- [ ] Write tests\n- [x] Ship release",
    );
    let patch = doc.toggle_task_line(0).expect("line 0 should toggle");
    assert!(doc.line_str(0).contains("[x]"));
    assert_eq!(patch.inserted.len(), 1);

    let patch2 = doc.toggle_task_line(0).expect("line 0 should untoggle");
    assert!(doc.line_str(0).contains("[ ]"));
    assert_eq!(patch2.inserted.len(), 1);
}

#[test]
fn search_and_replace_across_rope() {
    let mut doc = Document::new(
        "sr".into(),
        "sr.md".into(),
        None,
        "Alpha beta alpha\nGamma ALPHA delta",
    );
    let insensitive = doc.search("alpha", false, false, 100);
    assert_eq!(insensitive.len(), 3);

    let sensitive = doc.search("alpha", true, false, 100);
    assert_eq!(sensitive.len(), 1);

    let replaced = doc.replace_all("alpha", "omega", false, false);
    assert_eq!(replaced, 3);
    assert_eq!(doc.search("omega", true, false, 100).len(), 3);
}

#[test]
fn fixture_file_parses_outline_and_stats() {
    let content = include_str!("fixtures/sample_latex.md");
    let mut doc = Document::new("fix".into(), "sample_latex.md".into(), None, content);
    assert!(!doc.outline().is_empty());
    let stats = doc.stats();
    assert!(stats.words > 20);
    assert!(stats.lines > 15);
    let html = doc.render_full_html();
    assert!(html.contains("mt-math-block"));
    assert!(html.contains("mt-math-inline"));
}

fn document(text: &str) -> Document {
    Document::new("test".into(), "test.md".into(), None, text)
}

#[test]
fn edited_lists_match_a_fresh_parse() {
    let mut doc = document("- a\n\ntext");
    doc.apply_edit(5, 9, "- b", true);
    assert_eq!(
        doc.render_full_html(),
        document("- a\n\n- b").render_full_html()
    );
}

#[test]
fn shifted_blocks_keep_relative_tasks_and_update_the_outline() {
    let mut doc =
        document("Introduction\n\nOther\n\n- [X] Task with [ ] in its label\n\n# Heading");
    let task_id = doc.blocks[2].id;
    doc.render_full_html();
    let patch = doc.apply_edit(0, 0, "\n", true);
    assert_eq!(patch.outline.unwrap()[0].line, 7);
    assert_eq!(doc.blocks[2].id, task_id);
    assert_eq!(doc.blocks[2].start_line, 5);
    assert!(
        doc.render_full_html()
            .contains("data-line-offset=\"0\" checked")
    );
    doc.toggle_task_line(5).unwrap();
    assert_eq!(doc.line_str(5), "- [ ] Task with [ ] in its label");
}

#[test]
fn incremental_word_counts_follow_inserted_and_removed_boundaries() {
    let mut doc = document("a");
    doc.apply_edit(1, 1, "b", false);
    doc.apply_edit(2, 2, "c", false);
    assert_eq!(doc.stats().words, 1);
    doc.apply_edit(1, 1, " ", false);
    assert_eq!(doc.stats().words, 2);
    doc.apply_edit(1, 2, "", false);
    assert_eq!(doc.stats().words, 1);
    doc.apply_edit(1, 2, "中文", false);
    assert_eq!(doc.stats().words, 4);
}

#[test]
fn regex_and_literal_replacements_use_the_same_positions_as_search() {
    let mut doc = document("😀 price\nprice\nitem = 42");
    let hit = &doc.search("price", true, false, 1)[0];
    let query = marktype::engine::SearchQuery {
        query: "price",
        case_sensitive: true,
        use_regex: false,
    };
    assert_eq!(hit.col, 2);
    doc.replace_match(hit.start_char, hit.end_char, &query, "$10")
        .unwrap();
    assert_eq!(doc.replace_all("price", "$20", true, false), 1);
    assert_eq!(doc.replace_all(r"item = (\d+)", "count: $1", true, true), 1);
    assert_eq!(doc.search(r"\$20\ncount", true, true, 1)[0].end_line, 2);
    assert_eq!(doc.full_text(), "😀 $10\n$20\ncount: 42");
}

#[test]
fn editor_offsets_are_normalized_and_saving_preserves_bom_and_crlf() {
    let mut doc = document("\u{feff}First\r\n😀 next\r\n");
    assert_eq!(doc.full_text(), "First\n😀 next\n");
    doc.apply_edit(8, 8, "new ", false);
    let mut bytes = Vec::new();
    doc.write_text(&mut bytes).unwrap();
    assert_eq!(
        String::from_utf8(bytes).unwrap(),
        "\u{feff}First\r\n😀 new next\r\n"
    );
}

#[test]
fn incremental_block_boundaries_and_statistics_match_reopening() {
    let mut doc = document(
        "---\ntitle: test\n---\n\n# Heading\n\nParagraph\n\n- one\n- two\n\n```rust\nlet a = 1;\n```\n\n# End",
    );
    let fragments = [
        "", "x", "\n", "\n\n", "# H\n", "```\n", "---", "|a|", "😀", "- a\n", "$$\n",
    ];
    let mut rng = 71u64;
    for step in 0..500 {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        let start = (rng as usize) % (doc.rope.len_chars() + 1);
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        let end = (start + rng as usize % 8).min(doc.rope.len_chars());
        let fragment = fragments[(rng >> 32) as usize % fragments.len()];
        doc.apply_edit(start, end, fragment, true);
        let rebuilt = document(&doc.full_text());
        let boundaries = |doc: &Document| {
            doc.blocks
                .iter()
                .map(|b| (b.start_line, b.end_line, b.kind.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            boundaries(&doc),
            boundaries(&rebuilt),
            "edit {step}: {:?}",
            doc.full_text()
        );
        assert_eq!(doc.stats().words, rebuilt.stats().words, "edit {step}");
    }
}

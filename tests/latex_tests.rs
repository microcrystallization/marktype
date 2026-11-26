use std::fs;
use std::path::Path;

use marktype::latex::{Build, Engine, detect_root, main_file};

fn project(path: &Path) {
    fs::create_dir_all(path.join("sections")).unwrap();
    fs::create_dir_all(path.join("figures")).unwrap();
    for relative in [
        "main.tex",
        "sections/introduction.tex",
        "sections/results.tex",
        "references.bib",
        "figures/measurements.png",
    ] {
        fs::copy(
            Path::new("examples/latex-paper").join(relative),
            path.join(relative),
        )
        .unwrap();
    }
}

#[test]
fn recognizes_document_roots_and_does_not_guess_between_two_papers() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("Paper with spaces");
    project(&directory);
    let root = main_file(&directory.join("main.tex")).unwrap();
    let chapter = directory.join("sections/results.tex");
    assert_eq!(detect_root(&chapter, None), Some(root.clone()));
    assert_eq!(detect_root(&root, None), Some(root.clone()));
    assert_eq!(
        detect_root(&directory.join("references.bib"), None),
        Some(root.clone())
    );
    fs::copy(&root, directory.join("another.tex")).unwrap();
    assert_eq!(detect_root(&directory.join("references.bib"), None), None);
    assert_eq!(detect_root(&chapter, None), Some(root));
}

#[test]
fn compiles_a_paper_and_preserves_the_last_pdf_on_tex_errors() {
    let Some(engine) = std::env::var_os("MARKTYPE_TEST_TEX_ENGINE") else {
        return;
    };
    let engine = engine.to_str().unwrap();
    let engine: Engine = serde_json::from_str(&format!("\"{engine}\"")).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("Paper with spaces");
    project(&directory);
    let main = directory.join("main.tex");
    let result = Build::prepare(main.clone(), engine).unwrap().run(|_| {});
    assert!(result.error.is_none(), "{:?}\n{}", result.error, result.log);
    assert!(
        result.log.to_ascii_lowercase().contains("bibtex"),
        "{}",
        result.log
    );
    let pdf = result.pdf.unwrap();
    let bytes = fs::read(&pdf).unwrap();
    assert!(bytes.starts_with(b"%PDF-"));
    assert!(bytes.len() > 4_000);
    assert!(!main.with_extension("aux").exists());
    assert!(result.synctex, "SyncTeX should be generated during build");
    let synctex = marktype::synctex::SyncTex::load_for_pdf(&pdf).unwrap();
    let intro = directory.join("sections/introduction.tex");
    let forward = synctex.forward(&intro, 3).unwrap();
    assert_eq!(forward.page, 1);
    assert!(!forward.boxes.is_empty());
    let first_box = &forward.boxes[0];
    let inverse = synctex
        .inverse(
            forward.page,
            first_box.x + first_box.width * 0.5,
            first_box.y + first_box.height * 0.5,
        )
        .unwrap();
    assert_eq!(
        dunce::canonicalize(&inverse.path).unwrap(),
        dunce::canonicalize(&intro).unwrap()
    );

    fs::write(
        directory.join("sections/results.tex"),
        "\\NonexistentMarkTypeCommand",
    )
    .unwrap();
    let failed = Build::prepare(main, engine).unwrap().run(|_| {});
    assert!(failed.error.is_some());
    assert!(failed.pdf.is_none());
    assert!(
        failed.log.contains("NonexistentMarkTypeCommand"),
        "{}",
        failed.log
    );
    let retained = fs::read(pdf)
        .unwrap_or_else(|error| panic!("Previous PDF could not be read: {error}\n{}", failed.log));
    assert_eq!(retained, bytes);
}

#[test]
fn latexmk_runs_biber_for_biblatex_projects() {
    if std::env::var("MARKTYPE_TEST_TEX_ENGINE").as_deref() != Ok("pdf") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("Biber paper");
    project(&directory);
    let main = directory.join("main.tex");
    let contents = fs::read_to_string(&main).unwrap();
    let contents = contents.replace("\\begin{document}",
        "\\usepackage[backend=biber]{biblatex}\n\\addbibresource{references.bib}\n\\begin{document}")
        .replace("\\bibliographystyle{plain}\n\\bibliography{references}", "\\printbibliography");
    fs::write(&main, contents).unwrap();
    let result = Build::prepare(main, Engine::Pdf).unwrap().run(|_| {});
    assert!(result.error.is_none(), "{:?}\n{}", result.error, result.log);
    assert!(
        result.log.to_ascii_lowercase().contains("running 'biber"),
        "{}",
        result.log
    );
    assert!(result.pdf.unwrap().is_file());
}

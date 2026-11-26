use std::fs;
use std::io::Write;

use flate2::Compression;
use flate2::write::GzEncoder;
use marktype::synctex::SyncTex;

const SAMPLE_SYNCTEX: &str = "\
SyncTeX Version:1
Input:1:./main.tex
Input:2:./sections/introduction.tex
Output:pdf
Magnification:1000
Unit:1
X Offset:0
Y Offset:0
Content:
{1
[1,10:4718592,6578176:26214400,52428800,0
(1,12:4718592,9867264:20000000,786432,196608
x1,12:4718592,9867264
)
(2,5,3:4718592,13156352:18000000,786432,196608
x2,5,3:6578176,13156352
)
]
}
{2
[2,20:4718592,6578176:26214400,20000000,0
(2,22:4718592,11840717:19000000,786432,196608
)
]
}
Postamble:
Count:6
";

#[test]
fn parses_gzipped_synctex_and_performs_forward_and_inverse_sync() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Paper with spaces");
    fs::create_dir_all(root.join("sections")).unwrap();
    let main = root.join("main.tex");
    let intro = root.join("sections/introduction.tex");
    let pdf = root.join("main.pdf");
    let synctex_gz = root.join("main.synctex.gz");
    fs::write(&main, "\\documentclass{article}\n").unwrap();
    fs::write(&intro, "\\section{Introduction}\n").unwrap();
    fs::write(&pdf, b"%PDF-1.7\n").unwrap();

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(SAMPLE_SYNCTEX.as_bytes()).unwrap();
    fs::write(&synctex_gz, encoder.finish().unwrap()).unwrap();

    let st = SyncTex::load_for_pdf(&pdf).unwrap();

    // Forward sync on main.tex line 12 -> page 1
    let fwd_main = st.forward(&main, 12).unwrap();
    assert_eq!(fwd_main.page, 1);
    assert!(!fwd_main.boxes.is_empty());
    let b = &fwd_main.boxes[0];
    assert!((b.x - 71.73).abs() < 1.0, "unexpected x: {}", b.x);
    assert!((b.y - 138.0).abs() < 2.0, "unexpected y: {}", b.y);

    // Forward sync on nested chapter file sections/introduction.tex line 22 -> page 2
    let fwd_intro = st.forward(&intro, 22).unwrap();
    assert_eq!(fwd_intro.page, 2);
    assert!(!fwd_intro.boxes.is_empty());

    // Nearest-line fallback when exact line has no node (line 21 -> line 22)
    let fwd_nearest = st.forward(&intro, 21).unwrap();
    assert_eq!(fwd_nearest.page, 2);

    // Inverse sync on page 1 at intro box coordinates -> sections/introduction.tex line 5
    let inv_intro = st.inverse(1, 100.0, 195.0).unwrap();
    assert_eq!(
        dunce::canonicalize(&inv_intro.path).unwrap(),
        dunce::canonicalize(&intro).unwrap()
    );
    assert_eq!(inv_intro.line, 5);
    assert_eq!(inv_intro.column, Some(3));

    // Inverse sync on page 2 -> sections/introduction.tex line 22
    let inv_page2 = st.inverse(2, 100.0, 175.0).unwrap();
    assert_eq!(
        dunce::canonicalize(&inv_page2.path).unwrap(),
        dunce::canonicalize(&intro).unwrap()
    );
    assert_eq!(inv_page2.line, 22);
}

#[test]
fn parses_uncompressed_synctex_and_registers_local_pdf_url() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let main = root.join("paper.tex");
    let pdf = root.join("paper.pdf");
    let synctex_plain = root.join("paper.synctex");
    fs::write(&main, "\\documentclass{article}\n").unwrap();
    fs::write(&pdf, b"%PDF-1.7\n%test").unwrap();
    fs::write(
        &synctex_plain,
        SAMPLE_SYNCTEX.replace("./main.tex", "./paper.tex"),
    )
    .unwrap();

    let st = SyncTex::load_for_pdf(&pdf).unwrap();
    let fwd = st.forward(&main, 12).unwrap();
    assert_eq!(fwd.page, 1);

    let url = marktype::assets::register_pdf(&pdf).unwrap();
    assert!(url.starts_with("/local-pdf/"));
    let (status, mime, body) = marktype::assets::serve_request(&url);
    assert_eq!(status, 200);
    assert_eq!(mime, "application/pdf");
    assert!(body.starts_with(b"%PDF-"));
}

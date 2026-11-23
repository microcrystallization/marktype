# Contributing to MarkType

Thank you for your interest in contributing to **MarkType**!

## Development Setup

1. Install Rust (edition 2024, `rustc >= 1.88`): <https://rustup.rs>
2. On Windows, ensure Microsoft Edge WebView2 Runtime is installed (pre-installed on Windows 10/11).
3. Clone and run tests:

```bash
git clone https://github.com/microcrystallization/marktype.git
cd marktype
cargo test --locked --release
cargo run -- examples/getting-started.md
```

## Project Layout

```text
marktype/
├── src/                # Rust library & desktop binary (Rope engine, Markdown, Wallpaper FX, IPC)
├── ui/                 # Embedded frontend (HTML, CSS, i18n, JS)
├── assets/             # Bundled KaTeX, Geist fonts, and 7 CC0 Met Open Access artworks
├── tests/              # Integration tests & test fixtures
├── examples/           # Sample Markdown + LaTeX documents
└── docs/               # Screenshots and documentation
```

## Code Quality Checklist

Before opening a pull request, please verify:

- `cargo fmt --check` passes cleanly
- `cargo clippy --all-targets` reports no warnings
- `cargo test --release` passes all integration tests (including the `<10ms` incremental edit benchmark)
- Any new UI strings are added to `ui/i18n.js`

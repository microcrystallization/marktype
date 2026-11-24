# Changelog

## [0.1.1] - 2026-11-24

- Render images, GFM tables, link labels, and GitHub alerts directly in Live Preview. Click a widget to reveal its Markdown. Use Ctrl-click or Command-click to follow an external link.
- Reveal syntax only for the node touched by the cursor or selection. Adjacent inline formulas stay rendered while editing one formula.
- Highlight fenced Python, C++, Go, Bash, JSON, and YAML alongside Rust, JavaScript, and TypeScript.
- Load CodeMirror on entry to Live Preview, initialize code parsers when their languages appear, and batch widget requests. Large files use a windowed outline and defer Split preview rendering when opened in Live or Source mode.

## [0.1.0] - 2026-11-24

- Published packages for Windows x64, macOS on Apple Silicon and Intel, and Ubuntu 24.04 x64. GitHub Actions builds and tests each platform and publishes SHA-256 checksums.
- Added a CodeMirror 6 Live Preview for editing Markdown, rendered equations, and task lists together. The active line shows its Markdown markers. Collapsed equations retain the document's source line numbering.
- Kept Split and Source editing with syntax highlighting, synchronized scrolling in Split, and windowed loading for large documents.
- Bundled KaTeX, `mhchem`, and math fonts for offline rendering. The Split preview supports GFM tables, GitHub alerts, front matter, and a table of contents.
- Improved autosave, find and replace, full-document printing, and the prompt for unsaved changes. Added a light app icon and the About page. Windows shortcuts and file associations use the app icon.
- Added Chinese and English READMEs with screenshots of the light GitHub theme and Hokusai's *The Great Wave off Kanagawa*. The app includes seven public-domain wallpapers, custom wallpaper import, and six interface languages.

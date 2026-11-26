<div align="center">

<img src="assets/icon.svg" width="80" height="80" alt="MarkType icon">

# MarkType

A desktop Markdown and LaTeX editor for Windows, macOS, and Linux.

[简体中文](README.md) · **English** · [Releases](https://github.com/microcrystallization/marktype/releases)

</div>

![MarkType welcome screen with Hokusai's The Great Wave off Kanagawa](docs/screenshots/welcome.webp)

## Three editing views

- **Split** shows syntax-highlighted Markdown alongside an updating preview with support for synchronized scrolling, an outline, GFM tables, task lists, and GitHub alerts.
- **Live Preview** uses CodeMirror 6 to render images, GFM tables, link labels, GitHub alerts, and LaTeX in place. Moving the cursor onto an element reveals its Markdown while neighboring elements stay rendered. Fenced code highlighting covers Rust, JavaScript, TypeScript, Python, C++, Go, Bash, JSON, and YAML.
- **Source** gives you a full-width Markdown editor for working on the document structure. The line-number setting applies to all three views.

![Split view with Markdown source, rendered math and chemistry, and a GitHub alert](docs/screenshots/split.webp)

![Live Preview showing headings, syntax-highlighted code, and rendered LaTeX in the editor](docs/screenshots/live.webp)

### More screenshots

All screenshots use the light GitHub theme and The Great Wave wallpaper. The document is the [Markdown and LaTeX tour](examples/markdown-tour.md).

<details>
<summary>Source editor, appearance settings, and About</summary>

![Full-width source editor](docs/screenshots/source.webp)

![Light GitHub theme and The Great Wave wallpaper settings](docs/screenshots/appearance.webp)

![About MarkType](docs/screenshots/about.webp)

</details>

## Writing and export

- Split, Live Preview, and Source share a CodeMirror 6 editor with undo across view changes, multiple cursors, code folding, and search highlights.
- Paste a clipboard image to save it in the document's `assets/` folder and insert a relative link. An untitled document prompts for a save location first.
- Reload changes made to a file on disk or keep the editor's version. MarkType restores tabs, unsaved drafts, and window geometry when reopened.
- Right-click workspace items to create, rename, trash, or show them in the system file manager. Use `Ctrl+Shift+F` to search the workspace.
- Render Mermaid diagrams offline and choose whether HTML exports embed local images. Word and LaTeX export are available when Pandoc is installed.
- Bundled KaTeX, `mhchem`, and math fonts render equations and chemical notation offline.
- Tabs, a file tree, find and replace, autosave, HTML export, and printing to PDF through the system dialog.
- Large files load in source and preview windows. Live Preview draws content near the viewport and sends edit ranges to Rust's Rope text buffer.
- Seven public-domain artworks, custom wallpapers, and light and dark themes.

## Compile a LaTeX paper

1. Install TeX Live, MacTeX, or MiKTeX with `latexmk`, and put your TeX tools on the application’s `PATH`. Alternatively, install Tectonic. On Windows, `latexmk` may require Perl.
2. Open a `.tex` file or its project folder. MarkType reads `% !TeX root = ../main.tex` in a chapter. You can also browse for the main file in the compile dialog.
3. Press `Ctrl+Enter` (`⌘↵` on macOS), select an engine, and choose **Save and compile**. View or copy the build log in the dialog, and open the generated PDF next to the main file when the build succeeds.

The dialog supports pdfLaTeX, XeLaTeX, LuaLaTeX, and Tectonic. Automatic mode follows the main file’s `% !TeX program` directive, then tries `latexmk` with pdfLaTeX, then Tectonic. `latexmk` reruns TeX and invokes BibTeX or Biber as required by the project. Your TeX distribution supplies the packages and fonts. Tectonic may download packages on first use. The compiler resolves inputs and figures from the project directory and writes intermediate files to a temporary directory. Shell escape and project `latexmkrc` files are disabled.

Try the [example paper](examples/latex-paper/main.tex) for sections, cross-references, graphics, and BibTeX. Copy the full `latex-paper` folder to a writable location if it came from the system installation directory.

| Action | Shortcut |
| --- | --- |
| Split / Live Preview / Source | `Ctrl+\` / `Ctrl+/` / `Ctrl+Shift+/` |
| New / Open / Save | `Ctrl+N` / `Ctrl+O` / `Ctrl+S` |
| Find / Replace | `Ctrl+F` / `Ctrl+H` |
| Settings / Print | `Ctrl+,` / `Ctrl+P` |

Use `⌘` on macOS.

## Install

Download the Windows, macOS, and Ubuntu packages and their SHA-256 checksums from the [latest release](https://github.com/microcrystallization/marktype/releases/latest).

| Platform | Installation |
| --- | --- |
| Windows 10/11 x64 | Run `MarkType-Setup-<version>-x64.exe` or extract the portable ZIP. Microsoft Edge WebView2 Runtime is required. |
| macOS 13.3+ | Choose the `macos-arm64.dmg` or `macos-x64.dmg` for your Mac, then drag MarkType into Applications. The app is ad-hoc signed and has no Apple notarization. |
| Ubuntu 24.04 x64 | Run `sudo apt install ./MarkType-<version>-linux-amd64.deb`. |

## Build and release

Install the stable Rust toolchain. On Ubuntu 24.04, install `libgtk-3-dev libwebkit2gtk-4.1-dev`. On macOS, install the Xcode command line tools. Windows packaging requires Inno Setup.

```sh
cargo test --locked --release
cargo run --locked --release -- examples/markdown-tour.md
```

| Package | Command |
| --- | --- |
| Windows installer and portable ZIP | `.\scripts\build-installer.ps1` |
| macOS DMG | `bash scripts/build-macos.sh` |
| Linux DEB | `bash scripts/build-linux.sh` |

[CI](https://github.com/microcrystallization/marktype/actions/workflows/ci.yml) runs checks, tests, and desktop smoke tests. Run [Desktop packages](https://github.com/microcrystallization/marktype/actions/workflows/release.yml) manually for build artifacts. Pushing a new `v*` tag that matches the version in `Cargo.toml` creates a GitHub Release with packages and a SHA-256 checksum file.

The application embeds CodeMirror, KaTeX, and Mermaid. Use Node.js 24 to run `npm ci`, then `npm run build:live` and `npm run build:diagrams` to update the offline bundles.

On a Mac configured with an Apple Developer ID and `notarytool` credentials, pass `MARKTYPE_MACOS_IDENTITY` and `MARKTYPE_NOTARY_PROFILE` to `scripts/build-macos.sh` to sign, notarize, and staple the DMG.

## License

[MIT](LICENSE). See [NOTICE.md](NOTICE.md) for attributions for fonts, math rendering, the editor, and wallpapers.

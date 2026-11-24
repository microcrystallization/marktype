# Changelog

All notable changes to **MarkType** will be documented in this file.

## [0.1.1] - 2026-11-24

- Introduced a light MarkType icon and a richer About page with the installed version, feature details, and project links.
- Fixed missing Windows icons in the installed app, desktop and Start Menu shortcuts, and file associations.
- Improved syntax highlighting, synchronized scrolling, and typography in the editor and preview.
- Added an in-app save prompt and corrected autosave, search and replace, and full-document printing.

## [0.1.0] - 2026-11-23

### Added
- **Incremental Rust Markdown + LaTeX Engine**: `ropey::Rope` buffer with O(dirty block) `pulldown-cmark` incremental parsing (`< 0.2 ms` per keystroke on 50,000-line files; `~8 ms` large-file indexing).
- **Three Seamless View Modes**:
  - Real-time `<10ms` Split Dual-Pane view (`Ctrl+\`) with bidirectional synchronized scrolling.
  - Typora-style single-pane Live WYSIWYG mode (`Ctrl+/`) with in-place block editing and live math preview.
  - Syntax-highlighted Source Code mode (`Ctrl+Shift+/`).
- **Offline KaTeX + `mhchem` Math & Chemistry**: Inline `$...$`, display `$$...$$`, `\(...\)`, `\[...\]`, matrices, cases, and chemical equations bundled offline.
- **Full Typora Feature Set**: Outline & File Tree sidebar, multi-tab documents, Focus Mode (`F8`), Typewriter Mode (`F9`), Regex Find & Replace (`Ctrl+F` / `Ctrl+H`), interactive clickable task checkboxes, YAML Front Matter cards, `[TOC]`, Callouts, tables, Rust syntax highlighter, HTML export, and Print/PDF (`Ctrl+P`).
- **Kite-Inspired Wallpaper & Adaptive Theme System**:
  - 7 embedded public-domain classical artworks from The Met Open Access + custom PNG/JPEG/WebP import & history.
  - 5 cached pixel effects (`None`, `Scanlines`, `ASCII`, `Halftone`, `Dither`).
  - Frosted glass full-window backdrop with WCAG contrast-guarded automatic panel opacity (`0.55–0.92`) and dominant-colour adaptive theme tinting.
  - 180 ms cubic-bezier wallpaper crossfade & full-bleed sharp Hero Welcome View.
- **Seamless Frameless Native Window**: Custom integrated titlebar with DWM rounded corners, drop shadow, drag-to-move, double-click maximize, and 8-direction edge resizing.
- **Multi-Language i18n**: Live switching across 6 languages (`简体中文`, `繁體中文`, `English`, `日本語`, `한국어`, `Français`).

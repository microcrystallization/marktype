---
title: Getting Started with MarkType
engine: Rope + Incremental Block Diff (<10ms)
theme: Kite Adaptive Glass
---

# MarkType — Markdown & LaTeX Studio

> [!TIP] Sub-10ms Real-Time Dual-Pane Editing
> Type anywhere in the left source editor and watch the right preview update character-by-character with zero debounce.

[TOC]

## 1. LaTeX Mathematics & Chemistry

Inline formulas: Euler's identity $e^{i\pi} + 1 = 0$, Gaussian integral $\int_{-\infty}^{\infty} e^{-x^2}\,dx = \sqrt{\pi}$, and chemical equilibrium $\ce{CO2 + H2O <=> H2CO3}$.

$$
\mathcal{L}(\theta) = -\frac{1}{N}\sum_{i=1}^{N} \left[ y_i \log \hat{y}_i + (1 - y_i)\log(1 - \hat{y}_i) \right] + \frac{\lambda}{2}\|\theta\|_2^2
$$

$$
\begin{pmatrix}
\cos\theta & -\sin\theta & 0 \\
\sin\theta & \cos\theta & 0 \\
0 & 0 & 1
\end{pmatrix}
\begin{pmatrix} x \\ y \\ z \end{pmatrix}
=
\begin{cases}
x' = x\cos\theta - y\sin\theta \\
y' = x\sin\theta + y\cos\theta \\
z' = z
\end{cases}
$$

## 2. Typora-Compatible Formatting

Supports **bold**, *italic*, ***bold italic***, ~~strikethrough~~, ==highlight==, superscript E=mc^2^, subscript H~2~O, keyboard <kbd>Ctrl</kbd> + <kbd>\\</kbd>, and inline code `ropey::Rope`.

- [x] Real-time Split View (`Ctrl+\`)
- [x] Typora Live WYSIWYG Mode (`Ctrl+/`)
- [x] 7 curated Met Open Access wallpapers + 5 pixel effects (`Ctrl+,`)
- [ ] Click this checkbox to toggle the source directly

| Feature | MarkType |
| :--- | :--- |
| Document Buffer | `ropey::Rope` ($O(\log N)$ edits) |
| Markdown Engine | Incremental `pulldown-cmark` block diff |
| Math Engine | Bundled offline KaTeX + `mhchem` |
| Languages | 中文 / 繁體 / English / 日本語 / 한국어 / Français |

```rust
pub fn pi_series(terms: usize) -> f64 {
    let sum: f64 = (1..=terms).map(|k| 1.0 / (k as f64).powi(2)).sum();
    (sum * 6.0).sqrt()
}
```

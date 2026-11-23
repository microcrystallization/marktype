# LaTeX Math & Markdown Test Fixture

Inline math: Euler $e^{i\pi} + 1 = 0$, Gaussian $\int_{-\infty}^{\infty} e^{-x^2}\,dx = \sqrt{\pi}$, and chemistry $\ce{2H2 + O2 ->[\Delta] 2H2O}$.

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

| Engine Metric | MarkType (Rust) | Target |
| :--- | :---: | ---: |
| 50K-line File Open | **8 ms** | < 100 ms |
| 1-Char Incremental Patch | **0.16 ms** | < 10 ms |

- [x] Offline KaTeX `.woff2` math typography
- [ ] Clickable task list item

```rust
pub fn euler_identity() -> f64 {
    std::f64::consts::PI
}
```

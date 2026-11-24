# Third-Party Notices & Attributions

## Built-in Artworks (Wallpapers)

Migrated from [Kite](https://github.com/Kfkcome/kite). All 7 built-in paintings are public-domain works from **The Metropolitan Museum of Art Open Access** (CC0 1.0), located in `assets/wallpapers/`:

| File | Artwork | Met Object ID |
| --- | --- | --- |
| `great-wave.jpg` | Katsushika Hokusai, *Under the Wave off Kanagawa (The Great Wave)* | 45434 |
| `red-fuji.jpg` | Katsushika Hokusai, *South Wind, Clear Sky (Red Fuji)* | 36490 |
| `kanbara-snow.jpg` | Utagawa Hiroshige, *Evening Snow at Kanbara* | 56915 |
| `shono-rain.jpg` | Utagawa Hiroshige, *Sudden Shower at Shōno* | 36521 |
| `heart-of-the-andes.jpg` | Frederic Edwin Church, *Heart of the Andes* | 10481 |
| `wheat-field.jpg` | Vincent van Gogh, *Wheat Field with Cypresses* | 436535 |
| `fontainebleau-morning.jpg` | Théodore Rousseau, *An Early Summer Morning in the Forest of Fontainebleau* | 437517 |

## Wallpaper Engine & Aesthetic Inspiration

- Wallpaper artwork effects (`None`, `Scanlines`, `ASCII`, `Halftone`, `Dither`), dominant-colour extraction, WCAG contrast-guarded frosted panel opacity (`MIN_ALPHA = 0.55..=0.92`), surface tinting, and 180 ms cubic-bezier crossfade adapted from [Kite](https://github.com/Kfkcome/kite) and [zeron](https://github.com/zeronsh/zeron) (MIT).

## Typography & Math

- **Geist** & **Geist Mono** fonts by Vercel — SIL Open Font License 1.1 (`assets/fonts/`).
- **KaTeX** v0.16.21 & `mhchem` chemistry extension — MIT License (`assets/katex/`).

## Live Editor

- **CodeMirror 6** and **Lezer** power inline editing and Markdown parsing. The bundled editor includes the license texts for CodeMirror, Lezer, and their dependencies (`ui/vendor/codemirror.js`).

## Windows Installer Translations

- The Simplified and Traditional Chinese translation files are from [Inno Setup 6.7.1](https://github.com/jrsoftware/issrc/tree/cfdf48923178df4b4f040e038b423aa555a61ffc/Files/Languages/Unofficial). The files retain their original attribution and are distributed with the [Inno Setup License](packaging/windows/languages/LICENSE-InnoSetup.txt).

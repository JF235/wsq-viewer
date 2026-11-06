# WSQ algorithm document

`wsq.tex` is a didactic description of the WSQ fingerprint image compression algorithm
(FBI IAFIS-IC-0110, version 3.1): the symmetric wavelet transform and its 64 subbands, the
quantizer of encoder number two, the Huffman coding model and the file format, with
pseudocode for every step.

## Building

Needs LuaLaTeX (TeX Live 2026 or later) with the EB Garamond, Garamond-Math and Inconsolata
fonts, all part of TeX Live.

```sh
cd docs/algorithm
latexmk          # writes build/wsq.pdf
```

## Layout

| Path | Content |
|---|---|
| `wsq.tex` | preamble and section list |
| `sections/` | one file per section |
| `figures/` | TikZ figures; `subbands.tex`, `extension.tex`, `rates.tex` and `rates-table.tex` are generated |
| `images/` | raster figures from NIST image `a002` (generated) |
| `tools/wsqref.py` | an executable reading of the specification: the algorithms of the document, in Python and double precision, without optimization |
| `tools/gen_figures.py` | regenerates the generated figures |

## Regenerating the figures

Needs Python 3 with numpy and Pillow, the NIST compliance images and the NBIS `cwsq` and
`dwsq` programs:

- NIST WSQ reference images, version 2.0 (PGM):
  <https://nigos.nist.gov/wsq/reference_images_v2.0_pgm.tar>
- NBIS 5.0.0: <https://www.nist.gov/services-resources/software/nist-biometric-image-software-nbis>
  (with gcc 10 or later, add `-fcommon` to `CFLAGS` in `rules.mak`)

```sh
python3 -I tools/gen_figures.py --nist <reference_images_v2.0_pgm> --nbis <nbis bin dir>
```

`tools/wsqref.py` is documentation, not part of the extension, which has its own codec.

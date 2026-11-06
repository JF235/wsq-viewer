# WSQ performance document

`performance.tex` reports how the WSQ codec of this repository was made fast: the separation
between algorithms (generic kernels) and backends (scalar, SSE2, AVX2, AVX-512, WebAssembly
SIMD), how bit exactness with NBIS was kept, each optimization with its measured gain, the
comparison against NBIS 5.0.0, whether relaxing exactness pays, and a test of batch decoding
on a GPU.

## Building

Same requirements as the algorithm document (LuaLaTeX with EB Garamond, Garamond-Math and
Inconsolata), plus pgfplots.

```sh
cd docs/performance
latexmk          # writes build/performance.pdf
```

## Layout

| Path | Content |
|---|---|
| `performance.tex` | preamble and section list |
| `sections/` | one file per section |
| `figures/` | TikZ figures |
| `generated/` | tables built from benchmark outputs |

## Measurements

`bench/run_all.sh` runs every measurement of the document and writes text tables to a folder;
see its header for the inputs (the `wsq` command-line tool, the NBIS harness of `bench/nbis`,
the NIST reference images and the streams NBIS writes for them, and the WebAssembly modules).
The tables in `generated/` were built from those outputs; the server table (`server.tex`)
comes from the single-core decode and encode runs at 0.75 bits per pixel on a processor with
AVX-512.

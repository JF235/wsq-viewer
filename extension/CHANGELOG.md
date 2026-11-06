# Changelog

## 1.0.0 (2026-11-06)

First release of the rewritten extension; nothing to install besides it (no Python).

- WSQ files open as images, with the gestures of VS Code's image preview: scaled to fit, click
  and Ctrl+click (Alt+click on macOS) to zoom in and out, Ctrl+wheel and pinch, pixels drawn as
  squares beyond 300%, Ctrl+C to copy, zoom and scroll kept when the image is opened again.
- Status bar: zoom (click to pick a level), dimensions, file size, and the resolution and bit rate
  of the file, with its header and NISTCOM fields in the tooltip.
- Side-by-side diffs of `.wsq` files in source control.
- Commands: Export as PNG (keeps the resolution), Show Image Information, Select Zoom Level.
- A file icon for `.wsq` files in the Explorer.
- Decoder in WebAssembly (SIMD where available): by default the pixels of the NIST reference
  decoder (NBIS 5.0.0), bit for bit; `wsqViewer.decodeMode: fast` trades that for a few percent
  of speed within the tolerances of the specification.
- Runs in VS Code for the desktop, remote workspaces and vscode.dev, also in restricted mode.

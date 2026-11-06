# WSQ Viewer

Open WSQ fingerprint images (FBI IAFIS-IC-0110 v3.1) in Visual Studio Code the way PNG and JPEG
files open: click a `.wsq` file and the image appears.

## Features

- **Like the built-in image preview**: the image opens scaled to fit; click to zoom in,
  Ctrl+click (Alt+click on macOS) to zoom out, Ctrl+wheel or pinch to zoom smoothly; pixels are
  drawn as squares beyond 300%; zoom and scroll position survive switching tabs; Ctrl+C copies
  the image.
- **Status bar**: dimensions, zoom (click to pick a level), file size, and the WSQ resolution and
  bit rate, with the full header in the tooltip.
- **Diffs**: a changed `.wsq` file in source control shows the old and new images side by side.
- **Export as PNG**: from the editor's context menu, the Explorer or the Command Palette; the
  resolution is kept in the PNG file.
- **Image information**: dimensions, resolution, bit rate, compression ratio, wavelet filters
  and every NISTCOM field (`WSQ: Show Image Information`).
- **File icon** in the Explorer, in the style of the image icons of VS Code's default icon theme.
- **Reloads** when the file changes on disk.
- **Works everywhere VS Code does**: desktop, remote and vscode.dev, in restricted mode too.

## Decoder

The decoder is a Rust implementation of the WSQ specification compiled to WebAssembly, shipped
inside the extension: nothing else to install. It decodes a 1452 x 1006 image in a few
milliseconds, and images never leave your machine. Two modes (setting `wsqViewer.decodeMode`):

- `exact` (default): the pixels of the NIST reference decoder (NBIS 5.0.0), bit for bit, as
  checked on the NIST compliance data set;
- `fast`: optimized arithmetic, a few percent faster, within the tolerances of the specification
  (at least 99.9% of the pixels equal to the reference decoder, none off by more than one grey
  level).

Source, algorithm description and measurements: <https://github.com/JF235/wsq-viewer>.

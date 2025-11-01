# wsq-viewer

A small Visual Studio Code extension that provides a custom editor for WSQ fingerprint image files (`*.wsq`).

## What it does
- Registers a custom editor (`wsq.viewer`) for `*.wsq` files so you can preview WSQ images inside VS Code.
- Uses a Python decoder (configurable) to convert WSQ to a displayable format.

## Requirements
- Python 3 (or a Python interpreter that can run the decoder script). Configure the path with the setting `wsq.viewer.pythonPath` if necessary.
- The extension bundles a small Python decoder wrapper (`decoder.py`). If you prefer a different decoder, update the extension configuration or replace the script.

## Usage
1. Open a `*.wsq` file in VS Code.
2. The custom WSQ viewer should open automatically. If not, right-click the file and choose `Open With...` -> `WSQ Image Viewer`.

## Configuration
- `wsq.viewer.pythonPath` (string | null): absolute path to the Python executable to use for decoding. If null, the extension will try `python3` then `python`.

## License & Credits
- This project is provided under the MIT license (see `package.json`).
- Thanks to the authors of the `idemia/python-wsq` NBIS wrapper for providing the WSQ decoding tools used by this extension.

## Troubleshooting
- If images fail to open, verify Python is installed and the `wsq.viewer.pythonPath` setting points to a working interpreter that can run the bundled `decoder.py`.
- If you change the decoder script, rebuild the extension and repackage before installing the VSIX.

## Building & Packaging
From the extension folder:
```bash
npm install
npm run build   # produces bundled files in dist/
npm run vsix    # creates a .vsix package (set a valid publisher in package.json first)
```

If you want to publish to the Visual Studio Marketplace, set the `publisher` field in `package.json`, obtain a VSCE PAT, and run `npm run publish:vsce`.

---

Thank you for using wsq-viewer.

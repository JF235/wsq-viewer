// @ts-check
// Webview of the WSQ preview. It behaves like VS Code's built-in image preview: the image starts
// scaled to fit; a click zooms in through fixed levels and Ctrl+click (Alt+click on macOS) zooms
// out; Ctrl+wheel and pinch zoom continuously; past 300% the pixels are drawn as squares; Ctrl+C
// copies the image; zoom and scroll position survive the editor being hidden and shown again.
// The extension host decodes the file and sends grey pixels, painted here on a canvas.
'use strict';

(function () {
  /** Draw pixels as squares beyond this scale. */
  const PIXELATION_THRESHOLD = 3;
  const SCALE_PINCH_FACTOR = 0.075;
  const MAX_SCALE = 20;
  const MIN_SCALE = 0.1;
  const ZOOM_LEVELS = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1, 1.5, 2, 3, 5, 7, 10, 15, 20];

  // @ts-ignore provided by the webview host
  const vscode = acquireVsCodeApi();
  const isMac = /Mac/.test(navigator.userAgent);

  /** @returns {{ viewState: { scale: number, offsetX: number, offsetY: number } | null }} */
  function getSettings() {
    const data = document.getElementById('wsq-preview-settings')?.getAttribute('data-settings');
    return data ? JSON.parse(data) : { viewState: null };
  }

  // The webview state survives hiding the editor; the extension's copy survives closing it.
  /** @type {{ scale: number | 'fit', offsetX: number, offsetY: number }} */
  const initialState = vscode.getState() || getSettings().viewState || { scale: 'fit', offsetX: 0, offsetY: 0 };

  /** @type {number | 'fit'} */
  let scale = initialState.scale;
  let ctrlPressed = false;
  let altPressed = false;
  let hasImage = false;
  let consumeClick = true;
  let isActive = false;

  const body = document.body;
  const canvas = document.createElement('canvas');
  canvas.classList.add('scale-to-fit');
  canvas.setAttribute('role', 'img');

  /** @param {number} v @param {number} lo @param {number} hi */
  const clamp = (v, lo, hi) => Math.min(Math.max(v, lo), hi);

  const zoomOutModifier = () => (isMac ? altPressed : ctrlPressed);

  /** @type {ReturnType<typeof setTimeout> | undefined} */
  let saveTimer;

  /**
   * Keep the view state in the webview and, a moment later, in the extension.
   * @param {{ scale: number, offsetX: number, offsetY: number } | undefined} state
   */
  function saveState(state) {
    vscode.setState(state);
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => vscode.postMessage({ type: 'viewState', state }), 200);
  }

  /** @param {number | 'fit'} newScale */
  function updateScale(newScale) {
    if (!hasImage) {
      return;
    }
    if (newScale === 'fit') {
      scale = 'fit';
      canvas.classList.add('scale-to-fit');
      canvas.classList.remove('pixelated');
      // @ts-ignore non-standard but supported by every engine VS Code runs on
      canvas.style.zoom = 'normal';
      saveState(undefined);
    } else {
      scale = clamp(newScale, MIN_SCALE, MAX_SCALE);
      canvas.classList.toggle('pixelated', scale >= PIXELATION_THRESHOLD);
      // keep the point at the centre of the view in place
      const dx = (window.scrollX + body.clientWidth / 2) / body.scrollWidth;
      const dy = (window.scrollY + body.clientHeight / 2) / body.scrollHeight;
      canvas.classList.remove('scale-to-fit');
      // @ts-ignore
      canvas.style.zoom = String(scale);
      const x = body.scrollWidth * dx - body.clientWidth / 2;
      const y = body.scrollHeight * dy - body.clientHeight / 2;
      window.scrollTo(x, y);
      saveState({ scale, offsetX: x, offsetY: y });
    }
    vscode.postMessage({ type: 'zoom', value: scale });
  }

  /** @param {boolean} value */
  function setActive(value) {
    isActive = value;
    if (!value) {
      ctrlPressed = false;
      altPressed = false;
    }
    updateCursor();
  }

  function updateCursor() {
    body.classList.toggle('zoom-in', isActive && !zoomOutModifier());
    body.classList.toggle('zoom-out', isActive && zoomOutModifier());
  }

  /** Leave "fit" for the numeric scale the image is currently shown at. */
  function firstZoom() {
    if (hasImage && scale === 'fit') {
      scale = canvas.clientWidth / canvas.width;
      updateScale(scale);
    }
  }

  function zoomIn() {
    firstZoom();
    const s = /** @type {number} */ (scale);
    updateScale(ZOOM_LEVELS.find(level => level > s) ?? MAX_SCALE);
  }

  function zoomOut() {
    firstZoom();
    const s = /** @type {number} */ (scale);
    updateScale([...ZOOM_LEVELS].reverse().find(level => level < s) ?? MIN_SCALE);
  }

  /**
   * Paint grey pixels on the canvas.
   * @param {number} width @param {number} height @param {Uint8Array} pixels
   */
  function showImage(width, height, pixels) {
    canvas.width = width;
    canvas.height = height;
    const ctx = /** @type {CanvasRenderingContext2D} */ (canvas.getContext('2d'));
    const data = ctx.createImageData(width, height);
    const rgba = new Uint32Array(data.data.buffer);
    for (let i = 0; i < pixels.length; i++) {
      rgba[i] = 0xff000000 | (pixels[i] * 0x010101); // little-endian RGBA
    }
    ctx.putImageData(data, 0, 0);
    canvas.setAttribute('aria-label', `WSQ image, ${width} by ${height} pixels`);

    const first = !hasImage;
    hasImage = true;
    body.classList.remove('loading', 'error', 'lfs');
    body.classList.add('ready');
    if (first) {
      body.append(canvas);
    }
    updateScale(scale);
    if (first && initialState.scale !== 'fit') {
      window.scrollTo(initialState.offsetX, initialState.offsetY);
    }
    vscode.postMessage({ type: 'rendered', width, height });
  }

  /** @param {string} kind @param {string} [detail] */
  function showMessage(kind, detail) {
    body.classList.remove('loading', 'ready');
    body.classList.add(kind);
    const p = document.querySelector('.error-message .detail');
    if (p) {
      p.textContent = detail || '';
    }
  }

  /** Pixels arrive as a Uint8Array; accept the other shapes a message may take. */
  /** @param {unknown} v @returns {Uint8Array} */
  function bytes(v) {
    if (v instanceof Uint8Array) {
      return v;
    }
    if (v instanceof ArrayBuffer) {
      return new Uint8Array(v);
    }
    if (ArrayBuffer.isView(v)) {
      return new Uint8Array(v.buffer, v.byteOffset, v.byteLength);
    }
    return Uint8Array.from(Object.values(/** @type {object} */ (v)));
  }

  async function copyImage(retries = 5) {
    if (!hasImage) {
      return;
    }
    if (!document.hasFocus() && retries > 0) {
      // the command reveals the webview first; the clipboard needs the focus it is gaining
      setTimeout(() => copyImage(retries - 1), 20);
      return;
    }
    try {
      await navigator.clipboard.write([
        new ClipboardItem({
          'image/png': new Promise((resolve, reject) => canvas.toBlob(b => (b ? resolve(b) : reject(new Error('toBlob failed'))), 'image/png')),
        }),
      ]);
    } catch (e) {
      console.error(e);
    }
  }

  window.addEventListener('keydown', e => {
    ctrlPressed = e.ctrlKey;
    altPressed = e.altKey;
    updateCursor();
  });

  window.addEventListener('keyup', e => {
    ctrlPressed = e.ctrlKey;
    altPressed = e.altKey;
    updateCursor();
  });

  body.addEventListener('mousedown', e => {
    if (!hasImage || e.button !== 0) {
      return;
    }
    ctrlPressed = e.ctrlKey;
    altPressed = e.altKey;
    // the click that focuses an inactive editor does not zoom
    consumeClick = !isActive;
  });

  body.addEventListener('click', e => {
    if (!hasImage || e.button !== 0) {
      return;
    }
    if (consumeClick) {
      consumeClick = false;
      return;
    }
    if (zoomOutModifier()) {
      zoomOut();
    } else {
      zoomIn();
    }
  });

  body.addEventListener('wheel', e => {
    if (e.ctrlKey) {
      e.preventDefault(); // no page zoom on pinch
    }
    if (!hasImage) {
      return;
    }
    // pinching arrives as a wheel event with ctrlKey set
    if (!zoomOutModifier() && !e.ctrlKey) {
      return;
    }
    firstZoom();
    const delta = e.deltaY > 0 ? 1 : -1;
    updateScale(/** @type {number} */ (scale) * (1 - delta * SCALE_PINCH_FACTOR));
  }, { passive: false });

  window.addEventListener('scroll', () => {
    if (!hasImage || scale === 'fit') {
      return;
    }
    const state = vscode.getState();
    if (state) {
      saveState({ scale: state.scale, offsetX: window.scrollX, offsetY: window.scrollY });
    }
  }, { passive: true });

  document.addEventListener('copy', () => copyImage());

  for (const link of document.querySelectorAll('.open-file-link')) {
    link.addEventListener('click', e => {
      e.preventDefault();
      vscode.postMessage({ type: 'reopen-as-text' });
    });
  }

  window.addEventListener('message', e => {
    if (e.origin !== window.origin) {
      return;
    }
    const m = e.data;
    switch (m.type) {
      case 'image': showImage(m.width, m.height, bytes(m.pixels)); break;
      case 'error': showMessage('error', m.message); break;
      case 'lfs': showMessage('lfs'); break;
      case 'setScale': updateScale(m.scale); break;
      case 'setActive': setActive(m.value); break;
      case 'zoomIn': zoomIn(); break;
      case 'zoomOut': zoomOut(); break;
      case 'copyImage': copyImage(); break;
    }
  });

  vscode.postMessage({ type: 'ready' });
}());

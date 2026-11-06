/** A synthetic fingerprint-like pattern: curved ridges with a slow change of contrast. */
export function ridges(width: number, height: number): Uint8Array {
  const px = new Uint8Array(width * height);
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const r = Math.hypot(x - width * 0.45, y - height * 0.6);
      const contrast = 60 + 50 * Math.sin(y / height * Math.PI);
      px[y * width + x] = Math.round(140 + contrast * Math.sin(r * 0.55));
    }
  }
  return px;
}

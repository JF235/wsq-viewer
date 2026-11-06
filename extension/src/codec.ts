// Binding of the WebAssembly build of the codec (crates/wsq-wasm). It uses no VS Code or Node
// API, so the same code runs in the desktop extension host, in the web worker of vscode.dev and
// in the unit tests.

export interface WsqImage {
  width: number;
  height: number;
  /** Resolution from the NISTCOM comment, if the stream has one. */
  ppi: number | undefined;
  /** Row-major grey pixels, width * height bytes. */
  pixels: Uint8Array;
}

export interface WsqInfo {
  width: number;
  height: number;
  ppi: number | null;
  /** Target bit rate from the NISTCOM comment. */
  bitrate: number | null;
  /** Encoder number of the frame header (2 for encoder number two). */
  encoder: number;
  /** Software implementation number of the frame header. */
  software: number;
  /** Lengths of the lowpass and highpass analysis filters. */
  filters: [number, number];
  /** Number of entropy-coded blocks. */
  blocks: number;
  comments: string[];
}

export interface EncodeOptions {
  /** Bits per pixel; 0.75 is the usual rate for 500 ppi fingerprints. */
  bitrate?: number;
  ppi?: number;
  /** Fast mode: any operation order, within the compliance tolerances. */
  fast?: boolean;
}

interface Exports {
  memory: WebAssembly.Memory;
  wsq_alloc(len: number): number;
  wsq_free(ptr: number, len: number): void;
  wsq_decode(ptr: number, len: number, fast: number): number;
  wsq_width(): number;
  wsq_height(): number;
  wsq_ppi(): number;
  wsq_pixels(): number;
  wsq_info(ptr: number, len: number): number;
  wsq_encode(ptr: number, width: number, height: number, bitrate: number, ppi: number, fast: number): number;
  wsq_result_ptr(): number;
  wsq_result_len(): number;
  wsq_error_ptr(): number;
  wsq_error_len(): number;
  wsq_lanes(): number;
}

export class WsqError extends Error {
  override name = 'WsqError';
}

export class WsqCodec {
  private readonly w: Exports;

  private constructor(w: Exports) {
    this.w = w;
  }

  /** Instantiate the SIMD module when the engine supports it, the scalar one otherwise. */
  static async load(scalar: Uint8Array, simd?: Uint8Array): Promise<WsqCodec> {
    const usable = (b: Uint8Array) => b as Uint8Array<ArrayBuffer>;
    const bytes = simd && WebAssembly.validate(usable(simd)) ? usable(simd) : usable(scalar);
    const { instance } = await WebAssembly.instantiate(bytes, {});
    return new WsqCodec(instance.exports as unknown as Exports);
  }

  /** 4 when the SIMD module runs, 1 for the scalar one. */
  get lanes(): number {
    return this.w.wsq_lanes();
  }

  /** Decode a WSQ stream; the exact mode returns the pixels of NBIS, bit for bit. */
  decode(data: Uint8Array, fast = false): WsqImage {
    const w = this.w;
    this.call(data, ptr => w.wsq_decode(ptr, data.length, fast ? 1 : 0));
    const width = w.wsq_width();
    const height = w.wsq_height();
    const pixels = new Uint8Array(w.memory.buffer, w.wsq_pixels(), width * height).slice();
    return { width, height, ppi: w.wsq_ppi() || undefined, pixels };
  }

  /** Read the header, tables and comments without decoding the image. */
  info(data: Uint8Array): WsqInfo {
    this.call(data, ptr => this.w.wsq_info(ptr, data.length));
    return JSON.parse(new TextDecoder().decode(this.result())) as WsqInfo;
  }

  /** Encode grey pixels; the exact mode writes the bytes of the NBIS encoder. */
  encode(pixels: Uint8Array, width: number, height: number, opts: EncodeOptions = {}): Uint8Array {
    if (pixels.length !== width * height) {
      throw new WsqError(`expected ${width * height} pixels, got ${pixels.length}`);
    }
    const w = this.w;
    this.call(pixels, ptr => w.wsq_encode(ptr, width, height, opts.bitrate ?? 0.75, opts.ppi ?? 0, opts.fast ? 1 : 0));
    return this.result().slice();
  }

  /** Copy `input` into the module, run `f` on it and turn a failure into a WsqError. */
  private call(input: Uint8Array, f: (ptr: number) => number): void {
    const w = this.w;
    const len = Math.max(input.length, 1);
    const ptr = w.wsq_alloc(len);
    let ok: number;
    try {
      new Uint8Array(w.memory.buffer, ptr, input.length).set(input);
      ok = f(ptr);
    } finally {
      w.wsq_free(ptr, len);
    }
    if (!ok) {
      const message = new TextDecoder().decode(new Uint8Array(w.memory.buffer, w.wsq_error_ptr(), w.wsq_error_len()));
      throw new WsqError(message || 'WSQ codec failure');
    }
  }

  /** View of the last result inside the module memory; valid until the next call. */
  private result(): Uint8Array {
    return new Uint8Array(this.w.memory.buffer, this.w.wsq_result_ptr(), this.w.wsq_result_len());
  }
}

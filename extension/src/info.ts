import type { WsqInfo } from './codec.ts';

export interface InfoField {
  label: string;
  value: string;
}

/** Key-value pairs of a NISTCOM comment ("NIST_COM 9\nPIX_WIDTH 1452\n..."), or undefined. */
export function nistcom(comment: string): [string, string][] | undefined {
  if (!comment.startsWith('NIST_COM')) {
    return undefined;
  }
  return comment
    .split('\n')
    .map(line => line.trim())
    .filter(line => line.length > 0)
    .map(line => {
      const space = line.search(/\s/);
      return space < 0 ? [line, ''] : [line.slice(0, space), line.slice(space + 1).trim()];
    });
}

/** What the information view lists for a stream of `fileSize` bytes. */
export function describe(info: WsqInfo, fileSize: number): InfoField[] {
  const pixels = info.width * info.height;
  const fields: InfoField[] = [
    { label: 'Dimensions', value: `${info.width} x ${info.height} pixels` },
    { label: 'Resolution', value: info.ppi ? `${info.ppi} ppi` : 'not recorded' },
    { label: 'Bit rate', value: `${(8 * fileSize / pixels).toFixed(3)} bits per pixel` },
    { label: 'Compression ratio', value: `${(pixels / fileSize).toFixed(1)} : 1` },
  ];
  if (info.bitrate !== null) {
    fields.push({ label: 'Target bit rate', value: `${info.bitrate} bits per pixel` });
  }
  fields.push(
    { label: 'Wavelet filters', value: `${info.filters[0]}/${info.filters[1]} taps` },
    { label: 'Encoder number', value: String(info.encoder) },
    { label: 'Software', value: String(info.software) },
    { label: 'Entropy-coded blocks', value: String(info.blocks) },
  );
  for (const c of info.comments) {
    const pairs = nistcom(c);
    if (pairs) {
      for (const [k, v] of pairs) {
        fields.push({ label: k, value: v });
      }
    } else {
      fields.push({ label: 'Comment', value: c });
    }
  }
  return fields;
}

/** Short status bar text: resolution and actual bit rate. */
export function statusText(info: WsqInfo, fileSize: number): string {
  const bpp = `${(8 * fileSize / (info.width * info.height)).toFixed(2)} bpp`;
  return info.ppi ? `${info.ppi} ppi · ${bpp}` : bpp;
}

/** The fields as a Markdown table (status bar tooltip). */
export function markdownTable(fields: InfoField[]): string {
  const esc = (s: string) => s.replace(/[\\|*_`[\]<>]/g, m => `\\${m}`);
  return ['| | |', '|---|---|', ...fields.map(f => `| ${esc(f.label)} | ${esc(f.value)} |`)].join('\n');
}

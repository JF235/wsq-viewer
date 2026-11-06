import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { WsqInfo } from '../../src/codec.ts';
import { describe, markdownTable, nistcom, statusText } from '../../src/info.ts';

const info: WsqInfo = {
  width: 1000,
  height: 800,
  ppi: 500,
  bitrate: 0.75,
  encoder: 2,
  software: 0,
  filters: [9, 7],
  blocks: 3,
  comments: ['NIST_COM 9\nPIX_WIDTH 1000\nPIX_HEIGHT 800\nPPI 500\nLOSSY 1\nCOLORSPACE GRAY\nCOMPRESSION WSQ\nWSQ_BITRATE 0.750000', 'scanned by | test'],
};

test('NISTCOM comments become key-value pairs', () => {
  assert.deepEqual(nistcom('NIST_COM 2\nPPI 500\nEMPTY\n'), [['NIST_COM', '2'], ['PPI', '500'], ['EMPTY', '']]);
  assert.equal(nistcom('a plain comment'), undefined);
});

test('the information lists measured and recorded properties', () => {
  const fields = Object.fromEntries(describe(info, 75000).map(f => [f.label, f.value]));
  assert.equal(fields['Dimensions'], '1000 x 800 pixels');
  assert.equal(fields['Resolution'], '500 ppi');
  assert.equal(fields['Bit rate'], '0.750 bits per pixel');
  assert.equal(fields['Compression ratio'], '10.7 : 1');
  assert.equal(fields['Wavelet filters'], '9/7 taps');
  assert.equal(fields['PIX_WIDTH'], '1000');
  assert.equal(fields['Comment'], 'scanned by | test');
});

test('the status text shows resolution and actual bit rate', () => {
  assert.equal(statusText(info, 75000), '500 ppi · 0.75 bpp');
  assert.equal(statusText({ ...info, ppi: null }, 75000), '0.75 bpp');
});

test('Markdown special characters are escaped in the tooltip', () => {
  const md = markdownTable([{ label: 'Comment', value: 'a|b*c' }]);
  assert.ok(md.includes('| Comment | a\\|b\\*c |'), md);
});

// Integration tests, run inside the extension host of VS Code for the web (see run.mjs).
import * as vscode from 'vscode';
import type { TestApi } from '../../src/extension.ts';
import type { PreviewSnapshot } from '../../src/preview.ts';

const VIEW_TYPE = 'wsqViewer.preview';

function assert(cond: unknown, message: string): asserts cond {
  if (!cond) {
    throw new Error(message);
  }
}

async function until<T>(what: string, f: () => T | undefined | false | Promise<T | undefined | false>, timeoutMs = 15000): Promise<T> {
  const start = Date.now();
  for (;;) {
    const v = await f();
    if (v) {
      return v;
    }
    if (Date.now() - start > timeoutMs) {
      throw new Error(`timed out waiting for ${what}`);
    }
    await new Promise(r => setTimeout(r, 50));
  }
}

const folder = () => vscode.workspace.workspaceFolders![0].uri;
const file = (name: string) => vscode.Uri.joinPath(folder(), name);

async function api(): Promise<TestApi> {
  const ext = vscode.extensions.getExtension<TestApi>('jf235.wsq-viewer');
  assert(ext, 'extension not found');
  const exports = await ext.activate();
  assert(exports, 'the extension did not return its test API (not in test mode?)');
  return exports;
}

function activeTabInput(): unknown {
  return vscode.window.tabGroups.activeTabGroup.activeTab?.input;
}

async function preview(uri: vscode.Uri, f: (s: PreviewSnapshot) => boolean): Promise<PreviewSnapshot> {
  const a = await api();
  return until(`a preview of ${uri.path} that satisfies ${f}`, () => a.previews().find(s => s.uri === uri.toString() && f(s)));
}

async function openRendered(name: string): Promise<PreviewSnapshot> {
  const uri = file(name);
  await vscode.commands.executeCommand('vscode.open', uri);
  return preview(uri, s => s.rendered);
}

const observations: string[] = [];

function describeInput(input: unknown): string {
  if (input instanceof vscode.TabInputCustom) {
    return `custom editor ${input.viewType}`;
  }
  if (input instanceof vscode.TabInputTextDiff) {
    return 'text diff editor';
  }
  if (input instanceof vscode.TabInputText) {
    return 'text editor';
  }
  return input === undefined ? 'an editor the tabs API does not describe' : String(input);
}

const tests: [string, () => Promise<void>][] = [
  ['a .wsq file opens in the WSQ preview and is decoded and painted', async () => {
    const s = await openRendered('ridges.wsq');
    assert(s.width === 417 && s.height === 289, `size ${s.width}x${s.height}`);
    const input = activeTabInput();
    assert(input instanceof vscode.TabInputCustom && input.viewType === VIEW_TYPE, `active tab is ${describeInput(input)}`);
  }],

  ['exact mode is the default and the setting switches to fast', async () => {
    const uri = file('ridges.wsq');
    await openRendered('ridges.wsq');
    await preview(uri, s => s.mode === 'exact');
    const config = vscode.workspace.getConfiguration('wsqViewer');
    await config.update('decodeMode', 'fast', vscode.ConfigurationTarget.Global);
    try {
      await preview(uri, s => s.mode === 'fast' && s.rendered);
    } finally {
      await config.update('decodeMode', undefined, vscode.ConfigurationTarget.Global);
    }
    await preview(uri, s => s.mode === 'exact');
  }],

  ['.wsq files get the WSQ language and its icon', async () => {
    const languages = await vscode.languages.getLanguages();
    assert(languages.includes('wsq'), 'no wsq language registered');
    const ext = vscode.extensions.getExtension('jf235.wsq-viewer')!;
    const icon = ext.packageJSON.contributes.languages[0].icon;
    for (const theme of ['light', 'dark']) {
      const svg = new TextDecoder().decode(await vscode.workspace.fs.readFile(vscode.Uri.joinPath(ext.extensionUri, icon[theme])));
      assert(svg.startsWith('<svg') && svg.includes('clip-path'), `${theme} file icon is not the expected SVG`);
    }
  }],

  ['the file name match ignores case', async () => {
    const s = await openRendered('UPPER.WSQ');
    assert(s.width === 64 && s.height === 48, `size ${s.width}x${s.height}`);
  }],

  ['the web build runs the SIMD module', async () => {
    const lanes = await (await api()).lanes();
    assert(lanes === 4, `lanes ${lanes}`);
  }],

  ['zoom commands move between fixed levels', async () => {
    const uri = file('ridges.wsq');
    await openRendered('ridges.wsq');
    await vscode.commands.executeCommand('wsqViewer.zoomIn');
    const zoomed = await preview(uri, s => typeof s.scale === 'number');
    await vscode.commands.executeCommand('wsqViewer.zoomIn');
    const more = await preview(uri, s => typeof s.scale === 'number' && s.scale > (zoomed.scale as number));
    await vscode.commands.executeCommand('wsqViewer.zoomOut');
    await preview(uri, s => typeof s.scale === 'number' && s.scale < (more.scale as number));
  }],

  ['zoom and scroll come back when the image is opened again', async () => {
    const uri = file('other.wsq');
    await openRendered('other.wsq');
    await vscode.commands.executeCommand('wsqViewer.zoomIn');
    await vscode.commands.executeCommand('wsqViewer.zoomIn');
    const zoomed = await preview(uri, s => typeof s.scale === 'number' && s.scale > 1);
    await new Promise(r => setTimeout(r, 500)); // the webview reports its view state after 200 ms
    await vscode.commands.executeCommand('workbench.action.closeAllEditors');
    await until('the preview to close', async () => !(await api()).previews().some(s => s.uri === uri.toString()));
    await vscode.commands.executeCommand('vscode.open', uri);
    await preview(uri, s => s.rendered && s.scale === zoomed.scale);
  }],

  ['Export as PNG writes the image with its resolution', async () => {
    const dest = file('exported.png');
    const written = await vscode.commands.executeCommand<vscode.Uri>('wsqViewer.exportPng', file('ridges.wsq'), dest);
    assert(written?.toString() === dest.toString(), 'the command did not return the destination');
    const png = await vscode.workspace.fs.readFile(dest);
    assert(png[0] === 0x89 && png[1] === 0x50, 'not a PNG signature');
    const view = new DataView(png.buffer, png.byteOffset, png.byteLength);
    assert(view.getUint32(16) === 417 && view.getUint32(20) === 289, 'wrong PNG dimensions');
    assert(new TextDecoder().decode(png.subarray(37, 41)) === 'pHYs', 'no pHYs chunk after IHDR');
    assert(view.getUint32(41) === 19685, 'pHYs is not 500 ppi');
  }],

  ['a damaged file shows an error instead of an image', async () => {
    const uri = file('broken.wsq');
    await vscode.commands.executeCommand('vscode.open', uri);
    const s = await preview(uri, p => p.error !== undefined);
    assert(!s.rendered, 'a damaged file was painted');
    assert(/truncated|marker|invalid/i.test(s.error ?? ''), `unexpected error message: ${s.error}`);
  }],

  ['the preview reloads when the file changes', async () => {
    const uri = file('watched.wsq');
    await openRendered('watched.wsq');
    await vscode.workspace.fs.writeFile(uri, await vscode.workspace.fs.readFile(file('other.wsq')));
    await preview(uri, s => s.width === 300 && s.height === 200 && s.rendered);
  }],

  ['a diff of two .wsq files shows the WSQ preview on both sides', async () => {
    const a = file('ridges.wsq');
    const b = file('other.wsq');
    await vscode.commands.executeCommand('vscode.diff', a, b, 'ridges ↔ other');
    await preview(a, s => s.rendered);
    await preview(b, s => s.rendered);
    observations.push(`diff of two .wsq files: ${describeInput(activeTabInput())}, both sides decoded`);
  }],

  ['observation: how VS Code opens a PNG and a diff of two PNGs', async () => {
    await vscode.commands.executeCommand('vscode.open', file('a.png'));
    const single = await until('a tab for a.png', () => activeTabInput());
    observations.push(`a .png opens in: ${describeInput(single)}`);
    await vscode.commands.executeCommand('vscode.diff', file('a.png'), file('b.png'), 'a ↔ b');
    await new Promise(r => setTimeout(r, 1500));
    observations.push(`a diff of two .png files opens in: ${describeInput(activeTabInput())}`);
  }],
];

export async function run(): Promise<void> {
  observations.push(`VS Code ${vscode.version}`);
  const failures: string[] = [];
  for (const [name, test] of tests) {
    try {
      await test();
      console.log(`  ok    ${name}`);
    } catch (e) {
      failures.push(name);
      console.error(`  FAIL  ${name}\n        ${e instanceof Error ? e.stack ?? e.message : String(e)}`);
    } finally {
      await vscode.commands.executeCommand('workbench.action.closeAllEditors');
    }
  }
  for (const o of observations) {
    console.log(`  note  ${o}`);
  }
  console.log(`${tests.length - failures.length} passed, ${failures.length} failed`);
  if (failures.length) {
    throw new Error(`${failures.length} integration test(s) failed`);
  }
}

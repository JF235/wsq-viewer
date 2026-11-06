import * as vscode from 'vscode';
import type { WsqCodec, WsqImage, WsqInfo } from './codec.ts';
import { describe, markdownTable, statusText } from './info.ts';
import type { Scale, StatusBar } from './statusBar.ts';

export const VIEW_TYPE = 'wsqViewer.preview';

type PreviewState = 'visible' | 'active' | 'disposed';

/** Zoom and scroll position of a preview; undefined when the image is scaled to fit. */
export interface ViewState {
  scale: number;
  offsetX: number;
  offsetY: number;
}

/** Where previews keep their view state between being closed and opened again. */
export interface ViewStateStore {
  get(uri: vscode.Uri): ViewState | undefined;
  set(uri: vscode.Uri, state: ViewState | undefined): void;
}

/** Messages from the webview (media/preview.js). */
type WebviewMessage =
  | { type: 'ready' }
  | { type: 'zoom'; value: Scale }
  | { type: 'rendered'; width: number; height: number }
  | { type: 'viewState'; state: ViewState | undefined }
  | { type: 'reopen-as-text' };

/** What a test can observe of a preview. */
export interface PreviewSnapshot {
  uri: string;
  state: PreviewState;
  width?: number;
  height?: number;
  rendered: boolean;
  scale: Scale;
  mode?: 'fast' | 'exact';
  error?: string;
}

const GIT_LFS_PREFIX = 'version https://git-lfs.github.com/spec/v1';

/** Whether `uri` is decoded in fast mode rather than NBIS's exact arithmetic (the default). */
export function decodesFast(uri: vscode.Uri): boolean {
  return vscode.workspace.getConfiguration('wsqViewer', uri).get<string>('decodeMode', 'exact') === 'fast';
}

/** A Git LFS pointer shown in a diff instead of the image (as VS Code's image preview checks). */
async function isGitLfsPointer(uri: vscode.Uri): Promise<boolean> {
  if (uri.scheme !== 'git') {
    return false;
  }
  try {
    const stat = await vscode.workspace.fs.stat(uri);
    if (stat.size === 0 || stat.size > 1024) {
      return false;
    }
    return new TextDecoder().decode(await vscode.workspace.fs.readFile(uri)).startsWith(GIT_LFS_PREFIX);
  } catch {
    return false;
  }
}

function nonce(): string {
  const bytes = new Uint8Array(16);
  if (globalThis.crypto?.getRandomValues) {
    globalThis.crypto.getRandomValues(bytes);
  } else {
    for (let i = 0; i < bytes.length; i++) {
      bytes[i] = Math.floor(Math.random() * 256);
    }
  }
  return Array.from(bytes, b => b.toString(16).padStart(2, '0')).join('');
}

function errorMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** One open editor (or one side of a diff) showing a WSQ file. */
export class Preview implements vscode.Disposable {
  private state: PreviewState = 'visible';
  private scale: Scale = 'fit';
  private image: WsqImage | undefined;
  private info: WsqInfo | undefined;
  private fileSize: number | undefined;
  private mode: 'fast' | 'exact' | undefined;
  private error: string | undefined;
  private rendered = false;
  private loading: Promise<void> | undefined;
  private readonly disposables: vscode.Disposable[] = [];

  readonly resource: vscode.Uri;
  readonly panel: vscode.WebviewPanel;
  private readonly codec: Promise<WsqCodec>;
  private readonly statusBar: StatusBar;
  private readonly extensionUri: vscode.Uri;
  private readonly viewStates: ViewStateStore;

  constructor(resource: vscode.Uri, panel: vscode.WebviewPanel, codec: Promise<WsqCodec>, statusBar: StatusBar, extensionUri: vscode.Uri, viewStates: ViewStateStore) {
    this.resource = resource;
    this.panel = panel;
    this.codec = codec;
    this.statusBar = statusBar;
    this.extensionUri = extensionUri;
    this.viewStates = viewStates;

    panel.webview.options = {
      enableScripts: true,
      enableForms: false,
      localResourceRoots: [vscode.Uri.joinPath(extensionUri, 'media')],
    };
    panel.webview.html = this.html();

    this.disposables.push(
      panel.webview.onDidReceiveMessage((m: WebviewMessage) => this.onMessage(m)),
      panel.onDidChangeViewState(() => {
        this.updateState();
        this.post({ type: 'setActive', value: panel.active });
      }),
      panel.onDidDispose(() => this.dispose()),
      statusBar.onDidSelectScale(scale => {
        if (this.state === 'active') {
          this.post({ type: 'setScale', scale });
        }
      }),
    );

    // Reload when the file changes on disk, close when it is deleted (as the image preview does).
    if (resource.scheme === 'file' || vscode.workspace.fs.isWritableFileSystem(resource.scheme)) {
      const folder = vscode.Uri.joinPath(resource, '..');
      const name = resource.path.slice(resource.path.lastIndexOf('/') + 1);
      const watcher = vscode.workspace.createFileSystemWatcher(new vscode.RelativePattern(folder, name));
      this.disposables.push(
        watcher,
        watcher.onDidChange(uri => {
          if (uri.toString() === resource.toString()) {
            this.reload();
          }
        }),
        watcher.onDidDelete(uri => {
          if (uri.toString() === resource.toString()) {
            panel.dispose();
          }
        }),
      );
    }
    this.updateState();
  }

  snapshot(): PreviewSnapshot {
    return {
      uri: this.resource.toString(),
      state: this.state,
      width: this.image?.width,
      height: this.image?.height,
      rendered: this.rendered,
      scale: this.scale,
      mode: this.mode,
      error: this.error,
    };
  }

  get viewColumn(): vscode.ViewColumn | undefined {
    return this.panel.viewColumn;
  }

  zoomIn(): void {
    if (this.state === 'active') {
      this.post({ type: 'zoomIn' });
    }
  }

  zoomOut(): void {
    if (this.state === 'active') {
      this.post({ type: 'zoomOut' });
    }
  }

  copyImage(): void {
    if (this.state === 'active') {
      this.panel.reveal();
      this.post({ type: 'copyImage' });
    }
  }

  /** The decoded image, decoding it if needed. */
  async decoded(): Promise<WsqImage> {
    await this.load();
    if (!this.image) {
      throw new Error(this.error ?? 'the image could not be decoded');
    }
    return this.image;
  }

  async fields(): Promise<ReturnType<typeof describe>> {
    await this.load();
    if (!this.info || this.fileSize === undefined) {
      throw new Error(this.error ?? 'the image could not be read');
    }
    return describe(this.info, this.fileSize);
  }

  private onMessage(m: WebviewMessage): void {
    switch (m.type) {
      case 'ready':
        // Sent whenever the webview (re)starts, also after it was hidden and shown again.
        this.rendered = false;
        void this.load().then(() => this.send());
        break;
      case 'zoom':
        this.scale = m.value;
        this.updateState();
        break;
      case 'rendered':
        this.rendered = true;
        break;
      case 'viewState':
        this.viewStates.set(this.resource, m.state);
        break;
      case 'reopen-as-text':
        void vscode.commands.executeCommand('vscode.openWith', this.resource, 'default', this.panel.viewColumn);
        break;
    }
  }

  /** Read and decode the file once; later calls wait for the same work. */
  private load(): Promise<void> {
    this.loading ??= this.read();
    return this.loading;
  }

  private async read(): Promise<void> {
    this.image = undefined;
    this.info = undefined;
    this.error = undefined;
    try {
      if (await isGitLfsPointer(this.resource)) {
        this.error = 'lfs';
        return;
      }
      const data = await vscode.workspace.fs.readFile(this.resource);
      this.fileSize = data.length;
      const codec = await this.codec;
      const fast = decodesFast(this.resource);
      this.info = codec.info(data);
      this.image = codec.decode(data, fast);
      this.mode = fast ? 'fast' : 'exact';
    } catch (e) {
      this.error = errorMessage(e);
    } finally {
      this.updateState();
    }
  }

  private send(): void {
    if (this.error === 'lfs') {
      this.post({ type: 'lfs' });
    } else if (this.error !== undefined || !this.image) {
      this.post({ type: 'error', message: this.error ?? '' });
    } else {
      const { width, height, pixels } = this.image;
      this.post({ type: 'image', width, height, pixels });
    }
    this.post({ type: 'setActive', value: this.panel.active });
  }

  /** Read and decode the file again and repaint, keeping zoom and scroll. */
  reload(): void {
    if (this.state === 'disposed') {
      return;
    }
    this.loading = undefined;
    void this.load().then(() => this.send());
  }

  private post(message: unknown): void {
    if (this.state !== 'disposed') {
      void this.panel.webview.postMessage(message);
    }
  }

  private updateState(): void {
    if (this.state === 'disposed') {
      return;
    }
    if (this.panel.active) {
      this.state = 'active';
      const info = this.info && this.fileSize !== undefined
        ? {
          text: statusText(this.info, this.fileSize),
          tooltip: new vscode.MarkdownString(markdownTable(describe(this.info, this.fileSize))),
        }
        : undefined;
      this.statusBar.show(this, {
        size: this.image ? `${this.image.width}x${this.image.height}` : undefined,
        scale: this.scale,
        binarySize: this.fileSize,
        info,
      });
    } else {
      this.state = 'visible';
      this.statusBar.hide(this);
    }
  }

  private html(): string {
    const webview = this.panel.webview;
    const media = (file: string) => webview.asWebviewUri(vscode.Uri.joinPath(this.extensionUri, 'media', file)).toString();
    const n = nonce();
    const t = vscode.l10n.t;
    const escape = (s: string) => s.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
    const settings = escape(JSON.stringify({ viewState: this.viewStates.get(this.resource) ?? null }));
    return /* html */ `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0, maximum-scale=1.0, minimum-scale=1.0, user-scalable=no">
  <meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src ${webview.cspSource} data:; style-src ${webview.cspSource}; script-src 'nonce-${n}';">
  <meta id="wsq-preview-settings" data-settings="${settings}">
  <link rel="stylesheet" href="${media('preview.css')}">
  <title>WSQ Image Preview</title>
</head>
<body class="loading" data-vscode-context='{ "preventDefaultContextMenuItems": true }'>
  <div class="spinner" role="progressbar" aria-label="${t('Decoding the image')}"></div>
  <div class="message error-message">
    <p>${t('An error occurred while decoding the WSQ image.')}</p>
    <p class="detail"></p>
    <a href="#" class="open-file-link">${t("Open file using VS Code's standard text/binary editor?")}</a>
  </div>
  <div class="message lfs-message">
    <p>${t('The image is stored with Git LFS and is not available for preview.')}</p>
    <a href="#" class="open-file-link">${t("Open file using VS Code's standard text/binary editor?")}</a>
  </div>
  <script src="${media('preview.js')}" nonce="${n}"></script>
</body>
</html>`;
  }

  dispose(): void {
    if (this.state === 'disposed') {
      return;
    }
    this.statusBar.hide(this);
    this.state = 'disposed';
    for (const d of this.disposables) {
      d.dispose();
    }
  }
}

/** The custom editor provider: one Preview per editor, the active one receives commands. */
export class PreviewManager implements vscode.CustomReadonlyEditorProvider, ViewStateStore {
  private static readonly VIEW_STATES_KEY = 'wsqViewer.viewStates';
  private static readonly MAX_VIEW_STATES = 100;

  private readonly previews = new Set<Preview>();
  private active: Preview | undefined;
  private readonly extensionUri: vscode.Uri;
  private readonly codec: Promise<WsqCodec>;
  private readonly statusBar: StatusBar;
  private readonly memento: vscode.Memento;
  /** View states by resource, least recently used first. */
  private readonly viewStates: Map<string, ViewState>;

  constructor(extensionUri: vscode.Uri, codec: Promise<WsqCodec>, statusBar: StatusBar, memento: vscode.Memento) {
    this.extensionUri = extensionUri;
    this.codec = codec;
    this.statusBar = statusBar;
    this.memento = memento;
    this.viewStates = new Map(memento.get<[string, ViewState][]>(PreviewManager.VIEW_STATES_KEY, []));
  }

  /** Zoom and scroll of a resource seen before, unless the user turned view state restoring off. */
  get(uri: vscode.Uri): ViewState | undefined {
    if (!vscode.workspace.getConfiguration('workbench.editor', uri).get<boolean>('restoreViewState', true)) {
      return undefined;
    }
    return this.viewStates.get(uri.toString());
  }

  set(uri: vscode.Uri, state: ViewState | undefined): void {
    const key = uri.toString();
    this.viewStates.delete(key);
    if (state) {
      this.viewStates.set(key, state);
      for (const old of this.viewStates.keys()) {
        if (this.viewStates.size <= PreviewManager.MAX_VIEW_STATES) {
          break;
        }
        this.viewStates.delete(old);
      }
    }
    void this.memento.update(PreviewManager.VIEW_STATES_KEY, [...this.viewStates]);
  }

  openCustomDocument(uri: vscode.Uri): vscode.CustomDocument {
    return { uri, dispose: () => { } };
  }

  resolveCustomEditor(document: vscode.CustomDocument, panel: vscode.WebviewPanel): void {
    const preview = new Preview(document.uri, panel, this.codec, this.statusBar, this.extensionUri, this);
    this.previews.add(preview);
    if (panel.active) {
      this.active = preview;
    }
    panel.onDidChangeViewState(() => {
      if (panel.active) {
        this.active = preview;
      } else if (this.active === preview) {
        this.active = undefined;
      }
    });
    panel.onDidDispose(() => {
      this.previews.delete(preview);
      if (this.active === preview) {
        this.active = undefined;
      }
    });
  }

  get activePreview(): Preview | undefined {
    return this.active;
  }

  reloadAll(): void {
    for (const p of this.previews) {
      p.reload();
    }
  }

  snapshots(): PreviewSnapshot[] {
    return [...this.previews].map(p => p.snapshot());
  }
}

import * as vscode from 'vscode';

export type Scale = number | 'fit';

/** The zoom levels the status bar offers, as in VS Code's image preview. */
const SCALES: Scale[] = [10, 5, 2, 1, 0.5, 0.2, 'fit'];

export function zoomLabel(scale: Scale): string {
  return scale === 'fit' ? vscode.l10n.t('Whole Image') : `${Math.round(scale * 100)}%`;
}

/** File sizes in the format of VS Code's image preview (binary units, two decimals). */
export function formatBytes(size: number): string {
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0;
  let v = size;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return i === 0 ? `${size}B` : `${v.toFixed(2)}${units[i]}`;
}

/** A status bar item that one preview at a time (the active one) owns. */
class OwnedItem implements vscode.Disposable {
  readonly item: vscode.StatusBarItem;
  private owner: unknown;

  constructor(id: string, name: string, priority: number) {
    this.item = vscode.window.createStatusBarItem(id, vscode.StatusBarAlignment.Right, priority);
    this.item.name = name;
  }

  show(owner: unknown, text: string, tooltip?: string | vscode.MarkdownString): void {
    this.owner = owner;
    this.item.text = text;
    this.item.tooltip = tooltip;
    this.item.show();
  }

  hide(owner: unknown): void {
    if (owner === this.owner) {
      this.item.hide();
      this.owner = undefined;
    }
  }

  dispose(): void {
    this.item.dispose();
  }
}

export interface StatusContent {
  /** "width x height", once the image is decoded. */
  size?: string;
  scale: Scale;
  binarySize?: number;
  info?: { text: string; tooltip: vscode.MarkdownString };
}

/**
 * The status bar of the active WSQ preview. The size, zoom and file size entries sit where VS
 * Code's image preview puts its own (priorities 101, 102 and 100, right of the editor status);
 * the WSQ entry (resolution and bit rate) sits to their left.
 */
export class StatusBar implements vscode.Disposable {
  private readonly info = new OwnedItem('wsqViewer.info', vscode.l10n.t('WSQ Image Information'), 103);
  private readonly zoom = new OwnedItem('wsqViewer.zoom', vscode.l10n.t('WSQ Image Zoom'), 102);
  private readonly size = new OwnedItem('wsqViewer.size', vscode.l10n.t('WSQ Image Size'), 101);
  private readonly binarySize = new OwnedItem('wsqViewer.binarySize', vscode.l10n.t('WSQ Image Binary Size'), 100);
  private readonly scaleEmitter = new vscode.EventEmitter<Scale>();
  private readonly disposables: vscode.Disposable[] = [];

  /** The user picked a zoom level from the status bar. */
  readonly onDidSelectScale = this.scaleEmitter.event;

  constructor() {
    this.info.item.command = 'wsqViewer.showInfo';
    this.zoom.item.command = 'wsqViewer.selectZoomLevel';
    this.disposables.push(
      this.info, this.zoom, this.size, this.binarySize, this.scaleEmitter,
      vscode.commands.registerCommand('wsqViewer.selectZoomLevel', async () => {
        const pick = await vscode.window.showQuickPick(
          SCALES.map(scale => ({ label: zoomLabel(scale), scale })),
          { placeHolder: vscode.l10n.t('Select zoom level') },
        );
        if (pick) {
          this.scaleEmitter.fire(pick.scale);
        }
      }),
    );
  }

  show(owner: unknown, c: StatusContent): void {
    if (c.info) {
      this.info.show(owner, c.info.text, c.info.tooltip);
    } else {
      this.info.hide(owner);
    }
    this.zoom.show(owner, zoomLabel(c.scale), vscode.l10n.t('Select Zoom Level'));
    this.size.show(owner, c.size ?? '');
    if (c.binarySize !== undefined) {
      this.binarySize.show(owner, formatBytes(c.binarySize));
    } else {
      this.binarySize.hide(owner);
    }
  }

  hide(owner: unknown): void {
    for (const e of [this.info, this.zoom, this.size, this.binarySize]) {
      e.hide(owner);
    }
  }

  dispose(): void {
    for (const d of this.disposables) {
      d.dispose();
    }
  }
}

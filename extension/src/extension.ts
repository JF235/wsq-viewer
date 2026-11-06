import * as vscode from 'vscode';
import { WsqCodec } from './codec.ts';
import { describe } from './info.ts';
import { encodePng } from './png.ts';
import { decodesFast, PreviewManager, VIEW_TYPE, type PreviewSnapshot } from './preview.ts';
import { StatusBar } from './statusBar.ts';

/** What the integration tests read (only returned in test mode). */
export interface TestApi {
  previews(): PreviewSnapshot[];
  lanes(): Promise<number>;
}

async function loadCodec(extensionUri: vscode.Uri): Promise<WsqCodec> {
  const read = (name: string) => vscode.workspace.fs.readFile(vscode.Uri.joinPath(extensionUri, 'dist', 'wasm', name));
  const [scalar, simd] = await Promise.all([read('wsq-scalar.wasm'), read('wsq-simd.wasm')]);
  return WsqCodec.load(scalar, simd);
}

function basename(uri: vscode.Uri): string {
  return uri.path.slice(uri.path.lastIndexOf('/') + 1);
}

/** Where "Export as PNG" proposes to save: next to the source when that is writable. */
function defaultPngUri(source: vscode.Uri): vscode.Uri {
  const name = basename(source).replace(/\.wsq$/i, '') + '.png';
  if (source.scheme === 'file' || vscode.workspace.fs.isWritableFileSystem(source.scheme)) {
    return vscode.Uri.joinPath(source, '..', name);
  }
  const folder = vscode.workspace.workspaceFolders?.[0]?.uri;
  return folder ? vscode.Uri.joinPath(folder, name) : vscode.Uri.file(name);
}

export function activate(context: vscode.ExtensionContext): TestApi | undefined {
  const codec = loadCodec(context.extensionUri);
  codec.catch(e => void vscode.window.showErrorMessage(vscode.l10n.t('WSQ Viewer could not load its decoder: {0}', String(e))));

  const statusBar = new StatusBar();
  const manager = new PreviewManager(context.extensionUri, codec, statusBar, context.workspaceState);

  /** The image a command acts on: an explicit resource (explorer, tests) or the active preview. */
  const target = (uri?: vscode.Uri) => uri ?? manager.activePreview?.resource;

  context.subscriptions.push(
    statusBar,
    vscode.window.registerCustomEditorProvider(VIEW_TYPE, manager, {
      supportsMultipleEditorsPerDocument: true,
    }),
    vscode.workspace.onDidChangeConfiguration(e => {
      if (e.affectsConfiguration('wsqViewer.decodeMode')) {
        manager.reloadAll();
      }
    }),
    vscode.commands.registerCommand('wsqViewer.zoomIn', () => manager.activePreview?.zoomIn()),
    vscode.commands.registerCommand('wsqViewer.zoomOut', () => manager.activePreview?.zoomOut()),
    vscode.commands.registerCommand('wsqViewer.copyImage', () => manager.activePreview?.copyImage()),

    vscode.commands.registerCommand('wsqViewer.exportPng', async (uri?: vscode.Uri, destination?: vscode.Uri) => {
      const source = target(uri instanceof vscode.Uri ? uri : undefined);
      if (!source) {
        void vscode.window.showWarningMessage(vscode.l10n.t('Open a WSQ image first.'));
        return undefined;
      }
      const dest = destination ?? await vscode.window.showSaveDialog({
        defaultUri: defaultPngUri(source),
        filters: { [vscode.l10n.t('PNG image')]: ['png'] },
        title: vscode.l10n.t('Export {0} as PNG', basename(source)),
      });
      if (!dest) {
        return undefined;
      }
      try {
        const img = (await codec).decode(await vscode.workspace.fs.readFile(source), decodesFast(source));
        await vscode.workspace.fs.writeFile(dest, await encodePng(img.pixels, img.width, img.height, img.ppi));
      } catch (e) {
        void vscode.window.showErrorMessage(vscode.l10n.t('Could not export {0}: {1}', basename(source), e instanceof Error ? e.message : String(e)));
        return undefined;
      }
      if (!destination) {
        const open = vscode.l10n.t('Open');
        void vscode.window.showInformationMessage(vscode.l10n.t('Exported {0}.', basename(dest)), open).then(choice => {
          if (choice === open) {
            void vscode.commands.executeCommand('vscode.open', dest);
          }
        });
      }
      return dest;
    }),

    vscode.commands.registerCommand('wsqViewer.showInfo', async (uri?: vscode.Uri) => {
      const source = target(uri instanceof vscode.Uri ? uri : undefined);
      if (!source) {
        return;
      }
      let fields;
      try {
        const data = await vscode.workspace.fs.readFile(source);
        fields = describe((await codec).info(data), data.length);
      } catch (e) {
        void vscode.window.showErrorMessage(vscode.l10n.t('Could not read {0}: {1}', basename(source), e instanceof Error ? e.message : String(e)));
        return;
      }
      const pick = await vscode.window.showQuickPick(
        fields.map(f => ({ label: f.label, description: f.value })),
        { title: basename(source), placeHolder: vscode.l10n.t('Select a field to copy its value'), matchOnDescription: true },
      );
      if (pick?.description) {
        await vscode.env.clipboard.writeText(pick.description);
      }
    }),
  );

  if (context.extensionMode === vscode.ExtensionMode.Test) {
    return { previews: () => manager.snapshots(), lanes: async () => (await codec).lanes };
  }
  return undefined;
}

export function deactivate(): void { }

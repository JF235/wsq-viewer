import * as vscode from 'vscode';
import { spawn } from 'child_process';
import * as path from 'path';
import { getNonce } from './getNonce';

export function activate(context: vscode.ExtensionContext) {

    const provider = new WsqViewerProvider(context);
    context.subscriptions.push(
        vscode.window.registerCustomEditorProvider(WsqViewerProvider.viewType, provider, {
            webviewOptions: {
                retainContextWhenHidden: true,
            },
        })
    );
}

class WsqViewerProvider implements vscode.CustomReadonlyEditorProvider {

    public static readonly viewType = 'wsq.viewer';

    constructor(private readonly context: vscode.ExtensionContext) { }

    public openCustomDocument(
        uri: vscode.Uri
    ): vscode.CustomDocument {
        return { uri, dispose: () => { /* nada a limpar */ } };
    }

    public async resolveCustomEditor(
        document: vscode.CustomDocument,
        webviewPanel: vscode.WebviewPanel,
        _token: vscode.CancellationToken
    ): Promise<void> {

        webviewPanel.webview.options = {
            enableScripts: true,
            localResourceRoots: [
                vscode.Uri.joinPath(this.context.extensionUri, 'media')
            ]
        };

        webviewPanel.webview.html = this.getLoadingHtml(webviewPanel.webview);

        try {
            // Passe o webviewPanel para que possamos ler as configurações
            const base64Image = await this.getWsqAsBase64(document.uri, webviewPanel);
            webviewPanel.webview.html = this.getImageHtml(webviewPanel.webview, base64Image);

        } catch (error) {
            const errorMessage = (error instanceof Error) ? error.message : 'Erro desconhecido';
            webviewPanel.webview.html = this.getErrorHtml(webviewPanel.webview, errorMessage);
            vscode.window.showErrorMessage(`Falha ao decodificar WSQ: ${errorMessage}`);
        }
    }

    /**
     * ALTERADO: Esta função agora aceita o webviewPanel para ler configurações
     */
    private getWsqAsBase64(uri: vscode.Uri, webviewPanel: vscode.WebviewPanel): Promise<string> {
        return new Promise((resolve, reject) => {
            
            // Verifique seu 'webpack.config.js' para 'output.path'
            // Assumindo que seu webpack.config.js usa 'dist'
            const scriptPath = vscode.Uri.joinPath(
                this.context.extensionUri, 'dist', 'decoder.py'
            ).fsPath;
            
            const wsqPath = uri.fsPath;

            // --- NOVO: Lógica para encontrar o Python ---
            // 1. Tenta ler o caminho definido nas configurações do VS Code
            // CORREÇÃO: O segundo argumento deve ser um 'ConfigurationScope', como a 'uri' do recurso.
            const configPythonPath = vscode.workspace.getConfiguration('wsq.viewer', uri).get<string>('pythonPath');
            
            // 2. Define a lista de executáveis a tentar
            const pythonExecutables = configPythonPath 
                ? [configPythonPath] // Se o usuário definiu, usa apenas esse.
                : ['python3', 'python']; // Senão, tenta os padrões.
            // --- Fim da nova lógica ---

            let stdout = '';
            let stderr = '';

            // Tenta executar o primeiro comando da lista
            // ADICIONADO: lastError para rastrear o erro específico do spawn
            const trySpawn = (index: number, lastError: Error | null = null) => {
                if (index >= pythonExecutables.length) {
                    // Tentamos todos e falhamos
                    const errorMsg = configPythonPath
                        ? `O caminho do Python definido ('${configPythonPath}') não foi encontrado ou falhou.`
                        : "Não foi possível encontrar 'python3' ou 'python'. Por favor, garanta que o Python 3 está instalado e no seu PATH, ou defina 'wsq.viewer.pythonPath' nas suas configurações.";
                    
                    // --- NOVO: Adiciona o detalhe do último erro do spawn ---
                    const errorDetails = lastError ? ` (Detalhe: ${lastError.message})` : '';
                    reject(new Error(errorMsg + errorDetails));
                    // --- Fim da nova lógica ---
                    return;
                }

                const currentPython = pythonExecutables[index];
                const pythonProcess = spawn(currentPython, [scriptPath, wsqPath]);

                pythonProcess.stdout.on('data', (data) => {
                    stdout += data.toString();
                });

                pythonProcess.stderr.on('data', (data) => {
                    stderr += data.toString();
                });

                pythonProcess.on('close', (code) => {
                    if (code === 0) {
                        // Sucesso!
                        resolve(stdout.trim());
                    } else {
                        // O script falhou (ex: 'wsq' não instalado NESSE python)
                        // Rejeita com o erro do script
                        reject(new Error(stderr.trim() || `O script Python ('${currentPython}') falhou com o código ${code}.`));
                    }
                });

                pythonProcess.on('error', (err) => {
                    // O comando spawn falhou (ex: 'python3' não encontrado)
                    // Tenta o próximo da lista, passando o erro específico
                    trySpawn(index + 1, err);
                });
            };

            // Inicia a primeira tentativa
            trySpawn(0);
        });
    }

    // --- Funções HTML (getLoadingHtml, getErrorHtml, getImageHtml) ---
    // ... (Cole as 3 funções HTML da resposta anterior aqui. Elas não mudam.) ...
    
    private getLoadingHtml(webview: vscode.Webview): string {
        // (Esta função permanece a mesma do passo anterior)
        const nonce = getNonce();
        return `<!DOCTYPE html>
            <html lang="en">
            <head>
                <meta charset="UTF-8">
                <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${webview.cspSource}; script-src 'nonce-${nonce}';">
                <meta name="viewport" content="width=device-width, initial-scale=1.0">
                <title>Carregando WSQ</title>
                <style>
                    body, html { width: 100%; height: 100%; margin: 0; padding: 0; display: flex; align-items: center; justify-content: center; background-color: var(--vscode-editor-background); color: var(--vscode-editor-foreground); }
                    .loader { border: 5px solid var(--vscode-editor-foreground, #f3f3f3); border-top: 5px solid var(--vscode-button-background, #3498db); border-radius: 50%; width: 50px; height: 50px; animation: spin 1s linear infinite; }
                    @keyframes spin { 0% { transform: rotate(0deg); } 100% { transform: rotate(360deg); } }
                </style>
            </head>
            <body>
                <div class="loader"></div>
            </body>
            </html>`;
    }

    private getErrorHtml(webview: vscode.Webview, error: string): string {
         // (Esta função permanece a mesma do passo anterior)
        const nonce = getNonce();
        return `<!DOCTYPE html>
            <html lang="en">
            <head>
                <meta charset="UTF-8">
                <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${webview.cspSource}; script-src 'nonce-${nonce}';">
                <meta name="viewport" content="width=device-width, initial-scale=1.0">
                <title>Erro ao Carregar WSQ</title>
                <style>
                    body, html { width: 100%; height: 100%; margin: 0; padding: 1em; display: flex; flex-direction: column; align-items: center; justify-content: center; background-color: var(--vscode-editor-background); color: var(--vscode-errorForeground); font-family: var(--vscode-font-family, sans-serif); }
                    .error-container { background-color: var(--vscode-input-background); padding: 2em; border-radius: 5px; border: 1px solid var(--vscode-input-border); }
                    h1 { margin-top: 0; }
                    pre { white-space: pre-wrap; word-wrap: break-word; background-color: var(--vscode-textBlockQuote-background); padding: 1em; border-radius: 3px; }
                </style>
            </head>
            <body>
                <div class="error-container">
                    <h1>Erro ao Decodificar Imagem WSQ</h1>
                    <p>Não foi possível exibir o arquivo. Verifique os detalhes abaixo:</p>
                    <pre>${error.replace(/</g, "&lt;").replace(/>/g, "&gt;")}</pre>
                    <p><b>Ações Possíveis:</b></p>
                    <ul>
                        <li>Verifique se o Python 3 está instalado e no PATH do sistema.</li>
                        <li>Verifique se a biblioteca <code>wsq</code> está instalada (<code>pip install wsq</code>).</li>
                        <li>Se estiver usando um ambiente virtual (como Conda), defina o <b>wsq.viewer.pythonPath</b> nas suas configurações do VS Code.</li>
                    </ul>
                </div>
            </body>
            </html>`;
    }

    private getImageHtml(webview: vscode.Webview, base64Image: string): string {
        
        const panzoomUri = webview.asWebviewUri(
            vscode.Uri.joinPath(this.context.extensionUri, 'media', 'panzoom.min.js')
        );

        const nonce = getNonce();

        return `<!DOCTYPE html>
            <html lang="en">
            <head>
                <meta charset="UTF-8">
                <meta http-equiv="Content-Security-Policy" content="
                    default-src 'none'; 
                    img-src data: ${webview.cspSource}; 
                    style-src ${webview.cspSource}; 
                    script-src 'nonce-${nonce}' ${webview.cspSource};
                ">
                <meta name="viewport" content="width=device-width, initial-scale=1.0">
                <title>WSQ Viewer</title>
                <style>
                    body, html {
                        width: 100%;
                        height: 100%;
                        margin: 0;
                        padding: 0;
                        overflow: hidden;
                        background-color: var(--vscode-editor-background);
                    }
                    #panzoom-container {
                        width: 100%;
                        height: 100%;
                        display: flex;
                        align-items: center;
                        justify-content: center;
                        cursor: grab;
                    }
                    #panzoom-container:active {
                        cursor: grabbing;
                    }
                    img {
                        filter: var(--vscode-editor-foreground, #000000) brightness(0) invert(1);
                    }
                    body[data-vscode-theme-kind="vscode-light"] img,
                    body[data-vscode-theme-kind="vscode-high-contrast-light"] img {
                        filter: none;
                    }
                    .controls {
                        position: fixed;
                        top: 10px;
                        right: 10px;
                        z-index: 10;
                        display: flex;
                        flex-direction: column;
                        gap: 5px;
                    }
                    .controls button {
                        background-color: var(--vscode-button-background);
                        color: var(--vscode-button-foreground);
                        border: 1px solid var(--vscode-button-border, transparent);
                        border-radius: 3px;
                        padding: 5px 10px;
                        cursor: pointer;
                        font-family: var(--vscode-font-family, sans-serif);
                    }
                    .controls button:hover {
                        background-color: var(--vscode-button-hoverBackground);
                    }
                </style>
            </head>
            <body>
                
                <div id="panzoom-container">
                    <img src="data:image/png;base64,${base64Image}" alt="Imagem WSQ" id="wsq-image" />
                </div>

                <div class="controls">
                    <button id="zoom-in">+</button>
                    <button id="zoom-out">-</button>
                    <button id="reset">Reset</button>
                </div>

                <script src="${panzoomUri}"></script>

                <script nonce="${nonce}">
                    const elem = document.getElementById('wsq-image');
                    const container = document.getElementById('panzoom-container');
                    
                    const pz = panzoom(elem, {
                        maxZoom: 10,
                        minZoom: 0.1,
                        bounds: true,
                        boundsPadding: 0.1,
                    });

                    container.addEventListener('wheel', pz.zoomWithWheel);

                    document.getElementById('zoom-in').addEventListener('click', pz.zoomIn);
                    document.getElementById('zoom-out').addEventListener('click', pz.zoomOut);
                    document.getElementById('reset').addEventListener('click', () => {
                        pz.moveTo(0, 0);
                        pz.zoomAbs(0, 0, 1);
                    });

                    document.body.dataset.vscodeThemeKind = document.body.className;
                </script>
            </body>
            </html>`;
    }
}


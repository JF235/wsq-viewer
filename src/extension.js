"use strict";
var __createBinding = (this && this.__createBinding) || (Object.create ? (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    var desc = Object.getOwnPropertyDescriptor(m, k);
    if (!desc || ("get" in desc ? !m.__esModule : desc.writable || desc.configurable)) {
      desc = { enumerable: true, get: function() { return m[k]; } };
    }
    Object.defineProperty(o, k2, desc);
}) : (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    o[k2] = m[k];
}));
var __setModuleDefault = (this && this.__setModuleDefault) || (Object.create ? (function(o, v) {
    Object.defineProperty(o, "default", { enumerable: true, value: v });
}) : function(o, v) {
    o["default"] = v;
});
var __importStar = (this && this.__importStar) || (function () {
    var ownKeys = function(o) {
        ownKeys = Object.getOwnPropertyNames || function (o) {
            var ar = [];
            for (var k in o) if (Object.prototype.hasOwnProperty.call(o, k)) ar[ar.length] = k;
            return ar;
        };
        return ownKeys(o);
    };
    return function (mod) {
        if (mod && mod.__esModule) return mod;
        var result = {};
        if (mod != null) for (var k = ownKeys(mod), i = 0; i < k.length; i++) if (k[i] !== "default") __createBinding(result, mod, k[i]);
        __setModuleDefault(result, mod);
        return result;
    };
})();
Object.defineProperty(exports, "__esModule", { value: true });
exports.activate = activate;
const vscode = __importStar(require("vscode"));
const child_process_1 = require("child_process");
const getNonce_js_1 = require("./getNonce.js");
function activate(context) {
    const provider = new WsqViewerProvider(context);
    context.subscriptions.push(vscode.window.registerCustomEditorProvider(WsqViewerProvider.viewType, provider, {
        webviewOptions: {
            retainContextWhenHidden: true,
        },
    }));
}
class WsqViewerProvider {
    context;
    static viewType = 'wsq.viewer';
    constructor(context) {
        this.context = context;
    }
    openCustomDocument(uri) {
        return { uri, dispose: () => { } };
    }
    async resolveCustomEditor(document, webviewPanel, _token) {
        // ALTERADO: Configuração do Webview
        webviewPanel.webview.options = {
            enableScripts: true,
            // NOVO: Permite que o webview carregue arquivos da pasta 'media'
            localResourceRoots: [
                vscode.Uri.joinPath(this.context.extensionUri, 'media')
            ]
        };
        webviewPanel.webview.html = this.getLoadingHtml(webviewPanel.webview);
        try {
            const base64Image = await this.getWsqAsBase64(document.uri);
            // NOVO: Passa a URI do webview para a função HTML
            webviewPanel.webview.html = this.getImageHtml(webviewPanel.webview, base64Image);
        }
        catch (error) {
            const errorMessage = (error instanceof Error) ? error.message : 'Erro desconhecido';
            webviewPanel.webview.html = this.getErrorHtml(webviewPanel.webview, errorMessage);
            vscode.window.showErrorMessage(`Falha ao decodificar WSQ: ${errorMessage}`);
        }
    }
    getWsqAsBase64(uri) {
        return new Promise((resolve, reject) => {
            // ALTERADO: O caminho do script agora deve apontar para a pasta de saída
            // (geralmente 'dist' ou 'out'). Verifique seu webpack.config.js
            const outDir = this.context.extensionMode === vscode.ExtensionMode.Development
                ? 'out' // Pasta 'out' se estiver em modo de desenvolvimento (F5)
                : 'dist'; // Pasta 'dist' se for a extensão instalada
            // Verifique seu 'webpack.config.js' para 'output.path'
            // Se for 'dist', use 'dist'. Se for 'out', use 'out'.
            // Vamos assumir 'dist' como padrão do template moderno.
            const scriptPath = vscode.Uri.joinPath(this.context.extensionUri, 'dist', 'decoder.py' // Verifique se é 'dist' ou 'out'
            ).fsPath;
            const wsqPath = uri.fsPath;
            const pythonProcess = (0, child_process_1.spawn)('python3', [scriptPath, wsqPath]);
            let stdout = '';
            let stderr = '';
            pythonProcess.stdout.on('data', (data) => {
                stdout += data.toString();
            });
            pythonProcess.stderr.on('data', (data) => {
                stderr += data.toString();
            });
            pythonProcess.on('close', (code) => {
                if (code === 0) {
                    resolve(stdout.trim());
                }
                else {
                    reject(new Error(stderr.trim() || 'O script Python falhou sem saída de erro.'));
                }
            });
            pythonProcess.on('error', (err) => {
                if (err.message.includes('ENOENT')) {
                    const fallbackProcess = (0, child_process_1.spawn)('python', [scriptPath, wsqPath]);
                    let fbStdout = '';
                    let fbStderr = '';
                    fallbackProcess.stdout.on('data', (data) => fbStdout += data.toString());
                    fallbackProcess.stderr.on('data', (data) => fbStderr += data.toString());
                    fallbackProcess.on('close', (fbCode) => {
                        if (fbCode === 0) {
                            resolve(fbStdout.trim());
                        }
                        else {
                            reject(new Error(fbStderr.trim() || "Falha ao executar 'python'. Certifique-se de que Python está no PATH."));
                        }
                    });
                    fallbackProcess.on('error', () => {
                        reject(new Error("Não foi possível encontrar 'python3' ou 'python'. Por favor, garanta que o Python 3 está instalado e no seu PATH."));
                    });
                }
                else {
                    reject(new Error(`Falha ao iniciar o processo Python: ${err.message}`));
                }
            });
        });
    }
    // --- Funções HTML ---
    getLoadingHtml(webview) {
        // (Esta função permanece a mesma do passo anterior)
        const nonce = (0, getNonce_js_1.getNonce)();
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
    getErrorHtml(webview, error) {
        // (Esta função permanece a mesma do passo anterior)
        const nonce = (0, getNonce_js_1.getNonce)();
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
                    </ul>
                </div>
            </body>
            </html>`;
    }
    /**
     * ALTERADO: Esta função agora inclui o panzoom.js e controles
     */
    getImageHtml(webview, base64Image) {
        // NOVO: Obter a URI especial para o script na pasta 'media'
        const panzoomUri = webview.asWebviewUri(vscode.Uri.joinPath(this.context.extensionUri, 'media', 'panzoom.min.js'));
        const nonce = (0, getNonce_js_1.getNonce)();
        return `<!DOCTYPE html>
            <html lang="en">
            <head>
                <meta charset="UTF-8">
                
                <!-- ALTERADO: Política de Segurança de Conteúdo -->
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
                        /* NOVO: Esconde o overflow para o pan */
                        overflow: hidden;
                        background-color: var(--vscode-editor-background);
                    }
                    /* NOVO: O contêiner onde o pan/zoom acontece */
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
                    /* ALTERADO: A imagem não deve ter max-width/height */
                    img {
                        /* O filtro de inversão para temas escuros */
                        filter: var(--vscode-editor-foreground, #000000) brightness(0) invert(1);
                        /* Remove o 'object-fit' e 'max-width/height' */
                    }
                    body[data-vscode-theme-kind="vscode-light"] img,
                    body[data-vscode-theme-kind="vscode-high-contrast-light"] img {
                        filter: none;
                    }

                    /* NOVO: Estilos para os botões de controle */
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
                
                <!-- NOVO: Contêiner para o panzoom -->
                <div id="panzoom-container">
                    <img src="data:image/png;base64,${base64Image}" alt="Imagem WSQ" id="wsq-image" />
                </div>

                <!-- NOVO: Botões de controle -->
                <div class="controls">
                    <button id="zoom-in">+</button>
                    <button id="zoom-out">-</button>
                    <button id="reset">Reset</button>
                </div>

                <!-- NOVO: Carrega a biblioteca panzoom -->
                <script src="${panzoomUri}"></script>

                <!-- NOVO: Script de inicialização -->
                <script nonce="${nonce}">
                    // Pega os elementos
                    const elem = document.getElementById('wsq-image');
                    const container = document.getElementById('panzoom-container');
                    
                    // Inicializa o panzoom
                    const pz = panzoom(elem, {
                        maxZoom: 10,
                        minZoom: 0.1,
                        bounds: true, // Mantém a imagem dentro da tela
                        boundsPadding: 0.1,
                    });

                    // 1. Habilita o zoom com a roda do mouse
                    container.addEventListener('wheel', pz.zoomWithWheel);

                    // 2. Conecta os botões
                    document.getElementById('zoom-in').addEventListener('click', pz.zoomIn);
                    document.getElementById('zoom-out').addEventListener('click', pz.zoomOut);
                    document.getElementById('reset').addEventListener('click', () => {
                        // Reseta o pan (movimento) e o zoom
                        pz.moveTo(0, 0);
                        pz.zoomAbs(0, 0, 1); // Zoom para 1x, centrado
                    });

                    // Passa o tema atual do VS Code para o CSS (para o filtro da imagem)
                    document.body.dataset.vscodeThemeKind = document.body.className;
                </script>
            </body>
            </html>`;
    }
}
//# sourceMappingURL=extension.js.map
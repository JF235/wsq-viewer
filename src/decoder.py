import sys
import io
import base64
from PIL import Image

# Importar 'wsq' registra o plugin na biblioteca PIL (Pillow)
try:
    import wsq
except ImportError:
    print("Erro: A biblioteca 'wsq' nao foi encontrada. Por favor, instale com 'pip install wsq'", file=sys.stderr)
    sys.exit(1)

def decode_wsq(file_path):
    """
    Decodifica um arquivo WSQ para uma string Base64 PNG.
    """
    try:
        # 1. Abrir a imagem WSQ (graças ao 'import wsq', o PIL agora entende .wsq)
        img = Image.open(file_path)

        # 2. Salvar a imagem em um buffer de memória no formato PNG
        with io.BytesIO() as buffered:
            img.save(buffered, format="PNG")
            
            # 3. Codificar os bytes do PNG em Base64
            img_bytes = buffered.getvalue()
            img_base64 = base64.b64encode(img_bytes)
            
            # 4. Decodificar para UTF-8 para que possa ser impresso como texto
            return img_base64.decode('utf-8')

    except Exception as e:
        # Imprime qualquer erro no stderr para o VS Code capturar
        print(f"Erro ao processar a imagem: {e}", file=sys.stderr)
        sys.exit(1)

if __name__ == "__main__":
    # O caminho do arquivo é passado como o primeiro argumento
    if len(sys.argv) < 2:
        print("Erro: Nenhum arquivo fornecido.", file=sys.stderr)
        sys.exit(1)
        
    file_path = sys.argv[1]
    
    # Executa a decodificação e imprime o resultado no stdout
    base64_string = decode_wsq(file_path)
    if base64_string:
        print(base64_string)

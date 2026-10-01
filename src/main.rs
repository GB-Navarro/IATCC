/*
   Este binário é o ponto de entrada mínimo do crate. Os fluxos operacionais
   ficam nos binários especializados em src/bin; este arquivo mantém o pacote
   executável padrão simples e útil para verificar a instalação do MVP.
*/

fn main() {
    // Informa que o executável base está instalado; as operações reais usam os subcomandos.
    eprintln!("Use um dos binários do MVP: capturar, preparar, classificar, mcp ou web.");
}

/*
   Este binário é a entrada CLI da captura DTCC. Recebe a janela e o diretório
   de saída e delega o download incremental ao módulo dtcc da biblioteca.
*/

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "capturar",
    about = "Congela uma amostra de slices públicos do DTCC (CFTC_FX)"
)]
struct Args {
    /// Minutos de captura ao vivo: slices publicados a partir de agora.
    #[arg(long, default_value_t = 60)]
    minutos: u64,
    /// Inclui também os slices iniciados nos últimos N minutos (o Slice.json guarda ~24 h).
    #[arg(long, default_value_t = 0)]
    recentes: u64,
    /// Em vez de slices, baixa o arquivo cumulativo de um dia inteiro (AAAA-MM-DD).
    #[arg(long)]
    dia: Option<String>,
    #[arg(long, default_value = "data/amostra")]
    diretorio: PathBuf,
}

fn main() -> Result<()> {
    // Faz o parse dos argumentos e inicia a captura do módulo dtcc.
    let args = Args::parse();
    match args.dia {
        Some(dia) => IATCC::dtcc::capture_day(&dia, args.diretorio),
        None => IATCC::dtcc::capture_for(args.minutos, args.recentes, args.diretorio),
    }
}

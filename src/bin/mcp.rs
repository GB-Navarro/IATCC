/*
   Este binário implementa o servidor MCP via JSON-RPC sobre stdin/stdout, com
   as quatro ferramentas da seção 9 do MVP. Consultas usam o banco diretamente;
   a classificação chama a mesma fábrica genérica de providers do CLI. Nada
   além do protocolo vai para o stdout: logs vão para o stderr.
*/

use anyhow::{Result, bail};
use clap::Parser;
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::path::PathBuf;

#[derive(Debug, Parser)]
struct Args {
    // Configuração compartilhada pelas ferramentas MCP de consulta/classificação.
    #[arg(long, default_value = "mvp.db")]
    banco: PathBuf,
    #[arg(long, default_value = "brain")]
    brain: PathBuf,
    #[arg(long, default_value = "config/thresholds.toml")]
    config: PathBuf,
    #[arg(long)]
    provider: Option<String>,
    #[arg(long, default_value = "config/jev_criteria.json")]
    jev_criteria: PathBuf,
}

fn ferramentas() -> Value {
    json!([
        { "name": "listar_grupos",
          "description": "Lista grupos da amostra, com a classificação da IA quando houver. `par` filtra por trecho do par (ex.: BRL); `estrutura` casa com a resposta da IA ou com a regra do código.",
          "inputSchema": { "type": "object", "properties": {
              "par": { "type": "string" }, "estrutura": { "type": "string" },
              "limite": { "type": "integer" } } } },
        { "name": "ver_grupo",
          "description": "Pernas, números do teste do hedge e classificações de um grupo.",
          "inputSchema": { "type": "object", "required": ["grupo_id"],
              "properties": { "grupo_id": { "type": "string" } } } },
        { "name": "classificar_grupo",
          "description": "Pede à IA a classificação de um grupo e grava o resultado.",
          "inputSchema": { "type": "object", "required": ["grupo_id"],
              "properties": { "grupo_id": { "type": "string" } } } },
        { "name": "resumo",
          "description": "Contagem por estrutura e quantas respostas da IA concordam com a regra do código.",
          "inputSchema": { "type": "object", "properties": {} } }
    ])
}

fn executar(nome: &str, args: &Value, cli: &Args) -> Result<Value> {
    // Despacha uma ferramenta MCP e converte seu resultado para JSON serializável.
    let conn = IATCC::db::open_existente(&cli.banco)?;
    let grupo_id = || {
        args.get("grupo_id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("grupo_id é obrigatório"))
    };
    match nome {
        "listar_grupos" => {
            let par = args.get("par").and_then(Value::as_str);
            let estrutura = args.get("estrutura").and_then(Value::as_str);
            let limite = args.get("limite").and_then(Value::as_u64).unwrap_or(100) as usize;
            IATCC::db::list_groups(&conn, par, estrutura, limite)
        }
        "ver_grupo" => {
            let id = grupo_id()?;
            let grupo = IATCC::db::get_group(&conn, id)?;
            Ok(json!({
                "grupo": IATCC::ia::payload(&grupo),
                "regra_codigo": grupo.regra_codigo,
                "regra_codigo_ref": grupo.regra_codigo_ref,
                "classificacoes": IATCC::db::history(&conn, id)?,
            }))
        }
        "classificar_grupo" => {
            let classifier =
                IATCC::ia::classifier_from_env(cli.provider.as_deref(), &cli.jev_criteria)?;
            let outcome = IATCC::ia::classificar_e_gravar(
                &conn,
                grupo_id()?,
                &cli.brain,
                &cli.config,
                classifier.as_ref(),
            )?;
            match outcome.classificacao {
                Some(c) => Ok(serde_json::to_value(c)?),
                None => bail!("a IA recusou a classificação"),
            }
        }
        "resumo" => IATCC::db::summary(&conn),
        _ => bail!("ferramenta desconhecida: {nome}"),
    }
}

fn main() -> Result<()> {
    // Processa mensagens JSON-RPC 2.0, uma por linha.
    let args = Args::parse();
    let mut saida = std::io::stdout();
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let message: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                eprintln!("JSON-RPC inválido: {error}");
                continue;
            }
        };
        // Mensagens sem id são notificações e não têm resposta.
        let Some(id) = message.get("id").cloned() else {
            continue;
        };
        let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
        let response = match message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "initialize" => json!({"jsonrpc": "2.0", "id": id, "result": {
                "protocolVersion": params.get("protocolVersion").cloned().unwrap_or(json!("2025-06-18")),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "dtcc-fx", "version": "0.1.0"}}}),
            "ping" => json!({"jsonrpc": "2.0", "id": id, "result": {}}),
            "tools/list" => json!({"jsonrpc": "2.0", "id": id, "result": {"tools": ferramentas()}}),
            "tools/call" => {
                let nome = params
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let arguments = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let (texto, erro) = match executar(nome, &arguments, &args) {
                    Ok(value) => (value.to_string(), false),
                    Err(error) => {
                        eprintln!("{nome}: {error:#}");
                        (format!("{error:#}"), true)
                    }
                };
                json!({"jsonrpc": "2.0", "id": id, "result": {
                    "content": [{"type": "text", "text": texto}], "isError": erro}})
            }
            outro => json!({"jsonrpc": "2.0", "id": id,
                "error": {"code": -32601, "message": format!("método desconhecido: {outro}")}}),
        };
        writeln!(saida, "{response}")?;
        saida.flush()?;
    }
    Ok(())
}

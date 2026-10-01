/*
   Este binário serve a interface web do MVP e uma API HTTP de consulta.
   Consultar não exige chave de API: o provider só é criado quando uma
   classificação é pedida, numa thread bloqueante fora do runtime async.
*/

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use clap::Parser;
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::services::ServeDir;

#[derive(Debug, Clone, Parser)]
struct Args {
    // Caminhos, provider e bind necessários para iniciar a API e a interface.
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
    #[arg(long, default_value = "static")]
    static_dir: PathBuf,
    #[arg(long, default_value = "127.0.0.1:3000")]
    bind: String,
}

#[derive(Debug, Deserialize)]
struct GroupQuery {
    // Filtros de listagem recebidos na query string.
    par: Option<String>,
    estrutura: Option<String>,
    limite: Option<usize>,
}

type ApiResult = Result<Json<serde_json::Value>, (StatusCode, String)>;

fn api_error(error: anyhow::Error) -> (StatusCode, String) {
    // Converte erros internos em resposta HTTP com a mensagem completa.
    let message = format!("{error:#}");
    let status = if message.contains("não encontrado") {
        StatusCode::NOT_FOUND
    } else if message.contains("não existe") {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    (status, message)
}

async fn blocking<F>(f: F) -> ApiResult
where
    F: FnOnce() -> anyhow::Result<serde_json::Value> + Send + 'static,
{
    // SQLite e a chamada HTTP à IA são bloqueantes: rodam fora do runtime async.
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
        .map(Json)
        .map_err(api_error)
}

async fn info(State(args): State<Arc<Args>>) -> ApiResult {
    // Barra de título: amostra, contagens e provider configurado.
    blocking(move || {
        let conn = IATCC::db::open_existente(&args.banco)?;
        Ok(json!({
            "resumo": IATCC::db::summary(&conn)?,
            "provider": IATCC::ia::provider_name(args.provider.as_deref()),
            "estruturas": IATCC::tipos::ESTRUTURAS,
        }))
    })
    .await
}

async fn list(State(args): State<Arc<Args>>, Query(query): Query<GroupQuery>) -> ApiResult {
    // Lista grupos aplicando os filtros enviados pela interface.
    blocking(move || {
        let conn = IATCC::db::open_existente(&args.banco)?;
        IATCC::db::list_groups(
            &conn,
            query.par.as_deref().filter(|v| !v.is_empty()),
            query.estrutura.as_deref().filter(|v| !v.is_empty()),
            query.limite.unwrap_or(5000),
        )
    })
    .await
}

async fn detail(State(args): State<Arc<Args>>, Path(id): Path<String>) -> ApiResult {
    // Retorna grupo, classificação atual e histórico completo.
    blocking(move || {
        let conn = IATCC::db::open_existente(&args.banco)?;
        Ok(json!({
            "grupo": IATCC::db::get_group(&conn, &id)?,
            "classificacao": IATCC::db::latest_classification(&conn, &id)?,
            "historico": IATCC::db::history(&conn, &id)?,
        }))
    })
    .await
}

async fn brain(State(args): State<Arc<Args>>) -> ApiResult {
    // Expõe o brain concatenado para inspeção na tela (F2).
    blocking(move || {
        let brain = IATCC::ia::carregar_brain(&args.brain)?;
        Ok(json!({"hash": IATCC::ia::sha256(&brain), "brain": brain}))
    })
    .await
}

async fn classify(State(args): State<Arc<Args>>, Path(id): Path<String>) -> ApiResult {
    // Cria o provider agora (a chave pode ter sido definida depois) e grava o resultado.
    blocking(move || {
        let classifier =
            IATCC::ia::classifier_from_env(args.provider.as_deref(), &args.jev_criteria)?;
        let conn = IATCC::db::open_existente(&args.banco)?;
        let outcome = IATCC::ia::classificar_e_gravar(
            &conn,
            &id,
            &args.brain,
            &args.config,
            classifier.as_ref(),
        )?;
        match outcome.classificacao {
            Some(c) => Ok(serde_json::to_value(c)?),
            None => anyhow::bail!("a IA recusou a classificação"),
        }
    })
    .await
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Monta o estado, registra rotas e inicia o servidor HTTP.
    let args = Args::parse();
    if !args.banco.exists() {
        eprintln!(
            "aviso: {} não existe; rode `cargo run --release --bin preparar` antes de usar a tela",
            args.banco.display()
        );
    }
    let static_dir = args.static_dir.clone();
    let bind = args.bind.clone();
    let router = Router::new()
        .route("/api/info", get(info))
        .route("/api/grupos", get(list))
        .route("/api/grupos/:id", get(detail))
        .route("/api/grupos/:id/classificar", post(classify))
        .route("/api/brain", get(brain))
        .fallback_service(ServeDir::new(static_dir))
        .with_state(Arc::new(args));
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    println!("http://{bind}");
    axum::serve(listener, router).await?;
    Ok(())
}

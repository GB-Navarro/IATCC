/*
   Este arquivo encapsula o transporte HTTP usado pelos adaptadores de IA.
   A implementação Ureq faz chamadas reais, enquanto o trait permite injetar
   transportes fake nos testes; a função de retry centraliza o comportamento
   para respostas temporárias 429 e 5xx.
*/

use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Value,
}

pub trait HttpTransport: Send + Sync {
    fn request_json(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: Option<&Value>,
    ) -> Result<HttpResponse>;
}

pub type SharedTransport = Arc<dyn HttpTransport>;

#[derive(Clone)]
pub struct UreqTransport {
    agent: ureq::Agent,
}

impl UreqTransport {
    // Cria um cliente HTTP com timeout configurável para chamadas de produção.
    pub fn new(timeout: Duration) -> Self {
        Self {
            agent: ureq::AgentBuilder::new().timeout(timeout).build(),
        }
    }
}

impl Default for UreqTransport {
    fn default() -> Self {
        Self::new(Duration::from_secs(300))
    }
}

impl HttpTransport for UreqTransport {
    // Executa uma chamada JSON e preserva corpo e status, inclusive em erros HTTP.
    fn request_json(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: Option<&Value>,
    ) -> Result<HttpResponse> {
        let mut request = self.agent.request(method, url);
        for (name, value) in headers {
            request = request.set(name, value);
        }
        let response = match body {
            Some(body) => request.send_json(body),
            None => request.call(),
        };
        match response {
            Ok(response) => Ok(HttpResponse {
                status: response.status(),
                body: response.into_json().unwrap_or(Value::Null),
            }),
            Err(ureq::Error::Status(status, response)) => {
                let text = response.into_string().unwrap_or_default();
                let body = serde_json::from_str(&text).unwrap_or(Value::String(text));
                Ok(HttpResponse { status, body })
            }
            Err(error) => Err(error).context(format!("requisição HTTP {method} {url}")),
        }
    }
}

pub fn request_with_retries(
    transport: &dyn HttpTransport,
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: Option<&Value>,
    attempts: u32,
    backoff: Duration,
) -> Result<HttpResponse> {
    // Repete apenas falhas transitórias, devolvendo imediatamente erros definitivos.
    let attempts = attempts.max(1);
    for attempt in 0..attempts {
        let response = transport.request_json(method, url, headers, body)?;
        if response.status != 429 && response.status < 500 {
            return Ok(response);
        }
        if attempt + 1 < attempts {
            std::thread::sleep(backoff * 2_u32.pow(attempt));
        }
        if attempt + 1 == attempts {
            return Ok(response);
        }
    }
    bail!("tentativas HTTP esgotadas")
}

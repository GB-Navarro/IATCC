/*
   Este é o ponto de entrada da biblioteca do MVP IATCC. Ele declara os módulos
   de ingestão, cálculo, persistência, classificação e integração com providers.
   Os binários em src/bin reutilizam esta biblioteca para oferecer os comandos
   de captura, preparação, classificação, MCP e web.
*/
#![allow(non_snake_case)]

pub mod calculos;
pub mod classifier;
pub mod db;
pub mod dtcc;
pub mod golden;
pub mod grupos;
pub mod ia;
pub mod parse;
pub mod provider_http;
pub mod providers;
pub mod tipos;

pub use tipos::{Classificacao, Grupo, HedgeTest, Perna};

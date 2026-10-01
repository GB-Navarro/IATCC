# Estruturas

Definições (n_opt = número de opções; lados = moedas de call distintas):

- Opção única: n_opt = 1.
- Straddle: 2 opções, lados opostos, mesmo strike, mesmo vencimento.
- Strangle: 2 opções, lados opostos, strikes diferentes, mesmo vencimento, MESMA direção.
- Risk reversal: igual ao strangle, mas em direções OPOSTAS.
- Collar: risk reversal mais uma posição no ativo de notional cheio que as opções protegem.
- Spread vertical: 2 opções do mesmo lado, strikes diferentes, mesmo vencimento.
- Calendar spread: 2 opções do mesmo lado, mesmo strike, vencimentos diferentes.
- Spread diagonal: 2 opções do mesmo lado, strike e vencimento diferentes.
- Fracionamento de notional: a mesma opção repetida (mesmo lado, strike e vencimento).
  Não é estrutura.
- Lados opostos, vencimentos diferentes: forma sem nome de mercado.
- Seagull: 3 opções, 2 lados, 3 strikes distintos, mesmo vencimento.
- Strip/Strap: 3 opções, 2 lados, mesmo strike e vencimento (straddle 2:1).
- Ratio spread: 3 ou 4 opções do mesmo lado em 2 strikes com quantidades desiguais.
- Iron condor: 4 opções, 2 de cada lado.
- Butterfly: 4 opções do mesmo lado em 3 strikes, com o strike do MEIO repetido.
- Condor: 4 opções do mesmo lado em 4 strikes.
- Programa de hedge: 6 ou mais opções, um par por vencimento, até 3 strikes.
- Não identificado: qualquer forma fora da lista.

Hedges (NDF, Forward) no grupo não mudam o nome da forma; cite-os à parte.
Para 2 opções em lados opostos com strikes diferentes, a forma sozinha NÃO decide entre
strangle, risk reversal e collar: use o arquivo 03.

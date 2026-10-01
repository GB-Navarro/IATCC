# O que o dado mostra e o que não mostra

Mostra: produto, par de moedas, strike, vencimento, notionais, prêmio, plataforma,
horário da execução (em segundos) e se a perna faz parte de um pacote (Package indicator).

NÃO mostra:
- Quem comprou e quem vendeu. Por isso strangle, risk reversal e collar têm as mesmas
  duas opções no dado. Só o tamanho do hedge (arquivo 03) ajuda a separar.
- Operações à vista (spot) e exposições comerciais. Um collar protege um ativo; se esse
  ativo for spot, ele nunca aparece.
- Um identificador de pacote. O grupo foi montado por horário, plataforma e par, e pode
  conter pernas de operações diferentes.

Um notional "capado" (marcado como capado) é só um limite inferior: o valor real é maior.

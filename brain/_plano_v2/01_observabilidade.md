# Observabilidade

O tape público mostra produto, par, strike, vencimento, notional (possivelmente
capado), prêmio, plataforma, timestamp em segundos e Package indicator.

Ele não mostra package identifier, comprador, vendedor, payer, receiver ou a
intenção econômica. Portanto não se deve inferir posição comprada/vendida a
partir de palavras do FISN. A associação é uma hipótese baseada em coincidência
temporal e nos números publicados.

Um notional terminado em `+` é um limite inferior. Se qualquer perna relevante
estiver capada, o teste de hedge não é conclusivo e a resposta deve ser
`Ambiguous`.

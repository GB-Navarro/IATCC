# Straddle

Classifique como `Straddle` quando há duas opções vanilla ou NDO, uma call e
uma put identificáveis pela moeda-base, com o mesmo strike e o mesmo vencimento.
Essa regra de forma não depende de um NDF observado.

Se strike, vencimento ou lado estiver ausente, não complete a informação por
inferência. Use `Ambiguous`.

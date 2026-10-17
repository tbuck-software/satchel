# 0007 Kryptografie im Prozess statt über Binaries

Status: angenommen 2026-08-27

## Kontext

`utils/pgp_engine.rs` ruft `gpg`, `utils/smime_engine.rs` ruft `openssl`
als Binary. `UnimplementedRustEngine` ist ein Stub. Auf Android/iOS gibt
es keine Binaries. `openssl` als vendored Crate ist auf Mobile bereits
gelinkt.

## Entscheidung

- **S/MIME** über `openssl::cms` im Prozess — dieselbe Bibliothek, die
  schon da ist. Priorität vor PGP.
- **PGP** über `rpgp` (pure Rust). `gpg`-Engine bleibt als Option für
  Nutzer mit Hardware-Token, ist aber nicht mehr Pflicht.
- Private Schlüssel liegen pro Gerät (0009); Übertragung nur beim
  Pairing (0010).

## Konsequenzen

`brew install gnupg` verschwindet aus CI. Die Engines werden Traits mit
zwei Implementierungen, getestet gegen den Korpus (0002) mit
signierten/verschlüsselten Fixtures.

# QNC-shared

Zajedničke kocke QNC-a i QNC-AI-ja (korisnik 2026-11-12: "kocke su shared-zajedničke").

- `crates/`: kocke koje su bile iste u oba repoa (isti git tree), nisu GUI (`*-desktop`,
  `*-desktop-adapter`, `*-blocks`) i ovise samo o drugim zajedničkim kockama (247 kocki).
- `contracts/`, `catalogs/`, `seed/`: ugovori i podaci koje te kocke ugrađuju pri gradnji
  (`include_str!`) ili koje njihovi testovi čitaju. Zajednički su (korisnik: "1, ugovori zajednički").
- QNC i QNC-AI ih uzimaju kao path ovisnost: u svom `Cargo.toml`
  `qnc-x = { path = "../QNC-shared/crates/qnc-x" }`. Repoi stoje jedan uz drugi u istoj mapi.

Pravila:

- GUI je samostalan po aplikaciji (QNC svoj, QNC-AI svoj); AI je zajednički.
- Kad aplikacija traži promjenu zajedničke kocke, original ostaje ovdje, a ta aplikacija
  uzima promijenjenu kopiju u svoj `crates/` i u svom `Cargo.toml` pokazuje na nju.
- Promjena ovdje vrijedi za obje aplikacije: gradi se i testira u oba repoa.
- Zakon razvoja je `AGENTS.md` QNC-a.

Provjera: `cargo test --workspace --release` u ovoj mapi; zatim `cargo check --workspace` u QNC-u i QNC-AI-ju.

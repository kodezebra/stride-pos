# stride-pos

Rules, layout and the libs rev procedure live in `stride-libs/AGENTS.md` — read it first.
Product-only rules below.

- Own migrations in `apps/desktop/migrations/`, own `application_id`, own version in
  `apps/desktop/Cargo.toml`.
- Tag prefix `pos-v`. Tags come only from `main`, after green CI.
- `Cargo.lock` is committed; CI builds `--locked`.
- Never restate the libs rules here. If a rule applies to both products, it goes in
  `stride-libs/AGENTS.md`.

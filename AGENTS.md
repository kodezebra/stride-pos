# stride-pos

Rules, layout and the libs rev procedure live in `stride-libs/AGENTS.md` — read it first.
Product-only rules below.

## Git

Work locally. Every branch stays on this machine; only `main` is pushed.

- Branch locally, verify, merge locally into `main`, then `git push origin main`.
- Never push a feature/fix/chore/docs/release branch. No PRs from one.
- `main` moves by merge, never by a direct commit.

## Product

- Own migrations in `apps/desktop/migrations/`, own `application_id`, own version in
  `apps/desktop/Cargo.toml`.
- Tag prefix `pos-v`. Tags come only from `main`, after green CI.
- `Cargo.lock` is committed; CI builds `--locked`.
- Never restate the libs rules here; the Git block above is the deliberate exception.
  If any other rule applies to both products, it goes in `stride-libs/AGENTS.md`.

# Contributing

Use `dev` for experimental work and open pull requests into `dev`. Stable promotion goes from `dev` to
`main` after validation. Do not publish packages manually from a workstation.

Run before opening a pull request:

```sh
pnpm test
pnpm typecheck
cargo fmt --all -- --check
cargo test --workspace
```

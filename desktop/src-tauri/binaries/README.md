# externalBin staging directory

`tauri.conf.json`'s `bundle.externalBin` references `binaries/wgenty-code`.
Tauri resolves it as `wgenty-code-<target-triple>[.exe]` inside this directory.

Staged binaries are **generated** — do not commit them. Run
`desktop/scripts/bundle.sh` (or the CI desktop job) to produce them:

- `wgenty-code-aarch64-apple-darwin`
- `wgenty-code-x86_64-apple-darwin`
- `wgenty-code-x86_64-unknown-linux-gnu`
- `wgenty-code-x86_64-pc-windows-msvc.exe`

`.gitkeep` keeps the directory in the repo.

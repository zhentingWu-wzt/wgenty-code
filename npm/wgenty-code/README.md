# wgenty-code

High-performance Rust implementation of the Wgenty Code CLI.

This is the main npm package. It depends on a prebuilt platform-specific
binary (installed automatically via `optionalDependencies`) and launches it
transparently — no Rust toolchain required.

## Install

```bash
npm install -g wgenty-code
```

## Usage

```bash
wgenty-code --version
wgenty-code repl
wgenty-code query --prompt "hello"
```

See the [project README](https://github.com/zhentingWu-wzt/wgenty-code) for
full CLI documentation.

## Supported platforms

- linux-x64 / linux-arm64
- darwin-x64 (macOS Intel) / darwin-arm64 (Apple Silicon)
- win32-x64

If your platform is unsupported, you’ll see a clear error at launch. Install
the Rust toolchain and `cargo install wgenty_code` as a fallback.

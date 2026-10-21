fn main() {
    let dist = std::path::Path::new("web/dist");
    if !dist.exists() {
        std::fs::create_dir_all(dist).expect("create web/dist");
    }
    // 目录为空时放 .gitkeep 占位，保证 rust-embed 的 folder 属性在 cargo 元数据
    // 校验阶段不因目录缺失而失败；vite build 的 emptyOutDir 会清掉它，属正常
    // （下次 cargo 构建时本脚本仅重建占位，见设计 §6 "vite 交互"）。
    let empty = dist
        .read_dir()
        .map(|mut d| d.next().is_none())
        .unwrap_or(true);
    if empty {
        std::fs::write(dist.join(".gitkeep"), b"").expect("write .gitkeep");
    }
    println!("cargo:rerun-if-changed=web/dist");
}

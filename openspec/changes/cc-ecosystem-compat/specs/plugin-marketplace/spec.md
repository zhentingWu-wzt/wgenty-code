# plugin-marketplace

Marketplace 服务从 GitHub repos 实时获取插件索引。

## Requirements

- **REQ-PM-001**: 必须支持 `known_marketplaces.json` 配置格式（source.repo → installLocation）
- **REQ-PM-002**: 首次使用时自动 `git clone --depth 1` marketplace repo
- **REQ-PM-003**: 必须从 marketplace repo 中解析插件索引（`index.json` 或目录扫描）
- **REQ-PM-004**: `search(query)` 必须返回真实 marketplace 数据（替换硬编码示例）
- **REQ-PM-005**: `install(name)` 必须从 marketplace 找到插件源 URL 并 git clone 到标准 `cache/<publisher>/<plugin>/<version>/` 路径
- **REQ-PM-006**: 支持 marketplace 自动更新（`git pull` 检查新版本）
- **REQ-PM-007**: `install()` 必须处理 marketplace entry 的三种 `source` 类型——`LocalPath`（marketplace 本地子目录，如 `"./plugins/x"`）、`GitSubdir`（`{"source":"git-subdir","url":"...","path":"plugins/x","ref":"main"}` 格式）、`RemoteUrl`（`{"source":"url","url":"https://github.com/..."}` 格式）——每种都正确安装到 `cache/<publisher>/<plugin>/<version>/` 路径，解析使用 `#[serde(untagged)]` enum 自动匹配

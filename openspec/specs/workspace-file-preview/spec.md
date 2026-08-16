# workspace-file-preview Specification

## Purpose
为浏览器端提供已注册项目（含 worktree）内文件的浏览与只读预览能力：daemon 暴露带路径边界与大小约束的文件列表/内容读取 API，web 端以文件树与多类型预览面板消费。
## Requirements
### Requirement: 工作区目录列表

daemon SHALL 提供 `GET /api/v1/fs/entries?path=<dir>`：返回指定目录下条目的混合列表，每个条目 MUST 含 `name`、`is_dir`、`size`；目录条目 MUST 排在文件条目之前，同级按名称排序（不区分大小写）。列表 MUST 忽略隐藏文件（以 `.` 开头）与符号链接条目。

#### Scenario: 列出工作区根目录

- **WHEN** 请求 `GET /api/v1/fs/entries?path=<已注册项目根>`
- **THEN** 返回 200 与条目数组，目录在前、文件在后，条目含 `name`/`is_dir`/`size`

#### Scenario: 目录不存在

- **WHEN** 请求的 `path` 指向不存在的路径
- **THEN** 返回 404 与结构化错误信息

### Requirement: 文件内容读取

daemon SHALL 提供 `GET /api/v1/fs/file?path=<file>`：

- 文本文件 MUST 返回结构化响应：按行切分的内容、`version`（`mtime_ms` 与 `size` 组合）、编码为 UTF-8
- 二进制文件 MUST 按 mime 类型返回原始字节（`Content-Type` 由扩展名映射；无法识别时返回 `application/octet-stream` 与可探测的 `is_binary` 标记）
- 响应 MUST 含 `version` 版本信息，作为后续编辑能力（乐观并发写入）的基础

#### Scenario: 读取文本文件

- **WHEN** 请求指向路径边界内的 UTF-8 文本文件
- **THEN** 返回 200，含按行的内容数组与 `version`（mtime/size）

#### Scenario: 读取图片

- **WHEN** 请求指向路径边界内的 `.png` 文件且大小未超二进制上限
- **THEN** 返回 200，`Content-Type: image/png`，响应体为原始字节

### Requirement: 路径边界约束

`/api/v1/fs/entries` 与 `/api/v1/fs/file` MUST 将目标路径 canonicalize 后校验：仅当结果位于某个**已注册项目根**或**其 worktree 根**之内时才允许访问。越界路径（包括 `..` 逃逸、符号链接指向边界外、绝对路径直指系统目录）MUST 返回 403。此口径 MUST NOT 影响 `GET /api/v1/fs/dirs`（目录选择器的全盘浏览行为保持不变）。

#### Scenario: 路径穿越被拒

- **WHEN** 请求 `GET /api/v1/fs/file?path=/etc/passwd`
- **THEN** 返回 403，响应体含结构化错误，不泄露文件是否存在

#### Scenario: 符号链接逃逸被拒

- **WHEN** 边界内某符号链接指向边界外的文件，请求读取该链接路径
- **THEN** 返回 403

### Requirement: 读取大小上限

`/api/v1/fs/file` MUST 对文本文件设 1.5MB、对二进制文件设 5MB 的上限；超限 MUST 返回 413 与结构化错误（含 `size` 与 `limit`）。目录列表端点 MUST 对单次返回条目数设上限（默认 2000），超限时 MUST 在响应中标记截断。

#### Scenario: 超大文本文件

- **WHEN** 请求读取 2MB 的 UTF-8 文本文件
- **THEN** 返回 413，错误体含实际大小与上限值

### Requirement: 工作区文件树

web 端 MUST 在项目树的 task（workspace）节点下提供文件浏览入口：展开后懒加载该 workspace 根的目录列表，逐级展开子目录；目录条目可展开，文件条目可点击打开预览。文件树 MUST 与会话列表共存，不替换现有功能。

#### Scenario: 懒加载展开

- **WHEN** 用户首次展开某 task 节点下的文件树根
- **THEN** 前端请求该 workspace 根的目录列表并渲染一级条目；展开子目录时才请求该子目录

#### Scenario: worktree 隔离

- **WHEN** 某 task 绑定独立 worktree，用户在其文件树中浏览
- **THEN** 列表与预览均来自该 worktree 的工作区路径，而非项目主 checkout

### Requirement: 多类型预览面板

web 端 MUST 在主区域以 tab 呈现预览：

- 文本/代码：带行号显示，代码类扩展名 MUST 提供语法高亮（复用既有 shiki 集成）
- Markdown：MUST 渲染为富文本（含代码块高亮），并保留查看源码入口
- 图片（png/jpg/jpeg/gif/webp/svg）与 PDF：MUST 以 blob URL 加载原生呈现；SVG MUST 以图片方式呈现且不执行内嵌脚本
- 不支持的二进制与超限文件：MUST 显示友好提示（含文件大小），MUST NOT 白屏或崩溃

同一文件重复打开 MUST 复用既有 tab 而非新开。

#### Scenario: 打开代码文件

- **WHEN** 用户点击文件树中的 `.rs` 文件且大小在上限内
- **THEN** 主区新开（或复用）该文件的 tab，内容带行号与语法高亮

#### Scenario: 打开超限文件

- **WHEN** 用户点击 4MB 的文本文件
- **THEN** tab 内显示"文件过大"提示与实际大小，无内容渲染


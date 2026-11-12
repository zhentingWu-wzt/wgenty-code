# Tasks: multi-provider-models

## 1. 配置 schema 与迁移

- [x] 1.1 `ProviderGroup`/`ProviderModel` 结构与 serde（`models.providers`、`active_model`；`#[serde(default)]` 兼容旧文件）
- [x] 1.2 profiles→providers 迁移：按归一化 base_url host 聚合、内置 host→厂商名映射、endpoint 覆盖下放到模型条目、`active_profile`→`active_model` 映射；幂等
- [x] 1.3 迁移写回：生成 `settings.json.bak-<date>` 一次性备份后删除旧 `profiles` 字段
- [x] 1.4 `endpoint_for_tier` 改为跨厂商遍历 `providers` 模型 tier，`(provider, model)` 序确定性取首；`small` 遗留槽兼容并入
- [x] 1.5 API key 解析链：模型覆盖 > 厂商 key > 厂商 env（KIMI/ZHIPU/DEEPSEEK/ANTHROPIC/OPENAI）> 现有全局链
- [x] 1.6 旧格式 fixture 迁移测试 + tier 选择不变断言 + key 解析链测试

## 2. Session 级覆盖（daemon）

- [x] 2.1 `Session` 增 `model_override`/`permission_override` 字段（serde default；Plan 落盘归一化）
- [x] 2.2 `resolve_session_model(session_id) -> Settings`：全局 clone + overlay 合成（base_url/key/protocol 取自 ProviderGroup）；`run_session_turn` 接入
- [x] 2.3 `effective_permission(session_id)`：session 覆盖 > PermissionModeStore > Normal；GET/POST permission-mode handler 与 TaskTool spawn 快照改走该入口
- [ ] 2.4 `POST /model/switch` 扩展：带 `session_id` 只写覆盖不落盘；`GET /models`（list）带 `session_id` 时 active 按合成值；`ModelChanged`/`ModeChanged` 广播携带 `session_id`
- [ ] 2.5 覆盖重置端点/参数（清除 session 的模型或模式覆盖）
- [ ] 2.6 daemon 测试：同 project 双 session 隔离、fallback 链、重置、spawn 快照、Plan 不持久化

## 3. 动态模型目录

- [ ] 3.1 `ModelCatalogService`：OpenAI 兼容 `GET /models` 拉取、内置目录静态表、手工条目合并去重（configured 标记）、TTL 5min 缓存、失败回退
- [ ] 3.2 `GET /api/v1/providers`（组列表 + 每组合并目录）；Anthropic 协议组跳过拉取
- [ ] 3.3 目录服务测试（mock 端点：成功合并/失败回退/缓存命中/跳过协议）

## 4. Web：两级 chip 与 session 状态

- [ ] 4.1 `sessionManager`：`modelName`/`permissionMode` 全局单值改为 per-entry（effective + override 标记）；`ModelChanged`/`ModeChanged` 按 `session_id` 定向更新
- [ ] 4.2 `api/client.ts`：`listProviders`、`switchModel({provider, model, sessionId?})`、`resetSessionModel/Mode`
- [ ] 4.3 Composer 上方 chip 行：模型 chip（两级 picker：厂商→模型，「跟随默认」重置项）+ 模式 chip；busy 态收窄策略
- [ ] 4.4 StatusBar 瘦身：移除模型/模式控件，保留连接/审批/workspace/context
- [ ] 4.5 web 测试：两级选择只影响当前 session、重置回退、chip 随会话切换同步、StatusBar 无模型/模式

# Tasks: multi-provider-models

## 1. 配置 schema 与迁移

- [x] 1.1 `ProviderGroup`/`ProviderModel` 结构与 serde（`models.providers`、`active_model`；`#[serde(default)]` 兼容旧文件） <!-- comet-task:c428d088-0c84-46cb-839c-bb129d22ead8 -->
- [x] 1.2 profiles→providers 迁移：按归一化 base_url host 聚合、内置 host→厂商名映射、endpoint 覆盖下放到模型条目、`active_profile`→`active_model` 映射；幂等 <!-- comet-task:5b1750e0-b863-4745-90d9-303549b44fef -->
- [x] 1.3 迁移写回：生成 `settings.json.bak-<date>` 一次性备份后删除旧 `profiles` 字段 <!-- comet-task:d1688e7f-8322-4476-8285-5fd82fa4b972 -->
- [x] 1.4 `endpoint_for_tier` 改为跨厂商遍历 `providers` 模型 tier，`(provider, model)` 序确定性取首；`small` 遗留槽兼容并入 <!-- comet-task:3edc8cc8-ba64-41db-b944-8e4ad54f2576 -->
- [x] 1.5 API key 解析链：模型覆盖 > 厂商 key > 厂商 env（KIMI/ZHIPU/DEEPSEEK/ANTHROPIC/OPENAI）> 现有全局链 <!-- comet-task:33de6b00-3d3f-4f25-80f3-9fbec5a2b15a -->
- [x] 1.6 旧格式 fixture 迁移测试 + tier 选择不变断言 + key 解析链测试 <!-- comet-task:e25f4a2a-f162-40d2-95ee-6ac926a446dc -->

## 2. Session 级覆盖（daemon）

- [x] 2.1 `Session` 增 `model_override`/`permission_override` 字段（serde default；Plan 落盘归一化） <!-- comet-task:6ed41ead-46df-4615-8266-f194e3ab6269 -->
- [x] 2.2 `resolve_session_model(session_id) -> Settings`：全局 clone + overlay 合成（base_url/key/protocol 取自 ProviderGroup）；`run_session_turn` 接入 <!-- comet-task:1f25139c-db6a-4bed-90f1-693ae0d588c4 -->
- [x] 2.3 `effective_permission(session_id)`：session 覆盖 > PermissionModeStore > Normal；GET/POST permission-mode handler 与 TaskTool spawn 快照改走该入口 <!-- comet-task:48b0ae28-cbe9-4955-a7fe-39cf687ec189 -->
- [x] 2.4 `POST /model/switch` 扩展：带 `session_id` 只写覆盖不落盘；`GET /models`（list）带 `session_id` 时 active 按合成值；`ModelChanged`/`ModeChanged` 广播携带 `session_id` <!-- comet-task:eb17ce24-aea4-4c82-9d79-ecbcaa707147 -->
- [x] 2.5 覆盖重置端点/参数（清除 session 的模型或模式覆盖） <!-- comet-task:bad50e0a-7fdc-4484-a640-4971984a4244 -->
- [x] 2.6 daemon 测试：同 project 双 session 隔离、fallback 链、重置、spawn 快照、Plan 不持久化 <!-- comet-task:f191ac35-abeb-44c7-9b9c-3d1d9ed63161 -->

## 3. 动态模型目录

- [x] 3.1 `ModelCatalogService`：OpenAI 兼容 `GET /models` 拉取、内置目录静态表、手工条目合并去重（configured 标记）、TTL 5min 缓存、失败回退 <!-- comet-task:8e80d6ce-2b7c-4722-9e28-7b98a7d4981a -->
- [x] 3.2 `GET /api/v1/models` 原地合并（组列表 + 每组合并目录，不新增 `/providers`）；Anthropic 协议组跳过拉取 <!-- comet-task:21f95283-8be3-4448-b3d4-b79d6c8cb263 -->
- [x] 3.3 目录服务测试（mock 端点：成功合并/失败回退/缓存命中/跳过协议） <!-- comet-task:7f007b60-a899-4bfa-979f-664e1b6c9d54 -->

## 4. Web：两级 chip 与 session 状态

- [x] 4.1 `sessionManager`：`modelName`/`permissionMode` 全局单值改为 per-entry（effective + override 标记）；`ModelChanged`/`ModeChanged` 按 `session_id` 定向更新 <!-- comet-task:5082553f-2a36-497b-8a08-565d08a53c7f -->
- [x] 4.2 `api/client.ts`：`listModels(sessionId?)`、`switchModel({provider, model, sessionId?})`、`resetSessionModel`、`resetSessionMode` <!-- comet-task:264fd1ad-ea4e-466f-a635-41fce92820b0 -->
- [x] 4.3 Composer 上方 chip 行：模型 chip（两级 picker：厂商→模型，「跟随默认」重置项）+ 模式 chip；busy 态收窄策略 <!-- comet-task:7bb70575-7926-4d98-aa93-490c2de6e626 -->
- [x] 4.4 StatusBar 瘦身：移除模型/模式控件，保留连接/审批/workspace/context <!-- comet-task:d0a62ddc-9938-4b50-99b5-02da3fcab309 -->
- [x] 4.5 web 测试：两级选择只影响当前 session、重置回退、chip 随会话切换同步、StatusBar 无模型/模式 <!-- comet-task:770fdfbc-b37c-4144-9449-cbfc9b8b8e9f -->

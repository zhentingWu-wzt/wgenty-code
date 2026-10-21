# Design: Model Profiles as Single Source of Truth

## 核心决策

### D1: `main` 保留为字段,但改为派生缓存

删除 `main` 字段需要改动 ~15 个文件 40+ 处只读消费点(`api/mod.rs`、`tools/`、`tui/`、daemon),收益低风险高。改为:

```rust
pub struct ModelsConfig {
    /// 运行时缓存:active profile 的物化视图。不序列化。
    #[serde(default = "default_main", skip_serializing)]
    pub main: ModelEndpoint,
    #[serde(default, skip_serializing)]  // legacy,load 时被 migrate 消费
    pub small: Option<ModelEndpoint>,
    #[serde(default, skip_serializing)]  // legacy,load 时被 migrate 消费
    pub planner: Option<ModelEndpoint>,
    pub profiles: HashMap<String, ModelEndpoint>,
    pub active_profile: Option<String>,
    pub roles: ModelRoles,               // { planner: Option<key> }
    ...
}
```

不变式:任何经过 `load_from_disk`(文件分支)或 `switch_to_profile` 的 Settings,`models.main == models.profiles[active_profile]`。手工构造的 Settings(测试)不保证,但 `main` 本身可独立工作。

### D2: 迁移在 load_from_disk 的文件分支执行

`Settings::default()`(无文件首启)不迁移——避免把 env 派生的默认端点冻结进磁盘 profile。迁移幂等,`switch_model` 的 load→switch→save 链路自然把旧格式文件写为新格式。

### D3: `endpoint_for_tier` 保留 `small` 兜底

不迁移的手工构造 Settings(单测、嵌入场景)仍可依赖 `models.small`;迁移后该字段为 None,走 profile 路径。tier 匹配从 HashMap 任意序改为 key 字典序第一个,多个同 tier 时 warn。

### D4: `planner_settings()` 统一 planner 解析

```
roles.planner(key) → profiles[key]
└ 否则 legacy models.planner 端点(未迁移内存态)
  └ 否则 None(用 main)
```

### D5: `Settings::set` 路径重映射(兼容 config-key-compat)

| 旧 key 前缀 | 重映射为 |
|---|---|
| `models.main` / `models.main.*` | `models.profiles.<active 或 "main">`(set_at 会补建对象) |
| `models.small` / `models.small.*` | `models.profiles.<首个 light-tier key 或 "small">` |
| `models.planner` / `models.planner.*` | `models.profiles.<roles.planner 或 "planner">` |

patch 落在既有 profile 完整对象上 → `models.main.name = X` 不再丢 base_url 等 siblings(旧行为依赖磁盘恰好有完整 main 对象)。

### D6: `switch_to_profile` 非破坏

```rust
let endpoint = profiles.get(profile)?.clone();
active_profile = Some(profile);
main = endpoint;   // 缓存同步
```

原 main 永远以 profile 形式存在,可切回。

## 风险与缓解

- **旧二进制读新格式失败**(`main` 旧代码必填):提案已声明单向迁移;`settings.json.bak.*` 与 git 可恢复。
- **`small_model_settings` 语义变化**:从"5 字段覆盖"变为"完整端点替换"。差异仅 per-endpoint `context_window`/`temperature` 开始生效——本就是文档声明的优先级行为。
- **测试手工构造 Settings 不走迁移**:`endpoint_for_tier`/`planner_settings` 保留 legacy 兜底(D3),现有测试语义不变。

## 不做

- 不合并 `models.routing` 与 `agent.rlm.auto_routing`(独立关注点,另行处理)。
- 不给 medium tier 增加专属槽位(medium ≡ main 是显式设计)。
- web 端零改动(picker 数据面不变)。

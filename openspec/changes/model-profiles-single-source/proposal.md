# Model Profiles as Single Source of Truth

## Why

当前 `models` 配置用三种机制表达"用哪个模型",语义互相重叠、边界靠代码隐式约定:

1. **`main` 双重身份**:既是用户配置入口,又是 `/model` 切换的**破坏性覆写槽位**(`switch_to_profile` 把 profile 整体拷入 `main`,原值丢失)。切走后无法切回;`active_profile` 与 `main` 可能失配,`list_models` 里的"按模型名反推 active"逻辑(`src/daemon/handlers/system.rs:129-152`)就是在给这个不一致打补丁。
2. **"便宜模型"三种表达**:`models.small`(legacy 槽位)、`tier: light` 的 profile、都没有时落回 `main`,优先级藏在 `endpoint_for_tier` 里。
3. **`planner` 合并逻辑重复三处**:`small_model_settings()`(`src/config/mod.rs:181`)、`tui/app/turn.rs:211` 与 `:393` 两段几乎相同的逐字段覆盖代码,平行实现同一语义。

后果:用户配置两个"便宜模型"不知道谁生效;`/model` 列表与磁盘状态可能不一致;`config set models.main.name` 会把 main 对象整个替换成 `{name:X}`(siblings 丢失依赖旧文件恰好有完整 main 对象)。

## What Changes

- **profiles 是唯一磁盘事实源**。每个可用的模型都是 `models.profiles` 里的命名条目;`active_profile` 指向当前生效者。
- **`main` 变为运行时派生缓存**:`#[serde(skip_serializing)]`,load/switch 时从 `active_profile` 物化,消费者(`ApiClient`、TUI、daemon)读 `models.main` 的代码不变。
- **`small`/`planner` 退化为角色引用**:
  - legacy `models.small` 加载时迁移为隐式 `tier: light` profile;
  - 新增 `models.roles.planner: Option<profile_key>`,legacy `models.planner` 端点加载时迁移为 profile + 角色引用;
  - `endpoint_for_tier` 保留对未迁移内存态 `small` 的兜底(测试/手工构造的 Settings 不经过 load)。
- **`switch_to_profile` 非破坏化**:只改 `active_profile` + 同步缓存,不再覆写丢失原 main。
- **`list_models` 简化**:active 即 `active_profile`,删除按名字反推的启发式。
- **`Settings::set` legacy 路径重映射**:`models.main.*` → `models.profiles.<active>.*`、`models.small.*` → light profile、`models.planner.*` → planner 角色;且因 patch 落在既有 profile 对象上,不再丢失 siblings 字段。
- **去重**:新增 `Settings::planner_settings()`,替换 `tui/app/turn.rs` 两处重复块。
- **tier 解析确定性**:同一 tier 多个 profile 时按 key 字典序取第一个并 warn(原 HashMap 迭代序不确定)。

### 迁移语义(加载时,幂等)

| 旧字段 | 迁移结果 |
|---|---|
| `main`,profiles 为空或不匹配 | 合成 profile `"main"`(键冲突时自动加后缀),`active_profile = "main"` |
| `main`,存在同名 profile | `active_profile` 指向该 profile,不合成 |
| `active_profile` 失效(指向已删 profile) | 同上两条规则重新解析 |
| `small`,且无 light-tier profile | 合成 profile `"small"` + `tier: light` |
| `small`,已有 light profile | 丢弃并 warn(原本就被遮蔽) |
| `planner` | 同名 profile 复用,否则合成 `"planner"`;`roles.planner` 指向它 |

保存后磁盘为新格式(不再含 `main`/`small`/`planner` 键)。**旧版本二进制无法读取新格式**(`main` 在旧代码中是必填字段)——降级需手工恢复,见 Impact。

## Capabilities

### Modified Capabilities

- `config-key-compat`: `models.main.*` / `models.small.*` / `models.planner.*` 的 `config set` 路径重映射到 profiles/roles 等价路径,行为对用户透明。

### New Capabilities

- `model-profiles-single-source`: profiles + active_profile + roles 的解析不变式、legacy 迁移规则、tier 角色解析确定性。

## Impact

- **数据流**:`Settings::load_from_disk`(文件分支)追加 `migrate_legacy()`;`switch_model` 的 load→switch→save 链路自然把旧格式文件升级为新格式。
- **行为改进**:仅配置过 `small` 的用户,迁移后 `has_tier(Light)` 为 true,自动路由(原来只有显式 `use_small_model` 生效)开始工作;`/model` picker 对只有 `main` 的用户不再为空(显示合成 profile)。
- **不兼容点**:磁盘格式单向升级;`endpoint_for_tier(Light)` 返回完整端点(含 per-endpoint `context_window`/`temperature`),而非旧 `small_model_settings` 只覆盖 5 个字段的语义。
- **不动**:`ModelEndpoint` 字段、`models.routing`、`agent.rlm.auto_routing` AND 语义、RLM pipeline/task 的调用面、web 端。

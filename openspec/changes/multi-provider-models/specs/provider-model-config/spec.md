# provider-model-config

## Purpose

以厂商为单位的模型配置结构：一个厂商一份连接信息（base_url + api_key）加模型列表，消除多模型场景下逐 profile 重复填写连接信息的负担，并为两级选择与动态目录提供数据基础。

## ADDED Requirements

### Requirement: 厂商分组配置结构

`settings.json` 的 `models.providers` SHALL 以厂商 id 为 key，每厂商包含 `base_url`、可选 `api_key`、可选显示名，以及该厂商下的模型列表。每个模型条目 SHALL 至少包含发送给 API 的模型名，可选 tier（light/medium/heavy）与其他端点级覆盖。

#### Scenario: 多厂商多模型配置

- **WHEN** 用户配置两个厂商（如 kimi、zhipu），各含 2 个模型
- **THEN** 每个厂商只存一份 `base_url` 与 `api_key`，旗下模型不重复携带连接信息

#### Scenario: 端点级覆盖优先于厂商默认

- **WHEN** 某模型条目显式设置了 `base_url` 或 `api_key` 覆盖
- **THEN** 该模型请求使用覆盖值；未设置覆盖的模型使用厂商级值

### Requirement: API key 解析顺序

每厂商的 API key SHALL 按以下顺序解析：模型条目覆盖 > 厂商级 `api_key` > 厂商对应的环境变量（如 `KIMI_API_KEY`、`ZHIPU_API_KEY`、`DEEPSEEK_API_KEY`、`ANTHROPIC_API_KEY`）> 现有全局回退链。

#### Scenario: 环境变量覆盖配置文件

- **WHEN** 配置文件中厂商 `api_key` 为空但环境变量 `KIMI_API_KEY` 已设置
- **THEN** 该厂商所有模型的请求鉴权使用环境变量值

### Requirement: 旧 profiles 自动迁移

系统启动加载配置时 SHALL 将旧 `models.profiles` 平铺结构自动迁移为厂商分组：按 `base_url`（归一化 host）聚合为厂商组，保留全部模型、tier、覆盖字段；迁移 SHALL 幂等（重复启动不产生重复组或重复模型）。迁移后 SHALL 保留当前 `active_profile` 指向的模型语义不变。

#### Scenario: 平铺 profiles 迁移

- **WHEN** 旧配置含 3 条 profiles，其中 2 条共享同一 `base_url`（同厂商）
- **THEN** 迁移后为 2 个厂商组，共享 `base_url` 的 2 条合并为 1 组下的 2 个模型，`active_profile` 指向的模型仍为当前生效模型

#### Scenario: 迁移不可逆但无破坏

- **WHEN** 迁移后的配置写回磁盘
- **THEN** 用户手工新增的旧结构 `profiles` 条目在下次启动时再次被迁移合入，不丢失

### Requirement: tier 路由跨厂商归并

tier（light/medium/heavy）SHALL 在模型条目上声明；tier 路由选择模型时 SHALL 跨所有厂商的声明模型归并（同一 tier 多候选时确定性选择，规则与现有 alphabetically-first 语义一致）。

#### Scenario: 跨厂商同 tier 多候选

- **WHEN** kimi 组下模型 A 与 zhipu 组下模型 B 均声明 tier=light
- **THEN** light 档位路由确定性选中其中之一，不因 HashMap 顺序而抖动

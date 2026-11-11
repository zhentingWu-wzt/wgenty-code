# model-catalog-fetch

## Purpose

按厂商动态拉取可用模型列表，让两级选择展示厂商实际提供的模型而不依赖用户手工枚举；拉取失败时以内置目录与手工配置兜底，不阻塞模型切换。

## ADDED Requirements

### Requirement: 厂商模型目录拉取

系统 SHALL 支持按厂商拉取可用模型列表（OpenAI 兼容 `GET /models` 端点），拉取结果与手工配置的模型条目合并展示；手工配置条目始终可见且标记为已配置。未提供 list-models 端点的厂商（如 Anthropic messages API）SHALL 跳过动态拉取，仅展示内置目录与手工配置。

#### Scenario: 拉取成功合并展示

- **WHEN** 厂商 list-models 端点返回 5 个模型，手工配置含其中 1 个及另 1 个端点上不存在的模型
- **THEN** 选择器展示 6 个条目（去重合并），手工配置的 2 个带"已配置"标记

#### Scenario: 不支持列表端点的厂商

- **WHEN** 厂商协议为 Anthropic messages（无 list-models）
- **THEN** 不发起拉取，选择器仅展示内置目录与手工配置条目

### Requirement: 拉取失败回退

模型目录拉取失败（网络错误、4xx/5xx、响应解析失败）SHALL 回退到内置目录（已知厂商的常用模型清单）加手工配置条目，并 SHALL 在合理 TTL 内缓存最近一次成功结果优先使用；失败不得阻塞选择器打开，也不得阻塞向手工配置模型的切换。

#### Scenario: 拉取失败不阻塞

- **WHEN** 厂商端点超时或返回 500
- **THEN** 选择器立即以内置目录+手工配置打开，切换到任一已知模型的行为不受影响

#### Scenario: 缓存优先

- **WHEN** 上次拉取成功后 5 分钟内再次打开选择器且本次拉取失败
- **THEN** 展示缓存的上次成功结果

### Requirement: 内置目录可维护

已知厂商（kimi、zhipu、deepseek、anthropic、openai 等）SHALL 有内置常用模型目录，随版本发布更新；内置目录条目与拉取/手工条目并存时按模型名去重。

#### Scenario: 内置目录兜底首用

- **WHEN** 用户首次配置某内置厂商且拉取尚未执行或失败
- **THEN** 选择器仍能展示该厂商的常用模型供选择

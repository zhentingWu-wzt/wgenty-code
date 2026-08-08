# Tasks: gui-session-management

## 1. 会话列表面板

- [x] 1.1 会话列表面板接入骨架面板挂载点，展示标题/时间等元信息 —— 复用 web/src/features/panels/SessionsPanel.tsx + components/SessionsBrowserModal.tsx
- [x] 1.2 新建/切换/删除会话（删除需确认，处理当前会话被删除的切换逻辑）—— 复用 web/src/features/sessions/NewSessionModal.tsx + sessionManager.ts

## 2. 搜索与恢复

- [x] 2.1 会话关键词搜索：DaemonClient.searchSessions（GET /sessions/search?q=）+ SessionsPanel 搜索输入框（300ms debounce，搜索时不分 active/archived）
- [x] 2.2 切换会话加载完整历史，长历史分页/懒加载 —— 复用 web/src/agent/sessionLoad.ts (sessionMessagesToDisplay 无损转换)
- [x] 2.3 恢复会话后可继续对话（上下文正确衔接）—— 复用 sessionStore.pushLoadedMessage + sessionRunner

## 3. checkpoint 与 undo-turn

- [x] 3.1 checkpoint 列表视图（各 turn 变更摘要）—— 复用 web/src/features/panels/CheckpointsPanel.tsx
- [x] 3.2 undo-turn 操作：二次确认交互 + 结果反馈 —— 复用 CheckpointsPanel.undo (client.undoTurns)

## 4. 验证

- [ ] 4.1 验收：列出/搜索/切换历史会话并恢复上下文继续对话
- [ ] 4.2 验收：对指定 turn 执行 undo 并确认回滚生效、取消不生效

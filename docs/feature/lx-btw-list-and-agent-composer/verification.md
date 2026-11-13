# 验证报告：BTW / Agents 列表-详情与 composer

> 状态：**自动检查已通过，手工项待用户执行**。
>
> 验证分支：`feat/lx-btw-list-and-agent-composer`
> 验证提交：`c107fb8c fix(task): correct panel cursor and add target loading`
> 验证日期：2026-11-13

## 1. 测试环境与种子数据

| 项 | 内容 |
| --- | --- |
| 工作区 | `/Users/yonah/projects/agent/lx-tui/.worktrees/lx-btw-list-and-agent-composer` |
| 会话 | `BTW/Agents 验证种子`（`sess-18e77c24cfc4f778-0`） |
| 主问题 | 6 个（验证问题 A–E + 子代理种子请求） |
| 子代理 | 1 个已完成：`Turing [explorer]`，标题「验证子代理」 |
| BTW 线程 | CLI 无创建子命令；首次手工验证时用 `/btw <问题>` 创建 |

启动：

```bash
cd /Users/yonah/projects/agent/lx-tui/.worktrees/lx-btw-list-and-agent-composer
cargo run
```

进入 TUI 后执行 `/resume`，选择「BTW/Agents 验证种子」。若需测试 BTW 线程，先执行 `/btw 请用一句话解释验证问题 A`。

## 2. 入口与列表

| # | 输入内容 | 验证步骤 | 期望结果 | 结果 |
| --- | --- | --- | --- | --- |
| A1 | `/btw` | 主输入框输入后 Enter | 打开 BTW 列表；最新 Q 高亮并贴底；面板高 8 行；上方显示该 Q 对话 | [ ] |
| A2 | `/btw 请用一句话解释验证问题 A` | Enter | 进入最新 Q 编辑态并提交；发送后仍留在编辑态；Esc 回列表可见新线程 | [ ] |
| A3 | `/subagents` | Enter | 打开 Agents 列表；可见 `Turing [explorer]`；上方显示其详情 | [ ] |
| A4 | 无 | 列表态、搜索为空时按 `←/→` | 在 Processes / Agents / BTW 间轮转；退出编辑态、清空搜索；高亮保留 | [ ] |
| A5 | 无 | 打开 BTW / Agents 面板 | tab 行 + 搜索行 + 5 行列表；滚动条只覆盖列表；Processes 仍为 6 行列表 | [ ] |
| A6 | 无 | 观察两页列表行 | BTW 为 `QA n · 标题` + 状态/`● side thread`；Agents 保持树形前缀、身份、状态、时长、标题 | [ ] |
| A7 | 无 | BTW 列表按 `Tab` / `Shift+Tab`；Agents 列表按 `Tab` | BTW 不动作；Agents 在 `all → active → done` 轮转 | [ ] |

## 3. 搜索、导航与鼠标

| # | 输入内容 | 验证步骤 | 期望结果 | 结果 |
| --- | --- | --- | --- | --- |
| B1 | `验证问题 C` | 在 BTW 搜索行直接输入 | 只保留匹配 Q；高亮与详情切到第一项；`Shift+Enter` 可进入该 Q；清空后保持当前项 | [ ] |
| B2 | `Turing` / `explorer` / `验证子代理` / `sub-18e77c301e508d70-a` | 在 Agents 搜索行逐个输入 | 分别按昵称、角色、标题、agent_id 命中；大小写不敏感 | [ ] |
| B3 | `zzz` | 在 BTW 和 Agents 搜索行输入 | 列表显示 `no matching questions` / `no matching agents`；详情保留最后选中内容 | [ ] |
| B4 | 无 | 列表态按 `↑/↓` | 高亮立即移动并切换详情；到两端停住，不关闭面板 | [ ] |
| B5 | 无 | 列表态按 `Enter`、`Ctrl+Enter`、`Shift+Enter` | `Enter` / `Ctrl+Enter` 不动作；`Shift+Enter` 进入编辑态 | [ ] |
| B6 | `abc` | 搜索非空时按 `←/→`，再清空后按 `←/→` | 非空时移动搜索光标；清空后切换 tab | [ ] |
| B7 | `abc` | 列表态按 `Esc` 两次 | 第一次清空搜索；第二次关闭面板 | [ ] |
| B8 | 无 | 详情区 `PgUp/PgDn`；列表/详情滚轮 | 详情 10 行/页；详情滚轮 3 行/格；列表滚轮移动列表；搜索行滚轮不穿透 | [ ] |
| B9 | 无 | 单击行、双击行、点击搜索行、拖动滚动条 | 单击选中并预览；双击进入编辑；搜索行定位文本光标；滚动条与键盘同源 | [ ] |
| B10 | 无 | 切 tab；关闭后重开 | 切 tab 退出编辑、清空搜索、保留高亮；重开为列表态、搜索为空、高亮保留 | [ ] |
| B11 | 无 | 高亮有线程和无线程的 Q | 有线程显示消息；无线程显示 `No side conversation yet`，且不提前写库 | [ ] |
| B12 | 无 | 高亮 Q 后回主会话发送新问题，再开 `/btw` | 新 Q 不抢走高亮；原 Q 仍选中（若未被删除） | [ ] |

## 4. 编辑态与发送

| # | 输入内容 | 验证步骤 | 期望结果 | 结果 |
| --- | --- | --- | --- | --- |
| C1 | 无 | BTW 列表选中 Q 后 `Shift+Enter` | tab/搜索/列表隐藏；1 行草稿 composer 高 3 行，3 行草稿高 5 行；左上角标记 `BTW`；右侧显示 `QA n · 标题 · To: ...` | [ ] |
| C2 | `请继续说明` | 输入后 Enter | 发送成功并留在编辑态；输入清空；回答流式显示；Esc 回列表可见线程状态 | [ ] |
| C3 | `草稿 ABC` | 输入后 Esc 回列表，再 `Shift+Enter` | 草稿原样保留 | [ ] |
| C4 | 无 | 编辑态运行中按 `Ctrl+D` | 停止当前 BTW 线程 / 当前 agent；仍留在编辑态；草稿保留 | [ ] |
| C5 | `@` | BTW 编辑态输入 `@`；Agents 编辑态输入 `@` 和 `/` | BTW 出现提及面板；Agents 不出现提及/命令面板，按普通文本输入 | [ ] |
| C6 | 无 | Agents 选中 Turing 后 `Shift+Enter` | 左上角显示 `Subagent`（不是 `Agent`）；右侧显示 `To: Turing [explorer]`；无 tab/搜索/列表 | [ ] |
| C7 | `请继续` | Agents 编辑态 Enter；Esc 回列表；再次进入 | 发送后留编辑态并清空；Esc 后草稿保留；再次进入恢复草稿 | [ ] |
| C8 | 无 | 停止一个运行中的 agent 后按 `Shift+Enter` | 不进入编辑态，toast 显示 `Cannot send input to a closed subagent` | [ ] |
| C9 | 无 | 把窗格缩到 40 列以下，看底边提示 | 提示从右往左整项丢弃；不截半条、不破边框 | [ ] |

## 5. Loading、光标与文案修复

| # | 输入内容 | 验证步骤 | 期望结果 | 结果 |
| --- | --- | --- | --- | --- |
| D1 | `/btw` | 打开列表后输入、清空、上下移动 | 硬件光标只在搜索行；详情区绝不出现可停留的输入光标 | [ ] |
| D2 | 无 | 主历史区拖选文字后执行 `/btw` | 主输入框与历史区残留选区被清除；无单格反显/残影 | [ ] |
| D3 | 无 | 进入 Agents 编辑态看左上角 | 标记为 `Subagent`；不出现 `Agent` | [ ] |
| D4 | 无 | BTW 编辑态发送问题后观察 composer 上方 | loading 行位于 composer 上方、与主 agent 同相对位置；不在 composer 内部 | [ ] |
| D5 | 无 | Agents 编辑态给 Turing 发消息后观察 | 当前 run `running` 时，composer 上方出现 loading；结束后消失 | [ ] |
| D6 | 无 | 观察 loading 动画与颜色 | 扫描窗口动画正常；只用同一前景色，靠 bold/dim 区分；无四色渐变 | [ ] |
| D7 | 无 | loading 时 Esc 回列表，再 `Shift+Enter` | 列表态不显示目标 loading；回编辑态后继续显示 | [ ] |
| D8 | 无 | 列表搜索输入、清空、上下移动 | 无 hardware cursor 残留在详情区；光标位置始终跟随搜索行 | [ ] |

## 6. 详情共享、滚动与渲染

| # | 输入内容 | 验证步骤 | 期望结果 | 结果 |
| --- | --- | --- | --- | --- |
| E1 | 无 | 高亮无线程 Q；再首次进入有线程 Q | 无线程显示 `No side conversation yet`；首次加载显示呼吸圆点 + `loading side conversation…` | [ ] |
| E2 | 无 | 详情上翻后发送新消息或等待流式输出 | 视口保持阅读位置，不跳顶；出现 `↓ jump to bottom`；点击/按键可回底 | [ ] |
| E3 | 无 | 展开/折叠工具卡片，点击代码块复制 | BTW 与 Agents 行为一致；复制按钮命中 `[❐]` | [ ] |
| E4 | 无 | 在两页详情拖选文本并复制 | 选区与复制正常；不会串到搜索框或 composer | [ ] |
| E5 | 无 | 调整窗格宽度/高度 | 重新折行、滚动条范围更新，边框与列表不破 | [ ] |

## 7. 删除能力与回归

| # | 输入内容 | 验证步骤 | 期望结果 | 结果 |
| --- | --- | --- | --- | --- |
| F1 | `/btw` | 主输入框输入 `/`，查看命令面板 | 只有 `/btw`，不再出现 `/btwList` | [ ] |
| F2 | `/btwList` | 主输入框输入并 Enter | 不被识别为命令，不打开旧选择面板 | [ ] |
| F3 | `/` | BTW 编辑态输入 `/` | 不出现 BTW 专属 `/` 命令面板 | [ ] |
| F4 | 无 | Agents 列表按 `Ctrl+A`；再按 `Ctrl+Shift+A` / `Cmd+Shift+A` | 不再停止全部 agent；`Ctrl+A` 到行首，组合键全选搜索文本 | [ ] |
| F5 | 无 | Agents 列表按 `Ctrl+D` | 只停止当前可见高亮 agent，不影响其他 agent | [ ] |
| F6 | 无 | 观察 Agents 列表行 | 没有内联追问输入；只能 `Shift+Enter` / 双击进入 composer | [ ] |
| F7 | 无 | 切到 Processes 页签 | 仍为 6 行列表、无搜索行；原有 Esc / 滚轮行为不变 | [ ] |

## 8. 自动检查

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all` | [x] 通过 |
| `cargo clippy --all-targets -- -D warnings` | [x] 通过 |
| `cargo test` | [x] 通过 |
| 光标回归测试 `btw_list_hardware_cursor_stays_on_the_search_row` | [x] 通过 |
| BTW/Agents 目标 loading 与单色断言 | [x] 通过 |

## 9. 备注

- CLI 种子会话已写入当前 worktree 的隔离开发库；主 checkout 的 `~/.lx-t-dev/db/lx.db` 不受影响。
- CLI 没有创建 BTW 侧线程的子命令；BTW 线程需在 TUI 内用 `/btw` 创建，属于手工验证前置步骤。
- 本分支已合并到 `dev`（合并提交 `b54a0031`）；合并前自动检查通过，终端交互项仍建议用户运行 `cargo run` 复核。

## 10. Review 修复后的补充检查

- 搜索非空时硬件光标仍落在文本真实光标列，不因可见文本宽度偏移；已补单测。
- Agents composer 长草稿按窗格宽度软换行并撑高到 5 行；已补单测。
- BTW 搜索后 `Shift+Enter` 直接进入第一项；切换 Q/搜索/剪切后旧选区清除。
- Processes 首行滚轮、编辑态 composer 滚轮、编辑态列表滚动条命中均已补回归断言。
- 目标 loading 在 BTW 工具/思考阶段显示对应文案，而不是恒为 `Working...`。
- 详见 [review.md](review.md)。

## 11. 合并记录（2026-11-13）

- 功能分支 `feat/lx-btw-list-and-agent-composer` 已合并到 `dev`，合并提交 `b54a0031`。
- 合并时主工作区只有旧版未跟踪的 feature 文档，已备份到临时目录后由分支版本取代；代码与文档均无冲突，`ort` 自动合并成功。
- 合并后执行：`cargo fmt --all -- --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`。
- 结果：`dev` 5347 passed / 0 failed / 6 ignored；clippy 与 fmt 通过。
- 合并后的 `dev` 仍建议用户运行 `cargo run` 复核终端 `Shift+Enter`、鼠标双击与 loading 观感。

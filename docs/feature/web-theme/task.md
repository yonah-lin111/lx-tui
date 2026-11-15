# Web 主题与顶栏调整：执行清单

- [x] `Config` 增加 `web_theme`（默认 `default`，校验 `default|glass`），JSON 读写与 `settings.json` 落盘。
- [x] `/api/config` 的 GET/PUT 与 `/api/state` 暴露 `web_theme`；只改主题不覆盖布局字段。
- [x] 页面响应注入 `<html data-theme="...">`，刷新首屏即为持久化主题。
- [x] 顶栏移除 `lx-tui` 文字，只留像素 logo；tab 紧邻 logo。
- [x] 顶栏最右侧新增主题 select（Default / Glass），自绘下拉，固定宽度。
- [x] 主题切换即时生效；保存失败回滚并 toast；迟到初始读取不覆盖手动选择。
- [x] 新增 `assets/theme.css`：iOS 液态玻璃令牌、渐变壁纸、SF Pro、圆角玻璃卡片、胶囊按钮/表格/提示层。
- [x] 动效对齐参考项目：tab 弹簧滑块、视图仅淡入、下拉菜单缩放入场、卡片/按钮 hover 过渡、reduced-motion 降级。
- [x] 顶栏与 range/provider 小 tab 统一胶囊滑块：hover 预览、离场回落选中项、文字颜色仅在选中态变化。
- [x] 玻璃切换性能：旧视图立即隐藏，新视图仅 opacity 淡入，移除整页 blur/translate 与离场层。
- [x] Default 主题零回归：所有玻璃规则限定在 `html[data-theme="glass"]`。
- [x] 测试：配置 round-trip/非法值、API 持久化与页面注入、资源路由与主题脚本断言。
- [x] 文档：`docs/standards/html-assets.md` 与 `docs/feature/web-theme/design.md`。
- [ ] 用户在真实浏览器确认观感与主题持久化。

# HTML 资源规范

本文定义 `assets/` 目录下的浏览器前端资源的目录划分、文件格式、提供方式与编辑约束。设置页等浏览器前端以独立静态文件维护，经 `include_str!` 编译进二进制，并按独立路由提供给浏览器。

## 定位与范围

- `assets/` 存放**浏览器前端资源**（HTML / CSS / JS）与其它需要原样嵌入二进制的静态文件。
- 前端资源与 TUI 渲染无关：页面通过本地回环服务的 JSON API 与 `app` 通信，不由 ratatui 绘制。
- 不使用 Rust 模板引擎生成 HTML，不引入 WASM 前端；资源以原生 HTML/CSS/JS 编写，保持零构建工具链。
- 未使用的资源不提前创建；`assets/` 按需增长。

## 目录与命名

```text
assets/
  settings.html        设置页结构（src/settings/service/page.rs 经 include_str! 加载）
  settings.css         设置页样式
  settings.js          设置页核心脚本（标签页、General、Commands、toast 与 select 组件）
  settings_models.js   Models 视图脚本（全局草稿、模型卡片、模态与思考等级编辑）
```

- 文件名使用小写 `snake_case`，一名一页；同一页面的 HTML / CSS / JS 共用词干（`<name>.html` / `.css` / `.js`）。
- 单页脚本超过 1000 行时按职责拆分：主 `<name>.js` 保留页面骨架与全局组件，其余视图拆为 `<name>_<section>.js`，均在 `page::asset` 登记并按 HTML 中的顺序加载。
- **结构、样式、脚本分文件维护**：HTML 不含内联 `<style>` / `<script>` 正文，改为 `<link rel="stylesheet" href="/<name>.css">` 与 `<script src="/<name>.js"></script>`。
- 若确需嵌入非 HTML 资源（图标、字体等二进制），单独放置并以扩展名区分；二进制用 `include_bytes!`，文本用 `include_str!`。

## 提供方式

- 资源经**独立路由**提供，与页面同源：`GET /settings.css`、`GET /settings.js`。浏览器对每个资源独立发起请求并可分别缓存。
- 每个资源在 Rust 侧以 `include_str!` 嵌入，注册进静态资源表（`page::asset`：路径 → `(Content-Type, body)`），由路由分派；新增资源只需在表中加一行。
- `Content-Type` 必须正确：`.css` 用 `text/css; charset=utf-8`，`.js` 用 `text/javascript; charset=utf-8`，`.html` 用 `text/html; charset=utf-8`。
- 资源表按**白名单**匹配：只有登记过的裸路径在 token 校验之前放行；其它路径仍走正常校验与路由（未知路径回 404）。
- **静态资源不含敏感内容，故不要求 token**（浏览器 `<link>` / `<script>` 请求不携带 query token）；页面 `GET /` 与所有 JSON API 仍逐请求校验 token，写操作仍做同源校验。
- 页面所需数据一律经 JSON API 拉取，随每个请求附带 token；资源本身不做数据渲染。

## 格式约束

- **禁止外部请求**：不得出现 `http://` / `https://` 链接、CDN、外链字体或外链图片。全部资源本地嵌入，保证离线可用，并满足「无外部资源」的测试断言。
- CSS 以 `:root` 集中配色令牌（`--bg` / `--panel` / `--accent` 等），组件只引用变量，不写死颜色；页面声明 `color-scheme: dark`。
- **UTF-8 编码**；HTML 以 `<!DOCTYPE html>` 开头，`</html>` 加换行结尾。
- 面向现代浏览器（系统自带浏览器即可），不使用需要构建或转译的语法。
- 用户可见文案为英文；注释用简体中文，与代码规范一致。

## 与 Rust 的对接

- 每个资源以一个纯函数暴露，签名固定返回 `&'static str`，内部用 `include_str!`：

  ```rust
  pub fn html() -> &'static str {
      include_str!("../../../assets/settings.html")
  }

  pub fn asset(path: &str) -> Option<(&'static str, &'static str)> {
      match path {
          "/settings.css" => Some(("text/css; charset=utf-8", include_str!("../../../assets/settings.css"))),
          "/settings.js"  => Some(("text/javascript; charset=utf-8", include_str!("../../../assets/settings.js"))),
          _ => None,
      }
  }
  ```

- `include_str!` 路径相对源文件；资源参与增量编译，修改后重新 `cargo build` 即生效，**无需额外构建步骤**。
- 路由层在 token 校验前先查静态资源表，命中即按登记的 `Content-Type` 返回；未命中继续走原有 token/同源校验与路由。
- 页面与 API 的契约（路径、方法、字段名）由服务端 `handlers` 定义；改接口时同步更新页面脚本中的 `fetch` 调用与相关测试。

## 安全约束

- 服务仅监听回环地址且带 capability token；页面从 `location.search` 读取 `token` 并在数据请求中附带，不改写这一约定。
- 写操作（非 GET/HEAD）会被服务端做同源校验；页面不得从第三方来源发请求。
- 不引入 `innerHTML` 注入等 XSS 面：动态文本使用 `textContent`，避免拼接未经转义的用户数据。
- 资源内不内联任何密钥；token 只经 URL 传入。

## 编辑与验证

- 直接编辑 `assets/` 下的文件即改变前端，遵循上述格式约束。
- 迁移或拆分必须保持**输出字节不变**时，用 `git show HEAD:<旧文件>` 或抽取脚本做逐字节比对，再跑测试。
- 设置页相关测试位于 `src/settings/service/tests.rs`：HTML 断言在页面响应内，CSS / JS 断言需**单独请求对应资源**再断言；改动结构、样式或脚本时同步更新这些断言，避免测试与资源漂移。
- 提交前按影响范围执行 `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test`。
- 交互与外观改动无法由 Agent 在终端内验证，需由用户运行 `cargo run` 并在浏览器中确认。

## 不使用 Tailwind CSS

本项目**不引入 Tailwind CSS**，理由如下（评估结论，非临时取舍）：

- 三种引入方式都不符合项目定位：
  - **Play CDN**（`<script src="https://cdn.tailwindcss.com">`）：违反「零外部资源」约束，且官方声明仅用于开发；不可用于本项目。
  - **Standalone CLI**：需下载并随仓库分发一个外部可执行文件，在 `justfile` / CI 增加构建步骤与产物管理，破坏「纯 `cargo` 构建」。
  - **npm + Vite / PostCSS**：重新引入 Node 工具链，与本项目「单二进制、零前端工具链」定位冲突最大。
- 页面规模小（2 个视图、约 10 个字段），现有原生 CSS 变量主题已足够；Tailwind 的收益不足以覆盖新增的构建链与产物治理成本。
- 现有测试断言精确 CSS 片段，替换为 Tailwind 需大面积重写断言，回归风险高。

若未来确需引入，应作为独立决策走 `docs/feature/` 设计评审，并优先评估 **Standalone CLI**（唯一不引入 Node 的选项），同时把构建产物（生成的 `.css`）与扫描配置一并纳入版本管理。

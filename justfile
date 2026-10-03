# lx-tui task runner

# 运行 TUI
run:
    cargo run

# 构建
build:
    cargo build --locked

# 格式化
fmt:
    cargo fmt

# 静态检查
lint:
    cargo fmt --check
    cargo clippy --all-targets --locked -- -D warnings

# 测试
test:
    cargo test --locked

# 提交前检查
check: lint test

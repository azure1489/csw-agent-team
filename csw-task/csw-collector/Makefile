# 情报收集员工作台 · 采集服务
#
# 本机 arm64 开发，生产是 agent 主机 x86_64 glibc 2.34 → 发布走 cargo-zigbuild 交叉编译。

TARGET_LINUX := x86_64-unknown-linux-gnu.2.34
BIN := csw-collector

.PHONY: check fmt fmt-check lint test build build-linux clean

## 提交前必过：格式 + lint + 测试
check: fmt-check lint test

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

lint:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

build:
	cargo build --workspace

## 交叉编译出生产二进制（阶段 0.2 的验证对象）
build-linux:
	cargo zigbuild --release --target $(TARGET_LINUX) -p $(BIN)
	@ls -lh target/x86_64-unknown-linux-gnu/release/$(BIN)

clean:
	cargo clean

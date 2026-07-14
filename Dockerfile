# ==========================================
# 多阶段构建：Wgenty Code Rust 容器镜像
# ==========================================

# 阶段 1: 构建阶段
FROM rust:1.89-bookworm AS builder

WORKDIR /build

# 安装 GUI 构建所需的系统依赖
RUN apt-get update && apt-get install -y --no-install-recommends \
    libsqlite3-dev \
    libxcb-render0-dev \
    libxcb-shape0-dev \
    libxcb-xfixes0-dev \
    libxkbcommon-dev \
    libwayland-dev \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# 复制依赖文件
COPY Cargo.toml Cargo.lock ./

# 复制源代码
COPY src ./src

# bundled-skills 的 rust-embed 编译期打包源（SKILL.md + 支撑文件）
COPY .wgenty-code/skills ./.wgenty-code/skills

# i18n 的 rust-embed 编译期打包源（.ftl 语言文件）
COPY locales ./locales

# 构建优化版本（完整功能）
RUN cargo build --release --bin wgenty-code

# ==========================================
# 阶段 2: 运行时阶段（最小镜像）
FROM debian:bookworm-slim

LABEL maintainer="wgenty-code"
LABEL description="High-performance Wgenty Code CLI - Rust Edition"
ARG BUILD_VERSION=0.1.0
LABEL version="${BUILD_VERSION}"

# 安装必要的运行时依赖
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# 从构建阶段复制二进制文件
COPY --from=builder /build/target/release/wgenty-code /usr/local/bin/

# 创建非特权用户
RUN groupadd -r claude && \
    useradd -r -g claude -m -d /home/claude -s /usr/sbin/nologin claude

# 创建配置目录
RUN mkdir -p /home/claude/.wgenty-code && \
    chown -R claude:claude /home/claude

# 设置环境变量
ENV PATH="/usr/local/bin:${PATH}" \
    HOME="/home/claude"

# 切换到非特权用户
USER claude

# 验证安装
RUN wgenty-code --version

# 设置入口点
ENTRYPOINT ["wgenty-code"]

# 默认命令：显示帮助
CMD ["--help"]

# ==========================================
# 构建说明：
# ==========================================
# docker build -t wgenty-code:latest .
# docker build -t wgenty-code:0.1.0 .
#
# 运行数据卷挂载：
# docker run -it --rm -v ~/.wgenty-code:/home/claude/.wgenty-code wgenty-code
#
# 使用示例：
# docker run --rm wgenty-code --version
# docker run -it --rm wgenty-code repl
# docker run --rm wgenty-code query --prompt "What is Rust?"

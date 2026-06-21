# Wgenty Code Rust 安装指南

本指南将帮助您安装和配置 Wgenty Code。

## 系统要求

- **Rust**：需要安装 Rust 1.75 或更高版本
- **Git**：需要安装 Git 用于克隆仓库
- **操作系统**：支持 Windows、Linux、macOS

## 安装步骤

### 1. 克隆仓库

```bash
git clone https://github.com/zhentingWu-wzt/wgenty-code.git
cd wgenty-code
```

### 2. 构建项目

```bash
cargo build --release
```

### 3. 安装到系统

#### Windows

```powershell
# 创建安装目录
$installDir = "$env:USERPROFILE\.wgenty-code\bin"
New-Item -ItemType Directory -Path $installDir -Force

# 复制可执行文件
Copy-Item ".\target\release\wgenty-code.exe" "$installDir\wgenty-code.exe"

# 添加到 PATH
$currentPath = [Environment]::GetEnvironmentVariable("PATH", "User")
if (-not $currentPath.Contains($installDir)) {
    [Environment]::SetEnvironmentVariable("PATH", "$currentPath;$installDir", "User")
}
```

#### Linux / macOS

```bash
# 创建安装目录
mkdir -p ~/.wgenty-code/bin

# 复制可执行文件
cp ./target/release/wgenty-code ~/.wgenty-code/bin/
chmod +x ~/.wgenty-code/bin/wgenty-code

# 添加到 PATH (bash)
echo 'export PATH="$HOME/.wgenty-code/bin:$PATH"' >> ~/.bashrc

# 或对于 zsh
# echo 'export PATH="$HOME/.wgenty-code/bin:$PATH"' >> ~/.zshrc
```

### 4. 验证安装

```bash
wgenty-code --version
# 输出: wgenty-code 0.1.0
```

## 配置

### 配置文件

配置文件位于 `~/.wgenty-code/settings.json`（JSON 格式，首次运行自动生成）。

### 设置 API 密钥

推荐使用环境变量：

```bash
# Linux/macOS
export ANTHROPIC_API_KEY="sk-ant-..."

# Windows (PowerShell)
$env:ANTHROPIC_API_KEY="sk-ant-..."
```

支持的环境变量（按优先级）：
- `ANTHROPIC_API_KEY` — Anthropic API 密钥
- `DASHSCOPE_API_KEY` — 阿里云 DashScope API 密钥
- `DEEPSEEK_API_KEY` — DeepSeek API 密钥
- `API_BASE_URL` — 自定义 API 端点

### 通过命令行配置

```bash
# 查看当前配置
wgenty-code config show

# 设置模型
wgenty-code config set models.main.name haiku

# 设置 API 基础 URL
wgenty-code config set models.main.base_url "https://api.deepseek.com"

# 重置配置到默认值
wgenty-code config reset
```

## 测试安装

```bash
# 测试基本功能
wgenty-code query --prompt "Hello!"

# 启动交互模式
wgenty-code repl

# 查看帮助信息
wgenty-code --help

# 查看系统信息
wgenty-code --info
```

## 升级

```bash
cd wgenty-code
git pull
cargo build --release
# 然后重新复制可执行文件到安装目录
```

## 卸载

### Windows

```powershell
# 删除安装目录
Remove-Item -Path "$env:USERPROFILE\.wgenty-code" -Recurse -Force

# 从 PATH 中移除
$currentPath = [Environment]::GetEnvironmentVariable("PATH", "User")
$newPath = $currentPath -replace [regex]::Escape("$env:USERPROFILE\.wgenty-code\bin;"), ""
[Environment]::SetEnvironmentVariable("PATH", $newPath, "User")

# 删除配置目录
Remove-Item -Path "$env:USERPROFILE\.wgenty-code" -Recurse -Force
```

### Linux / macOS

```bash
# 删除安装目录和配置
rm -rf ~/.wgenty-code

# 从 PATH 中移除
# 编辑 ~/.bashrc 或 ~/.zshrc，删除包含 "~/.wgenty-code/bin" 的行
```

## 故障排除

### 常见问题

1. **Rust 未安装**
   - 访问 https://rustup.rs/ 安装 Rust（需要 1.75+）

2. **Git 未安装**
   - Windows: 访问 https://git-scm.com/ 安装 Git
   - Linux: 使用包管理器安装 (e.g., `apt install git`)
   - macOS: 使用 Homebrew 安装 (`brew install git`)

3. **API 密钥配置错误**
   - 确保使用正确的环境变量名（`ANTHROPIC_API_KEY` / `DASHSCOPE_API_KEY` / `DEEPSEEK_API_KEY`）
   - 确保 API 密钥具有足够的权限

4. **PATH 配置问题**
   - 安装后重启终端
   - 手动检查 PATH 环境变量是否包含安装目录

### 查看日志

```bash
# 设置详细日志
RUST_LOG=wgenty_code=debug wgenty-code query --prompt "Hello!"
```

## 联系方式

如果您遇到任何问题，请在 GitHub 仓库中创建 issue：
https://github.com/zhentingWu-wzt/wgenty-code/issues

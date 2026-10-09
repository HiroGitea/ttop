<div align="center">

# ttop

**面向 Google Cloud TPU 的终端交互式监控工具**

![Rust](https://img.shields.io/badge/Rust-2021_edition-B7410E?style=flat-square&logo=rust&logoColor=white)
![Platform](https://img.shields.io/badge/platform-Linux-0A7BBB?style=flat-square&logo=linux&logoColor=white)
![Cloud TPU](https://img.shields.io/badge/Cloud_TPU-v2_%E2%80%93_7x-4285F4?style=flat-square)
![License](https://img.shields.io/badge/license-Apache--2.0-3DA639?style=flat-square)

[English](../README.md) | **简体中文** | [日本語](README.ja.md)

[界面说明](#界面说明) · [安装](#安装) · [使用方法](#使用方法) · [数据来源](#数据来源) · [开发](#开发)

</div>

<p align="center">
  <img src="screenshot.png" alt="ttop 运行界面" width="100%">
</p>

ttop 是一款面向 Google Cloud TPU 的终端监控工具，界面风格参照 `jtop`（Jetson）与 `btop`。项目使用 Rust 与 ratatui 编写，以单个静态二进制文件发布，运行时不依赖 Python。

## 界面说明

| 区域 | 内容 |
|---|---|
| 顶栏 | 主机名、芯片型号与数量、拓扑、libtpu 连接状态与版本、刷新间隔及当前时间 |
| TPU devices | 各逻辑设备的 TensorCore 占空比（duty cycle）、HBM 占用、HBM 带宽利用率、HLO 队列长度及所属进程 |
| Host | CPU 总占用率、逐核热力条（适用于 180 核以上的 TPU 主机）、内存、Swap、网络、磁盘、系统负载及运行时长 |
| 历史曲线 | 两组可切换的历史曲线（duty / HBM / 带宽），可显示全部设备或仅显示选中设备；设备数超过 8 个时绘制平均值曲线 |
| 设备详情 | 选中设备的路径、所属进程、各项指标、TensorCore 空闲时长，以及 HLO 执行耗时（mean / p50 / p90 / p95 / p99.9） |
| TPU processes | 持有 TPU 设备文件的进程，及其占用的设备与 HBM 总量 |

## 安装

### 一键安装

```bash
curl -fsSL https://raw.githubusercontent.com/HiroGitea/ttop/main/install.sh | sh
```

安装脚本会根据当前系统架构（x86_64 或 aarch64）从 GitHub Releases 下载对应的静态二进制文件，校验 SHA-256 后安装到 `~/.local/bin`。安装在用户可写的目录中，ttop 即可在无需 `sudo` 的情况下自动更新。若该目录不在 `PATH` 中，脚本会给出需要添加到 shell 配置文件中的语句。支持以下环境变量：

| 变量 | 说明 | 默认值 |
|---|---|---|
| `TTOP_VERSION` | 要安装的版本标签，例如 `v0.1.0` | 最新版本 |
| `TTOP_INSTALL_DIR` | 安装目录；目录不可写时使用 `sudo` | `~/.local/bin` |

例如，将指定版本安装到系统目录：

```bash
curl -fsSL https://raw.githubusercontent.com/HiroGitea/ttop/main/install.sh | TTOP_VERSION=v0.1.0 TTOP_INSTALL_DIR=/usr/local/bin sh
```

如需卸载，删除安装目录中的 `ttop` 文件即可，例如 `rm ~/.local/bin/ttop`。

### 自动更新

正式发布的版本在每次启动交互界面时，会在后台检查 GitHub Releases。发现新版本后，ttop 会自动下载、校验并替换自身的二进制文件，底栏随即显示 `✓ updated to vX.Y.Z · restart to apply`；已是最新版本时不显示任何提示。

- 若二进制文件所在目录不可写（例如 `/usr/local/bin`），ttop 仅提示有新版本，可执行 `sudo ttop --update` 完成更新。
- `ttop --update` 可随时手动更新到最新版本，同样适用于从源码编译的版本（此类版本不会自动更新）。
- 非交互模式（`--once`、`--json`、`--list-metrics`）不检查更新。
- 设置环境变量 `TTOP_DISABLE_AUTOUPDATER=1` 可关闭自动更新。

### 手动下载

预编译文件发布在 [Releases](https://github.com/HiroGitea/ttop/releases) 页面：

| 文件 | 平台 |
|---|---|
| `ttop-linux-x86_64` | x86_64 Linux，静态链接（musl） |
| `ttop-linux-aarch64` | aarch64 Linux，静态链接（musl） |

下载后执行 `chmod +x` 赋予可执行权限即可运行。各文件的校验值见 `SHA256SUMS`。

### 从源码编译

```bash
git clone https://github.com/HiroGitea/ttop.git
cd ttop
cargo build --release
./target/release/ttop
```

编译无需安装 `protoc`：libtpu 指标服务的 protobuf 消息以手写的 prost 结构体实现（`src/proto.rs`）。

### 交叉编译静态二进制

如需在其他机器上编译 x86_64 静态二进制文件，再复制到 TPU VM 运行，可执行：

```bash
rustup target add x86_64-unknown-linux-musl
RUSTFLAGS="-C linker=rust-lld -C target-feature=+crt-static" \
  cargo build --release --target x86_64-unknown-linux-musl
```

## 使用方法

### 交互模式

```bash
ttop                                      # 启动交互界面
ttop --demo                               # 使用模拟数据预览界面（无需 TPU）
ttop --demo --chip 7x --demo-devices 16   # 模拟 16 个 TPU7x 逻辑设备
ttop -i 0.5                               # 将刷新间隔设为 0.5 秒
ttop -p 8431,8432                         # 指定多个 libtpu 指标端口
```

### 非交互输出

```bash
ttop --once                               # 输出一次文本快照后退出
ttop --json -n 0 -i 5                     # 每 5 秒输出一行 JSON，便于脚本采集
ttop --list-metrics                       # 列出当前 libtpu 支持的全部指标名称
ttop --update                             # 更新到最新版本
```

完整参数列表请参阅 `ttop --help`。

### 快捷键

| 按键 | 功能 |
|---|---|
| <kbd>q</kbd> / <kbd>Esc</kbd> / <kbd>Ctrl</kbd>+<kbd>C</kbd> | 退出 |
| <kbd>↑</kbd> <kbd>↓</kbd> / <kbd>k</kbd> <kbd>j</kbd> | 选择设备或进程 |
| <kbd>Tab</kbd> | 在设备列表与进程列表之间切换焦点 |
| <kbd>c</kbd> / <kbd>v</kbd> | 切换左侧 / 右侧曲线的指标（duty → HBM → 带宽） |
| <kbd>a</kbd> | 曲线在全部设备与选中设备之间切换 |
| <kbd>p</kbd> / <kbd>Space</kbd> | 暂停 / 恢复采样 |
| <kbd>+</kbd> / <kbd>-</kbd> | 延长 / 缩短刷新间隔（0.1 – 10 秒） |
| <kbd>r</kbd> | 清空曲线历史 |
| <kbd>?</kbd> / <kbd>h</kbd> / <kbd>F1</kbd> | 显示帮助 |

## 数据来源

ttop 的数据来源与 Google 官方工具 `tpu-info`（cloud-accelerator-diagnostics）保持一致。

| 数据 | 来源 |
|---|---|
| 芯片发现 | `/sys/bus/pci/devices`，筛选 Google 厂商 ID `0x1ae0`，并依据 PCI device ID 识别 v2 / v3 / v4 / v5e / v5p / v6e / TPU7x |
| 运行时指标 | libtpu 内置的 gRPC 服务 `tpu.monitoring.runtime.RuntimeMetricService`，默认地址为 `localhost:8431`。多切片任务可通过 `TPU_RUNTIME_METRICS_PORTS=8431,8432,...` 或 `-p` 指定多个端口 |
| 进程 | 扫描 `/proc/*/fd`，查找打开了 `/dev/vfio/N` 或 `/dev/accelN` 的进程 |

> [!NOTE]
> 运行时指标服务仅在 JAX、PyTorch/XLA 等框架占用 TPU 期间可用。无任务运行时，顶栏显示 “libtpu idle”，芯片、进程与主机信息仍正常显示。

> [!TIP]
> 非 root 用户仅能看到本用户的进程。如需查看其他用户的进程，请以 root 身份运行 ttop，例如 `sudo "$(command -v ttop)"`（`sudo` 默认不会在 `~/.local/bin` 中查找命令）。

## 开发

### 代码结构

```
src/main.rs     命令行参数、事件循环、--once / --json 输出
src/device.rs   通过 PCI sysfs 发现芯片，通过 /proc 查找占用进程
src/proto.rs    libtpu 指标服务的 protobuf 消息（手写 prost 结构体）
src/grpc.rs     tonic 客户端（GetRuntimeMetric / ListSupportedMetrics）
src/sampler.rs  采样：真实后端（PCI + gRPC + procfs）与 --demo 模拟后端
src/host.rs     主机 CPU / 内存 / 网络 / 磁盘 / 进程统计
src/app.rs      界面状态与历史数据
src/ui.rs       ratatui 渲染
src/update.rs   从 GitHub Releases 自动更新
```

### 测试

执行 `cargo test` 运行单元测试，覆盖范围包括 protobuf 编解码、两种 device ID 格式、按芯片复制 duty cycle 的逻辑，以及与 tpu-info 中 `_get_percentile` 一致的分位数算法。

### 发布

推送 `v*` 格式的标签后，GitHub Actions（`.github/workflows/release.yml`）会运行测试，在 x86_64 与 arm64 runner 上编译静态二进制文件，并连同 `SHA256SUMS` 一起发布为 GitHub Release。标签版本须与 `Cargo.toml` 中的 `version` 一致。该流程在编译时设置 `TTOP_RELEASE_BUILD=1` 以启用自动更新，本地编译的版本默认不启用。

```bash
git tag v0.1.0
git push origin v0.1.0
```

## 许可证

本项目基于 Apache License 2.0 发布。

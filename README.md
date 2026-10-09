<div align="center">

# ttop

**An interactive terminal monitor for Google Cloud TPU**

![Rust](https://img.shields.io/badge/Rust-2021_edition-B7410E?style=flat-square&logo=rust&logoColor=white)
![Platform](https://img.shields.io/badge/platform-Linux-0A7BBB?style=flat-square&logo=linux&logoColor=white)
![Cloud TPU](https://img.shields.io/badge/Cloud_TPU-v2_%E2%80%93_7x-4285F4?style=flat-square)
![License](https://img.shields.io/badge/license-Apache--2.0-3DA639?style=flat-square)

**English** | [简体中文](docs/README.zh-CN.md) | [日本語](docs/README.ja.md)

[Interface](#interface) · [Installation](#installation) · [Usage](#usage) · [Data sources](#data-sources) · [Development](#development)

</div>

<p align="center">
  <img src="docs/screenshot.png" alt="ttop interface" width="100%">
</p>

ttop is a terminal monitoring tool for Google Cloud TPU, with an interface modeled on `jtop` (Jetson) and `btop`. It is written in Rust with ratatui and distributed as a single static binary, with no Python runtime dependency.

## Interface

| Panel | Contents |
|---|---|
| Header | Hostname, chip type and count, topology, libtpu connection state and version, refresh interval, current time |
| TPU devices | Per logical device: TensorCore duty cycle, HBM usage, HBM bandwidth utilization, HLO queue length, owning process |
| Host | Total CPU utilization, per-core heat strip (designed for TPU hosts with 180+ cores), memory, swap, network, disk, load average, uptime |
| History charts | Two switchable history charts (duty / HBM / bandwidth) for all devices or the selected device only; an average line is drawn when there are more than 8 devices |
| Device details | Path, owning process and metrics of the selected device, TensorCore idle time, and HLO execution time (mean / p50 / p90 / p95 / p99.9) |
| TPU processes | Processes holding TPU device files, together with the devices they occupy and their total HBM usage |

## Installation

### Prebuilt binaries

The `dist/` directory contains prebuilt binaries that can be copied to a TPU VM and run directly:

- `ttop-x86_64-linux-musl` — x86_64, statically linked
- `ttop-aarch64-linux-gnu` — aarch64

### Building from source

```bash
cargo build --release
./target/release/ttop
```

`protoc` is not required: the protobuf messages of the libtpu metrics service are implemented as hand-written prost structs (`src/proto.rs`).

### Cross-compiling a static binary

To build a static x86_64 binary on another machine and copy it to a TPU VM:

```bash
rustup target add x86_64-unknown-linux-musl
RUSTFLAGS="-C linker=rust-lld -C target-feature=+crt-static" \
  cargo build --release --target x86_64-unknown-linux-musl
```

## Usage

### Interactive mode

```bash
ttop                                      # launch the interactive UI
ttop --demo                               # preview the UI with simulated data (no TPU required)
ttop --demo --chip 7x --demo-devices 16   # simulate 16 logical TPU7x devices
ttop -i 0.5                               # set the refresh interval to 0.5 s
ttop -p 8431,8432                         # query multiple libtpu metrics ports
```

### Non-interactive output

```bash
ttop --once                               # print a single text snapshot and exit
ttop --json -n 0 -i 5                     # emit one JSON line every 5 s, for use in scripts
ttop --list-metrics                       # list all metric names supported by the running libtpu
```

Run `ttop --help` for the complete list of options.

### Key bindings

| Key | Action |
|---|---|
| <kbd>q</kbd> / <kbd>Esc</kbd> / <kbd>Ctrl</kbd>+<kbd>C</kbd> | Quit |
| <kbd>↑</kbd> <kbd>↓</kbd> / <kbd>k</kbd> <kbd>j</kbd> | Select a device or process |
| <kbd>Tab</kbd> | Switch focus between the device list and the process list |
| <kbd>c</kbd> / <kbd>v</kbd> | Cycle the metric of the left / right chart (duty → HBM → bandwidth) |
| <kbd>a</kbd> | Toggle the charts between all devices and the selected device |
| <kbd>p</kbd> / <kbd>Space</kbd> | Pause / resume sampling |
| <kbd>+</kbd> / <kbd>-</kbd> | Lengthen / shorten the refresh interval (0.1 – 10 s) |
| <kbd>r</kbd> | Clear chart history |
| <kbd>?</kbd> / <kbd>h</kbd> / <kbd>F1</kbd> | Show help |

## Data sources

ttop uses the same data sources as Google's official `tpu-info` tool (cloud-accelerator-diagnostics).

| Data | Source |
|---|---|
| Chip discovery | `/sys/bus/pci/devices`, filtered by the Google vendor ID `0x1ae0`; the PCI device ID identifies v2 / v3 / v4 / v5e / v5p / v6e / TPU7x |
| Runtime metrics | libtpu's built-in gRPC service `tpu.monitoring.runtime.RuntimeMetricService`, at `localhost:8431` by default. For multi-slice jobs, specify several ports with `TPU_RUNTIME_METRICS_PORTS=8431,8432,...` or `-p` |
| Processes | A scan of `/proc/*/fd` for processes that hold `/dev/vfio/N` or `/dev/accelN` open |

> [!NOTE]
> The runtime metrics service is only available while a framework such as JAX or PyTorch/XLA holds the TPU. When no workload is running, the header shows "libtpu idle", and chip, process and host information remains available.

> [!TIP]
> Non-root users can only see their own processes. To see processes owned by other users, run ttop with `sudo`.

## Development

### Project structure

```
src/main.rs     CLI arguments, event loop, --once / --json output
src/device.rs   chip discovery via PCI sysfs, owning processes via /proc
src/proto.rs    protobuf messages of the libtpu metrics service (hand-written prost)
src/grpc.rs     tonic client (GetRuntimeMetric / ListSupportedMetrics)
src/sampler.rs  sampling: real backend (PCI + gRPC + procfs) and the --demo mock backend
src/host.rs     host CPU / memory / network / disk / process statistics
src/app.rs      UI state and history
src/ui.rs       ratatui rendering
```

### Testing

`cargo test` runs the unit tests, which cover protobuf encoding and decoding, both device ID formats, the per-chip duty cycle replication logic, and the percentile algorithm (identical to `_get_percentile` in tpu-info).

## License

ttop is released under the Apache License 2.0.

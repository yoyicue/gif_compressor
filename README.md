# GIF 压缩工具

一个高效的GIF压缩工具，能够在保持图像质量的前提下将GIF文件压缩到指定大小。支持Python和Rust两种实现方式，都具有相同的功能但采用不同的技术方案。

## 功能特性

- 智能压缩GIF文件到指定目标大小（默认500KB）
- 保持原始图像的颜色和尺寸
- 多级压缩策略，从基础优化到有损压缩
- 自动优化帧数和帧延迟时间，保持动画流畅度
- 并行评估多种压缩参数组合以找到最优解
- 详细的压缩过程日志和错误处理
- 提供完整的命令行界面和批处理能力

## 系统架构

系统包含两个平行实现版本，它们共享相同的架构概念但使用不同的技术路径：

- **压缩管道**：从基础优化开始，逐步尝试更激进的压缩技术，直到达到目标大小或找到最优结果
- **外部依赖**：两个实现都依赖`gifsicle`工具进行核心GIF处理任务
- **并行处理**：同时评估多种压缩策略以快速找到最优解

Rust实现是项目的主要焦点，提供更高效的性能和资源管理。

## 依赖

### 通用依赖

- [gifsicle](https://www.lcdf.org/gifsicle/) - 强大的GIF处理命令行工具

### Python版本依赖

- Python 3.6+
- PIL/Pillow>=9.0.0 - 图像处理库，用于GIF处理
- python-magic>=0.4.24 - 用于文件类型检测
- tqdm>=4.62.0 - 用于显示进度条，提供更好的用户体验

### Rust版本依赖

- Rust 1.50+
- clap - 命令行参数解析库
- image - Rust图像处理库
- tempfile - 临时文件处理库
- anyhow - 错误处理库
- thiserror - 声明式错误处理库
- num_cpus - 获取系统CPU核心数量的库

## 安装

### 安装gifsicle

#### macOS:
```bash
brew install gifsicle
```

#### Linux:
```bash
sudo apt-get install gifsicle  # Debian/Ubuntu
sudo yum install gifsicle      # CentOS/RHEL
```

#### Windows:
从[gifsicle官网](https://www.lcdf.org/gifsicle/)下载并安装。

### Python版本

1. 创建并激活虚拟环境（可选）:
```bash
python -m venv .venv
source .venv/bin/activate  # Linux/macOS
.venv\Scripts\activate     # Windows
```

2. 安装依赖:
```bash
pip install -r requirements.txt
```

### Rust版本

1. 使用Cargo构建项目:
```bash
cargo build --release
```

## 使用方法

### Python版本

```bash
python gif_compressor.py 输入.gif 输出.gif [--target 目标大小KB] [--min-frames 最小帧数百分比] [--threads 线程数]
```

### Rust版本

```bash
cargo run --release -- 输入.gif 输出.gif [--target 目标大小KB] [--min-frames 最小帧数百分比] [--threads 线程数]
```

或者构建后直接运行:

```bash
./target/release/gif_compressor 输入.gif 输出.gif [--target 目标大小KB] [--min-frames 最小帧数百分比] [--threads 线程数]
```

### 参数说明

- `输入.gif`: 要压缩的GIF文件路径
- `输出.gif`: 压缩后的GIF文件保存路径
- `--target`: 目标文件大小（KB），默认为500KB
- `--min-frames`: 保留的最小帧数百分比，默认为原始帧数的10%
- `--threads`: 并行处理线程/进程数，默认为系统CPU核心数量（0表示自动检测）

## 压缩策略

本工具采用多阶段渐进式压缩方法：

1. **基础优化** - 使用gifsicle的`-O3`优化级别进行初步优化
2. **帧抽取** - 基于可配置参数智能跳过部分帧
3. **帧延迟调整** - 根据跳帧比例自动调整帧间延迟时间，保持动画流畅度
4. **有损压缩** - 应用多个级别的有损压缩（30-240）
5. **并行策略评估** - 同时测试多种策略组合，快速找到最优结果

每个压缩策略由以下组成：
- **跳帧值**：保留多少帧（例如，skip=2表示保留每2帧）
- **延迟值**：帧之间的时间间隔，按比例调整以保持动画速度

## 实现差异对比

两种实现在核心功能一致的情况下，技术实现细节有明显差异：

| 特性 | Python 实现 | Rust 实现 |
|------|------------|-----------|
| **并发模型** | 多进程 (`multiprocessing.Pool`) | 多线程 (`thread` + `mpsc` 通道) |
| **错误处理** | 异常处理 (try/except) | 结构化错误处理 (自定义`GifError`枚举和`Result`类型) |
| **资源管理** | 基本文件清理 | `TempFile`结构体与`Drop`特性自动资源管理 |
| **线程协作** | 进程池简单通信 | 原子操作与共享状态(`Arc<SharedState>`) |
| **性能表现** | 中等（受Python GIL限制） | 更高（高效内存管理和线程模型） |
| **内存安全** | 运行时检查 | 编译时保证 |
| **临时文件管理** | 手动跟踪和`os.unlink()`清理 | 结构化的资源管理和自动清理 |

Rust版本在大批量处理和性能敏感场景下表现更好，而Python版本更适合快速开发和原型验证。

## 示例

### Python版本

#### 将GIF压缩到500KB以下:
```bash
python gif_compressor.py input.gif output.gif
```

#### 将GIF压缩到500KB以下，保留至少20%的帧:
```bash
python gif_compressor.py input.gif output.gif --target 500 --min-frames 20
```

#### 使用4个进程并行处理:
```bash
python gif_compressor.py input.gif output.gif --threads 4
```

### Rust版本

#### 使用Cargo运行:
```bash
cargo run --release -- input.gif output.gif
cargo run --release -- input.gif output.gif --target 500 --min-frames 20
cargo run --release -- input.gif output.gif --threads 8
```

#### 使用编译后的二进制文件:
```bash
./target/release/gif_compressor input.gif output.gif
./target/release/gif_compressor input.gif output.gif --target 500 --min-frames 20
./target/release/gif_compressor input.gif output.gif --threads 8
```

## 批量处理脚本

提供了 `compress_gifs.sh` 脚本，可自动寻找最优参数并批量处理 GIF 文件：

```bash
# 编译工具
cargo build --release

# 运行脚本
./compress_gifs.sh
```

该脚本会自动测试从 95% 到 10% 的不同 min-frames 参数，为每个GIF文件找到最佳的压缩设置，能在保持足够质量的前提下达到目标大小。

### 可配置参数

```bash
# 脚本顶部可修改的配置
INPUT_DIR="input"        # 输入目录
OUTPUT_DIR="output"      # 输出目录
TARGET_SIZE=500          # 目标大小(KB)
MAX_MIN_FRAMES=95        # 最大测试值(%)
MIN_MIN_FRAMES=10        # 最小测试值(%)
STEP_SIZE=5              # 测试步长(%)
```

## 注意事项

- 如果无法达到目标大小，工具会输出最接近目标大小的结果
- 压缩大文件或帧数多的GIF可能需要较长时间
- 某些复杂的GIF可能需要更多手动优化才能达到很小的目标大小
- 批处理模式对于大量GIF文件处理特别有效
- Rust版本提供更好的资源管理和性能，特别适合服务器端部署 
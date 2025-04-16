# GIF 压缩工具

一个高效的GIF压缩工具，能够在保持图像质量的前提下将GIF文件压缩到指定大小。支持Python和Rust两种实现方式。

## 功能特性

- 智能压缩GIF文件到指定目标大小（如500KB）
- 保持图像的颜色数量和尺寸
- 自动优化帧数和帧延迟
- 使用多种压缩策略寻找最佳压缩效果
- 支持多线程并行处理，提高压缩速度
- 提供详细的压缩过程日志

## 依赖

### 通用依赖

- [gifsicle](https://www.lcdf.org/gifsicle/) - 强大的GIF处理命令行工具

### Python版本依赖

- Python 3.6+
- PIL/Pillow - Python图像处理库
- Pillow>=9.0.0 - 图像处理库，用于GIF处理
- python-magic>=0.4.24 - 用于文件类型检测
- tqdm>=4.62.0 - 用于显示进度条，提供更好的用户体验

### Rust版本依赖

- Rust 1.50+
- clap - 命令行参数解析库
- image - Rust图像处理库
- tempfile - 临时文件处理库
- anyhow - 错误处理库
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
pip install Pillow
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
python gif_compressor.py 输入.gif 输出.gif [--target 目标大小KB] [--min-frames 最小帧数百分比]
```

### Rust版本

```bash
cargo run --release -- 输入.gif 输出.gif [--target 目标大小KB] [--min-frames 最小帧数百分比]
```

或者构建后直接运行:

```bash
./target/release/gif_compressor 输入.gif 输出.gif [--target 目标大小KB] [--min-frames 最小帧数百分比]
```

### 参数说明

- `输入.gif`: 要压缩的GIF文件路径
- `输出.gif`: 压缩后的GIF文件保存路径
- `--target`: 目标文件大小（KB），默认为500KB
- `--min-frames`: 保留的最小帧数百分比，默认为原始帧数的10%
- `--threads`: 并行处理线程数，默认为系统CPU核心数量（0表示自动检测）

## 压缩策略

本工具使用以下策略来压缩GIF:

1. 基础优化 - 使用gifsicle的O3优化级别
2. 帧抽取 - 通过跳过一定数量的帧来减小文件大小
3. Lossy压缩 - 使用gifsicle的lossy参数进行有损压缩
4. 帧延迟调整 - 根据抽帧比例自动调整帧延迟时间
5. 多线程并行处理 - 同时尝试多种压缩策略，加快处理速度并找到最优解

工具会自动尝试多种组合策略，并选择最佳压缩效果。

## 示例

### Python版本

#### 将GIF压缩到500KB以下:

```bash
python gif_compressor.py input.gif output.gif
```

#### 将GIF压缩到300KB以下，保留至少20%的帧:

```bash
python gif_compressor.py input.gif output.gif --target 300 --min-frames 20
```

### Rust版本

#### 使用Cargo运行:

##### 将GIF压缩到500KB以下:

```bash
cargo run --release -- input.gif output.gif
```

##### 将GIF压缩到300KB以下，保留至少20%的帧:

```bash
cargo run --release -- input.gif output.gif --target 300 --min-frames 20
```

##### 使用8个线程进行并行处理:

```bash
cargo run --release -- input.gif output.gif --threads 8
```

#### 使用编译后的二进制文件:

##### 将GIF压缩到500KB以下:

```bash
./target/release/gif_compressor input.gif output.gif
```

##### 将GIF压缩到300KB以下，保留至少20%的帧:

```bash
./target/release/gif_compressor input.gif output.gif --target 300 --min-frames 20
```

##### 使用8个线程进行并行处理:

```bash
./target/release/gif_compressor input.gif output.gif --threads 8
```

## 注意事项

- 如果无法达到目标大小，工具会输出最接近目标大小的结果
- 压缩大文件可能需要较长时间
- 某些复杂的GIF可能需要更多手动优化才能达到很小的目标大小 
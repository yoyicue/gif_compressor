#!/bin/bash

# 定义路径和选项
EXECUTABLE="target/debug/gif_compressor"
INPUT_DIR="input"
OUTPUT_DIR="output"
TEMP_DIR="temp_compression_tests"
TARGET_SIZE=500 # 目标大小 (KB)
MAX_MIN_FRAMES=95 # 最大 min-frames 值（起始值）
MIN_MIN_FRAMES=10 # 最小 min-frames 值（结束值）
STEP_SIZE=5 # 每次减少的幅度

# 创建一个获取文件大小的函数 (KB)
get_file_size_kb() {
  local file="$1"
  if [[ "$OSTYPE" == "darwin"* ]]; then
    # macOS
    stat -f%z "$file" | awk '{print $1/1024}'
  else
    # Linux
    stat --format="%s" "$file" | awk '{print $1/1024}'
  fi
}

# 检查可执行文件是否存在
if [ ! -f "$EXECUTABLE" ]; then
  echo "错误：在 $EXECUTABLE 未找到可执行文件"
  echo "请先使用 'cargo build' 编译项目"
  exit 1
fi

# 检查输入目录是否存在
if [ ! -d "$INPUT_DIR" ]; then
  echo "错误：输入目录 $INPUT_DIR 不存在"
  exit 1
fi

# 创建输出目录（如果不存在）
mkdir -p "$OUTPUT_DIR"

# 创建临时目录（如果不存在）并在脚本结束时清理
mkdir -p "$TEMP_DIR"
trap 'rm -rf "$TEMP_DIR"' EXIT

echo "开始处理 $INPUT_DIR 中的 GIF 文件..."

# 查找并处理所有 gif 文件
find "$INPUT_DIR" -maxdepth 1 -name "*.gif" -print0 | while IFS= read -r -d $'\0' input_file; do
  filename=$(basename "$input_file")
  output_file="$OUTPUT_DIR/$filename"
  
  echo "==============================================="
  echo "分析: $filename"
  
  # 获取原始文件大小
  original_size=$(get_file_size_kb "$input_file")
  echo "  原始大小: ${original_size}KB"
  
  # 如果原始文件已经小于目标大小，直接复制
  if (( $(echo "$original_size < $TARGET_SIZE" | bc -l) )); then
    echo "  已经小于目标大小 ${TARGET_SIZE}KB，无需压缩"
    cp "$input_file" "$output_file"
    continue
  fi
  
  best_size=0
  best_min_frames=0
  best_temp_file=""
  
  # 从 MAX_MIN_FRAMES 测试到 MIN_MIN_FRAMES
  for min_frames in $(seq $MAX_MIN_FRAMES -$STEP_SIZE $MIN_MIN_FRAMES); do
    echo "  尝试 min_frames=$min_frames"
    temp_output="${TEMP_DIR}/${filename}.${min_frames}"
    
    # 执行压缩命令，添加 --target 参数
    "$EXECUTABLE" "$input_file" "$temp_output" --min-frames "$min_frames" --target "$TARGET_SIZE"
    
    # 检查命令是否成功
    if [ $? -ne 0 ]; then
      echo "    压缩失败，跳过"
      continue
    fi
    
    # 获取压缩后的大小
    compressed_size=$(get_file_size_kb "$temp_output")
    echo "    压缩后大小: ${compressed_size}KB"
    
    # 如果大小小于目标大小且大于当前最佳大小，更新最佳结果
    if (( $(echo "$compressed_size < $TARGET_SIZE" | bc -l) )) && 
       (( $(echo "$compressed_size > $best_size" | bc -l) )); then
      best_size=$compressed_size
      best_min_frames=$min_frames
      best_temp_file=$temp_output
      echo "    ✓ 更新最佳结果: ${compressed_size}KB (min_frames=$min_frames)"
    fi
  done
  
  # 如果找到符合条件的结果，使用它
  if [ "$best_min_frames" -ne 0 ]; then
    echo "  【最佳结果】: min_frames=$best_min_frames, 大小=${best_size}KB"
    echo "  (小于 ${TARGET_SIZE}KB 且最接近目标大小)"
    cp "$best_temp_file" "$output_file"
  else
    echo "  警告: 未找到小于 ${TARGET_SIZE}KB 的结果，尝试最低压缩率"
    # 使用最低的 min_frames 值重新压缩，同时传递 target 参数
    "$EXECUTABLE" "$input_file" "$output_file" --min-frames "$MIN_MIN_FRAMES" --target "$TARGET_SIZE"
    final_size=$(get_file_size_kb "$output_file")
    echo "  使用 min_frames=$MIN_MIN_FRAMES 压缩后大小: ${final_size}KB"
  fi
  
  echo "压缩完成: $filename"
  echo "==============================================="
done

echo "所有文件处理完成。输出文件位于 $OUTPUT_DIR 目录。"

# 临时目录将由 trap 命令自动清理
exit 0 
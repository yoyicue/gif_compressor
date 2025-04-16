import os
import subprocess
import tempfile
import argparse
from PIL import Image, ImageSequence
import math
import shutil
import multiprocessing
import time  # 添加时间模块导入

def get_file_size_kb(file_path):
    """获取文件大小（KB）"""
    return os.path.getsize(file_path) / 1024

def get_frame_count(gif_path):
    """获取GIF的帧数"""
    with Image.open(gif_path) as img:
        return sum(1 for _ in ImageSequence.Iterator(img))

def extract_frames(input_gif, output_pattern, skip=1, delay=10):
    """提取GIF的帧并保存为单独的GIF文件"""
    with Image.open(input_gif) as img:
        frames = []
        for i, frame in enumerate(ImageSequence.Iterator(img)):
            if i % skip == 0:  # 每隔skip帧取一帧
                frames.append(frame.copy())
    
    # 获取原始GIF的信息
    with Image.open(input_gif) as img:
        original_duration = img.info.get('duration', 100)  # 默认100ms
        loop = img.info.get('loop', 0)  # 默认无限循环
    
    # 应用新的延迟
    if delay is None:
        # 根据跳帧比例调整延迟
        delay = original_duration * skip
    
    # 保存为新的GIF文件
    if frames:
        frames[0].save(
            output_pattern,
            save_all=True,
            append_images=frames[1:],
            optimize=False,  # 由gifsicle优化
            duration=delay,
            loop=loop
        )
    else:
        # 至少保留一帧
        with Image.open(input_gif) as img:
            first_frame = next(ImageSequence.Iterator(img))
            first_frame.save(output_pattern, duration=delay, loop=loop)

def process_strategy(strategy_data):
    """处理单个压缩策略"""
    input_path, strategy, target_size_kb, process_id = strategy_data
    skip = strategy['skip']
    delay = strategy['delay']
    
    # 输出前缀，用于区分不同进程
    prefix = f"进程 {process_id}: "
    
    # 预计剩余帧数
    original_frame_count = get_frame_count(input_path)
    expected_frames = math.ceil(original_frame_count / skip)
    print(f"{prefix}策略: 保留约 {expected_frames} 帧 (每 {skip} 帧取1帧), 帧延迟: {delay}ms")
    
    # 提取帧
    try:
        temp_file_frames = tempfile.NamedTemporaryFile(suffix='.gif', delete=False).name
        extract_frames(input_path, temp_file_frames, skip, delay)
        
        # 检查提取是否成功
        if not os.path.exists(temp_file_frames) or get_file_size_kb(temp_file_frames) < 1:
            print(f"{prefix}  帧提取失败")
            return {
                'success': False,
                'size': float('inf'),
                'file': None
            }
    except Exception as e:
        print(f"{prefix}  帧提取出错: {e}")
        return {
            'success': False,
            'size': float('inf'),
            'file': None
        }
    
    # 优化提取后的帧
    try:
        temp_file_frames_opt = tempfile.NamedTemporaryFile(suffix='.gif', delete=False).name
        subprocess.run(['gifsicle', '-O3', temp_file_frames, '-o', temp_file_frames_opt], check=True)
        os.unlink(temp_file_frames)  # 删除未优化的版本
        temp_file_frames = temp_file_frames_opt
        
        frames_size = get_file_size_kb(temp_file_frames)
        print(f"{prefix}  抽帧后大小: {frames_size:.2f} KB")
        
        if frames_size <= target_size_kb:
            print(f"{prefix}  已达到目标大小!")
            return {
                'success': True,
                'size': frames_size,
                'file': temp_file_frames
            }
    except Exception as e:
        print(f"{prefix}  帧优化出错: {e}")
        if os.path.exists(temp_file_frames):
            os.unlink(temp_file_frames)
        return {
            'success': False,
            'size': float('inf'),
            'file': None
        }
    
    # 尝试不同的lossy值
    best_size = frames_size
    best_file = temp_file_frames
    
    for lossy_level in [30, 60, 90, 120, 150, 180, 210, 240]:
        try:
            temp_final = tempfile.NamedTemporaryFile(suffix='.gif', delete=False).name
            subprocess.run(['gifsicle', '-O3', '--lossy=' + str(lossy_level), 
                        best_file, '-o', temp_final], check=True)
            
            final_size = get_file_size_kb(temp_final)
            print(f"{prefix}  抽帧 + lossy={lossy_level} 后大小: {final_size:.2f} KB")
            
            if final_size <= target_size_kb:
                print(f"{prefix}  已达到目标大小!")
                if best_size > final_size:
                    if os.path.exists(best_file) and best_file != temp_file_frames:
                        os.unlink(best_file)
                    best_size = final_size
                    best_file = temp_final
                else:
                    os.unlink(temp_final)
                break
            
            if final_size < best_size:
                if os.path.exists(best_file) and best_file != temp_file_frames:
                    os.unlink(best_file)
                best_size = final_size
                best_file = temp_final
            else:
                os.unlink(temp_final)
        except Exception as e:
            print(f"{prefix}  lossy={lossy_level}压缩出错: {e}")
            if os.path.exists(temp_final):
                os.unlink(temp_final)
    
    return {
        'success': True,
        'size': best_size,
        'file': best_file
    }

def optimize_gif(input_path, output_path, target_size_kb, min_frame_percent=10, threads=0):
    """压缩GIF到目标大小，保持颜色数量和尺寸 (并行版本)"""
    # 设置线程数
    if threads <= 0:
        threads = multiprocessing.cpu_count()
    
    # 初始文件大小
    original_size = get_file_size_kb(input_path)
    print(f"原始大小: {original_size:.2f} KB")
    
    if original_size <= target_size_kb:
        print("文件已经小于目标大小，无需压缩")
        shutil.copy(input_path, output_path)
        return
    
    # 获取初始帧数
    original_frame_count = get_frame_count(input_path)
    print(f"原始帧数: {original_frame_count}")
    
    # 基础优化 - 使用gifsicle的最高优化级别
    temp_file_opt = tempfile.NamedTemporaryFile(suffix='.gif', delete=False).name
    subprocess.run(['gifsicle', '-O3', input_path, '-o', temp_file_opt], check=True)
    
    opt_size = get_file_size_kb(temp_file_opt)
    print(f"基础优化后大小: {opt_size:.2f} KB")
    
    if opt_size <= target_size_kb:
        shutil.move(temp_file_opt, output_path)
        return
    
    # 计算最小保留帧数
    min_frames = max(3, int(original_frame_count * min_frame_percent / 100))
    
    # 生成压缩策略
    strategies = []
    
    # 从2抽1开始，最多抽到保留最小帧数
    max_skip = max(2, min(10, math.ceil(original_frame_count / min_frames)))
    for skip in range(2, max_skip + 1):
        strategies.append({
            'skip': skip,
            'delay': int(100 * skip / original_frame_count) + 10  # 根据抽帧比例调整延迟
        })
    
    # 如果帧数很多，尝试更激进的抽帧策略
    if original_frame_count > 30:
        aggressive_skips = [max_skip + 5, max_skip + 10]
        for skip in aggressive_skips:
            if original_frame_count / skip >= min_frames:
                strategies.append({
                    'skip': skip,
                    'delay': int(100 * skip / original_frame_count) + 10
                })
    
    # 限制线程数不超过策略数量
    actual_threads = min(threads, len(strategies))
    print(f"开始使用 {actual_threads} 个进程并行处理 {len(strategies)} 个压缩策略...")
    
    # 准备并行处理的数据
    process_data = [
        (input_path, strategy, target_size_kb, i+1) 
        for i, strategy in enumerate(strategies)
    ]
    
    # 创建进程池处理多个策略
    best_size = opt_size
    best_file = temp_file_opt
    
    with multiprocessing.Pool(processes=actual_threads) as pool:
        # 异步执行所有策略
        results = []
        for result in pool.imap_unordered(process_strategy, process_data):
            results.append(result)
            
            # 如果找到满足条件的结果，提前结束
            if result['success'] and result['size'] <= target_size_kb:
                pool.terminate()  # 立即终止所有进程
                break
    
    # 分析结果，找出最佳结果
    for result in results:
        if not result['success']:
            continue
            
        if result['size'] <= target_size_kb:
            # 已达到目标大小
            if best_file != temp_file_opt and os.path.exists(best_file):
                os.unlink(best_file)
            best_size = result['size']
            best_file = result['file']
            print(f"找到达到目标大小的策略! 大小: {best_size:.2f} KB")
            break
        elif result['size'] < best_size:
            # 大小比之前的结果更好
            if best_file != temp_file_opt and os.path.exists(best_file):
                os.unlink(best_file)  # 删除之前的最佳文件
            best_size = result['size']
            best_file = result['file']
    
    # 使用找到的最佳文件
    print("\n复制最佳结果到输出文件...")
    shutil.move(best_file, output_path)
    
    # 清理临时文件
    if os.path.exists(temp_file_opt) and temp_file_opt != best_file:
        os.unlink(temp_file_opt)
    
    # 如果还是没达到目标大小，给出提示
    if best_size > target_size_kb:
        print(f"\n无法达到目标大小 {target_size_kb} KB。")
        print(f"最接近的大小是 {best_size:.2f} KB，已保存到输出文件。")
        print("建议尝试允许减少尺寸或颜色数量以达到更小的文件大小。")

def main():
    # 记录开始时间
    start_time = time.time()
    
    parser = argparse.ArgumentParser(description='GIF压缩工具 - 保持颜色和尺寸')
    parser.add_argument('input', help='输入GIF文件路径')
    parser.add_argument('output', help='输出GIF文件路径')
    parser.add_argument('--target', type=float, default=500, help='目标文件大小(KB)，默认500KB')
    parser.add_argument('--min-frames', type=int, default=10, 
                        help='保留的最小帧数百分比，默认为原始帧数的10%')
    parser.add_argument('--threads', type=int, default=0,
                        help='并行处理线程数，默认为系统CPU核心数（0表示自动检测）')
    
    args = parser.parse_args()
    
    # 检查输入文件是否存在
    if not os.path.exists(args.input):
        print(f"错误: 输入文件 '{args.input}' 不存在")
        return
    
    # 确保目标路径的目录存在
    output_dir = os.path.dirname(args.output)
    if output_dir and not os.path.exists(output_dir):
        os.makedirs(output_dir)
    
    thread_count = args.threads if args.threads > 0 else multiprocessing.cpu_count()
    print(f"开始压缩 '{args.input}' 到 '{args.output}' (目标: {args.target} KB, 线程数: {thread_count})")
    optimize_gif(args.input, args.output, args.target, args.min_frames, args.threads)
    
    final_size = get_file_size_kb(args.output)
    print(f"完成! 最终大小: {final_size:.2f} KB")
    
    # 计算并输出处理时间
    elapsed_ms = int((time.time() - start_time) * 1000)
    print(f"处理了 {elapsed_ms} 毫秒")

if __name__ == "__main__":
    main()
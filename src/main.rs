use anyhow::{Context, Result};
use clap::{App, Arg};
use image::{codecs::gif::GifDecoder, AnimationDecoder};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Sender, Receiver};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use tempfile::NamedTempFile;

/// 获取文件大小（KB）
fn get_file_size_kb<P: AsRef<Path>>(path: P) -> Result<f64> {
    let metadata = fs::metadata(path).context("获取文件元数据失败")?;
    Ok(metadata.len() as f64 / 1024.0)
}

/// 获取GIF的帧数
fn get_frame_count<P: AsRef<Path>>(path: P) -> Result<usize> {
    let file = File::open(path).context("打开文件失败")?;
    let decoder = GifDecoder::new(BufReader::new(file)).context("创建GIF解码器失败")?;
    let frames = decoder.into_frames();
    let count = frames.count();
    Ok(count)
}

/// 提取GIF帧并保存为新的GIF
fn extract_frames<P: AsRef<Path>, Q: AsRef<Path>>(
    input_path: P,
    output_path: Q,
    skip: usize,
    delay: u16,
) -> Result<()> {
    // 打开输入文件
    let file = File::open(&input_path).context("打开输入文件失败")?;
    let decoder = GifDecoder::new(BufReader::new(file)).context("创建GIF解码器失败")?;
    
    // 提取所有帧
    let frames = decoder.into_frames().collect_frames().context("收集GIF帧失败")?;
    let total_frames = frames.len();
    
    // 根据skip参数选择帧
    let mut selected_frames = Vec::new();
    for i in (0..total_frames).step_by(skip) {
        selected_frames.push(frames[i].clone());
    }
    
    if selected_frames.is_empty() {
        // 至少保留一帧
        if !frames.is_empty() {
            selected_frames.push(frames[0].clone());
        } else {
            return Err(anyhow::anyhow!("输入GIF没有帧"));
        }
    }
    
    // 由于GIF格式复杂，我们使用临时目录和gifsicle来完成帧提取和合并
    let temp_dir = tempfile::Builder::new()
        .prefix("gif_frames_")
        .tempdir()
        .context("创建临时目录失败")?;
    
    // 保存所有选择的帧到临时目录，并收集路径字符串
    let mut frame_paths = Vec::new();
    for (i, frame) in selected_frames.iter().enumerate() {
        let frame_path = temp_dir.path().join(format!("frame_{}.gif", i));
        let frame_file = File::create(&frame_path).context("创建临时帧文件失败")?;
        let mut frame_writer = BufWriter::new(frame_file);
        
        // 使用image库保存单帧GIF
        frame.buffer().write_to(&mut frame_writer, image::ImageOutputFormat::Gif)
            .context("写入帧数据失败")?;
        
        // 保存路径字符串
        frame_paths.push(frame_path.to_string_lossy().to_string());
    }
    
    // 使用gifsicle合并帧
    let output_path_str = output_path.as_ref().to_string_lossy().to_string();
    let delay_str = delay.to_string();
    
    // 检查gifsicle是否存在
    match Command::new("gifsicle").arg("--version").output() {
        Ok(_) => {}, // 命令存在，继续执行
        Err(_) => return Err(anyhow::anyhow!("未找到gifsicle命令，请确保已安装"))
    }
    
    // 构建优化的参数列表
    let mut gifsicle_args = Vec::with_capacity(frame_paths.len() + 8);
    
    // 添加优化选项
    gifsicle_args.push("--no-warnings".to_string());        // 减少不必要的输出
    gifsicle_args.push("--no-conserve-memory".to_string()); // 使用更多内存提高速度
    gifsicle_args.push("--no-app-extensions".to_string());  // 移除应用扩展数据
    gifsicle_args.push("--no-comments".to_string());        // 移除注释
    gifsicle_args.push("--no-names".to_string());           // 移除名称元数据
    gifsicle_args.push("-o".to_string());
    gifsicle_args.push(output_path_str);
    gifsicle_args.push("--delay".to_string());
    gifsicle_args.push(delay_str);
    gifsicle_args.push("--loopcount=forever".to_string());
    
    // 添加所有帧路径 (已经是String类型)
    for path in &frame_paths {
        gifsicle_args.push(path.clone());
    }
    
    // 执行gifsicle命令
    let _output = Command::new("gifsicle")
        .args(&gifsicle_args)
        .output()
        .context("执行gifsicle命令失败")?;
    
    // 检查命令是否成功
    if !_output.status.success() {
        let stderr = String::from_utf8_lossy(&_output.stderr);
        return Err(anyhow::anyhow!("gifsicle命令执行失败: {}", stderr));
    }
    
    Ok(())
}

/// 表示临时文件 - 优化版本
struct TempFile {
    path: PathBuf,
}

impl TempFile {
    fn new(temp_file: NamedTempFile) -> Self {
        // 将临时文件转换为保留路径但取消自动删除的版本
        let path = temp_file.path().to_path_buf();
        let _temp_path = temp_file.into_temp_path();
        // 这里_temp_path会被丢弃，但文件不会被删除
        Self { path }
    }
    
    fn path_str(&self) -> String {
        self.path.to_string_lossy().to_string()
    }
    
    // 当不再需要文件时手动删除
    fn cleanup(&self) -> std::io::Result<()> {
        if self.path.exists() {
            std::fs::remove_file(&self.path)?;
        }
        Ok(())
    }
}

// Drop实现会在TempFile被丢弃时尝试删除文件
impl Drop for TempFile {
    fn drop(&mut self) {
        // 尝试删除文件，但忽略任何错误
        let _ = self.cleanup();
    }
}

// Clone实现，允许复制TempFile
impl Clone for TempFile {
    fn clone(&self) -> Self {
        Self {
            path: self.path.clone(),
        }
    }
}

/// 压缩策略结构
struct Strategy {
    skip: usize,
    delay: u16,
}

/// 策略处理结果
struct StrategyResult {
    size: f64,
    file: Option<TempFile>,
    success: bool,
}

/// 共享状态结构体，用于线程间通信
struct SharedState {
    // 是否找到满足目标大小的结果
    found_target: AtomicBool,
    // 当前已找到的最佳大小，初始值设为最大值
    best_size: std::sync::atomic::AtomicU64,
}

impl SharedState {
    fn new() -> Self {
        Self {
            found_target: AtomicBool::new(false),
            best_size: std::sync::atomic::AtomicU64::new(u64::MAX),
        }
    }
    
    // 更新最佳大小（如果提供的大小更小）
    fn update_best_size(&self, size: f64) -> bool {
        let size_bits = size.to_bits();
        let mut current = self.best_size.load(Ordering::Relaxed);
        
        loop {
            // 如果新大小不比当前更好，不更新
            if size_bits >= current {
                return false;
            }
            
            // 尝试原子更新，成功则返回true
            match self.best_size.compare_exchange(
                current,
                size_bits,
                Ordering::SeqCst,
                Ordering::Relaxed
            ) {
                Ok(_) => return true,
                Err(actual) => current = actual,
            }
        }
    }
    
    // 获取当前最佳大小
    fn get_best_size(&self) -> f64 {
        let bits = self.best_size.load(Ordering::Relaxed);
        f64::from_bits(bits)
    }
    
    // 设置已找到目标
    fn set_found_target(&self) {
        self.found_target.store(true, Ordering::Relaxed);
    }
    
    // 检查是否已找到目标
    fn is_target_found(&self) -> bool {
        self.found_target.load(Ordering::Relaxed)
    }
}

/// 处理单个策略
fn process_strategy(
    input_path: &str,
    strategy: Strategy,
    target_size_kb: f64,
    thread_id: usize,
    shared_state: &SharedState,
) -> StrategyResult {
    // 创建跟踪输出的记录器
    let output_prefix = format!("线程 {}: ", thread_id);
    let log = |msg: &str| {
        let message = format!("{}{}", output_prefix, msg);
        // 使用Mutex来确保输出不会被打断
        println!("{}", message);
    };
    
    // 如果已经找到目标，立即返回
    if shared_state.is_target_found() {
        log("已有其他线程找到满足条件的结果，提前退出");
        return StrategyResult {
            size: f64::MAX,
            file: None,
            success: false,
        };
    }
    
    let skip = strategy.skip;
    let delay = strategy.delay;
    
    // 预计剩余帧数
    let expected_frames = match get_frame_count(input_path) {
        Ok(count) => (count as f64 / skip as f64).ceil() as usize,
        Err(_) => 0,
    };
    
    log(&format!("策略: 保留约 {} 帧 (每 {} 帧取1帧), 帧延迟: {}ms", 
                expected_frames, skip, delay));
    
    // 使用image库提取帧
    let temp_frames = match NamedTempFile::new() {
        Ok(file) => TempFile::new(file),
        Err(_) => {
            log("  创建临时文件失败");
            return StrategyResult {
                size: f64::MAX,
                file: None,
                success: false,
            };
        }
    };
    
    // 检查是否有线程已经找到结果
    if shared_state.is_target_found() {
        log("已有其他线程找到满足条件的结果，提前退出");
        return StrategyResult {
            size: f64::MAX,
            file: None,
            success: false,
        };
    }
    
    let temp_frames_path = temp_frames.path_str();
    
    if let Err(e) = extract_frames(input_path, &temp_frames_path, skip, delay) {
        log(&format!("  帧提取失败: {}", e));
        return StrategyResult {
            size: f64::MAX,
            file: None,
            success: false,
        };
    }
    
    // 检查是否有线程已经找到结果
    if shared_state.is_target_found() {
        log("已有其他线程找到满足条件的结果，提前退出");
        return StrategyResult {
            size: f64::MAX,
            file: None,
            success: false,
        };
    }
    
    // 检查提取是否成功
    match get_file_size_kb(&temp_frames_path) {
        Ok(size) if size < 1.0 => {
            log("  帧提取生成的文件过小");
            return StrategyResult {
                size: f64::MAX,
                file: None,
                success: false,
            };
        },
        Ok(_) => {}, // 文件大小正常，继续处理
        Err(_) => {
            log("  无法读取提取的帧大小");
            return StrategyResult {
                size: f64::MAX,
                file: None,
                success: false,
            };
        }
    };
    
    // 优化提取后的帧
    let temp_frames_opt = match NamedTempFile::new() {
        Ok(file) => TempFile::new(file),
        Err(_) => {
            log("  创建优化临时文件失败");
            return StrategyResult {
                size: f64::MAX,
                file: None,
                success: false,
            };
        }
    };
    
    // 检查是否有线程已经找到结果
    if shared_state.is_target_found() {
        log("已有其他线程找到满足条件的结果，提前退出");
        return StrategyResult {
            size: f64::MAX,
            file: None,
            success: false,
        };
    }
    
    let temp_frames_opt_path = temp_frames_opt.path_str();
    
    let args = vec!["-O3", &temp_frames_path, "-o", &temp_frames_opt_path];
    
    let _output = match Command::new("gifsicle")
        .args(&args)
        .output() {
        Ok(output) => output,
        Err(_) => {
            log("  执行gifsicle帧优化失败");
            return StrategyResult {
                size: f64::MAX,
                file: None,
                success: false,
            };
        }
    };
    
    if !_output.status.success() {
        log("  帧优化失败");
        return StrategyResult {
            size: f64::MAX,
            file: None,
            success: false,
        };
    }
    
    // 清理第一个临时文件，不再需要它
    let _ = temp_frames.cleanup();
    
    let frames_size = match get_file_size_kb(&temp_frames_opt_path) {
        Ok(size) => size,
        Err(_) => {
            log("  无法读取优化后帧大小");
            return StrategyResult {
                size: f64::MAX,
                file: None,
                success: false,
            };
        }
    };
    
    log(&format!("  抽帧后大小: {:.2} KB", frames_size));
    
    if frames_size <= target_size_kb {
        log("  已达到目标大小!");
        // 设置标志通知其他线程已找到满足条件的结果
        shared_state.set_found_target();
        return StrategyResult {
            size: frames_size,
            file: Some(temp_frames_opt),
            success: true,
        };
    }
    
    // 跟踪当前策略下的最佳结果
    let mut best_size = frames_size;
    let mut best_file = Some(temp_frames_opt);
    
    // 批量尝试不同的lossy值
    // 创建临时文件和对应的lossy级别
    let lossy_levels = [30, 60, 90, 120, 150, 180, 210, 240];
    
    // 每次处理两个lossy级别，平衡进程创建开销和并行效率
    let chunk_size = 2;
    
    for chunk in lossy_levels.chunks(chunk_size) {
        // 先检查是否有线程已经找到结果
        if shared_state.is_target_found() {
            log("已有其他线程找到满足条件的结果，提前退出");
            return StrategyResult {
                size: best_size,
                file: best_file,
                success: true,
            };
        }
        
        let mut temp_files = Vec::with_capacity(chunk.len());
        let mut results = Vec::with_capacity(chunk.len());
        
        // 创建这一批次的临时文件
        for &level in chunk {
            match NamedTempFile::new() {
                Ok(file) => {
                    temp_files.push((level, TempFile::new(file)));
                },
                Err(_) => {
                    log(&format!("  创建lossy={}临时文件失败", level));
                }
            }
        }
        
        let current_best_path = match &best_file {
            Some(file) => file.path_str(),
            None => break,
        };
        
        // 处理这一批次的lossy级别
        for (level, temp_file) in &temp_files {
            let temp_path = temp_file.path_str();
            
            // 创建lossy参数
            let lossy_arg = format!("--lossy={}", level);
            
            // 优化的gifsicle命令参数
            let args = vec![
                "-O3", 
                "--no-warnings",
                "--no-conserve-memory", 
                "--no-comments", 
                "--no-names",
                &lossy_arg,
                &current_best_path, 
                "-o", 
                &temp_path
            ];
            
            let _output = match Command::new("gifsicle")
                .args(&args)
                .output() {
                Ok(output) if output.status.success() => {
                    match get_file_size_kb(&temp_path) {
                        Ok(size) => {
                            log(&format!("  抽帧 + lossy={} 后大小: {:.2} KB", level, size));
                            results.push((*level, size));
                        },
                        Err(_) => {
                            log(&format!("  无法读取lossy={}压缩后大小", level));
                        }
                    }
                },
                _ => {
                    log(&format!("  lossy={}压缩失败", level));
                }
            };
        }
        
        // 处理这一批次的结果
        for (_result_idx, (level, size)) in results.iter().enumerate() {
            if *size <= target_size_kb {
                log(&format!("  lossy={} 已达到目标大小!", level));
                
                // 找到对应的临时文件
                if let Some((_, temp_file)) = temp_files.iter().find(|(l, _)| *l == *level) {
                    // 如果当前结果比之前的好，替换并清理旧文件
                    if best_size > *size {
                        if let Some(old_file) = best_file.take() {
                            let _ = old_file.cleanup(); // 清理旧文件
                        }
                        best_size = *size;
                        best_file = Some(temp_file.clone());
                    }
                }
                
                // 设置标志通知其他线程已找到满足条件的结果
                shared_state.set_found_target();
                break;
            } else if *size < best_size {
                // 找到对应的临时文件
                if let Some((_, temp_file)) = temp_files.iter().find(|(l, _)| *l == *level) {
                    // 替换旧文件并清理
                    if let Some(old_file) = best_file.take() {
                        let _ = old_file.cleanup(); // 清理旧文件
                    }
                    best_size = *size;
                    best_file = Some(temp_file.clone());
                }
            }
        }
        
        // 如果已找到目标，不再处理更多批次
        if shared_state.is_target_found() {
            break;
        }
        
        // 清理这批次中未被选中的临时文件
        for (_level, temp_file) in &temp_files {
            if let Some(best) = &best_file {
                if best.path != temp_file.path {
                    let _ = temp_file.cleanup();
                }
            } else {
                let _ = temp_file.cleanup();
            }
        }
    }
    
    StrategyResult {
        size: best_size,
        file: best_file,
        success: true,
    }
}

/// 优化GIF到目标大小 (并行版本)
fn optimize_gif<P: AsRef<Path>, Q: AsRef<Path>>(
    input_path: P,
    output_path: Q,
    target_size_kb: f64,
    min_frame_percent: u32,
    threads: usize,
) -> Result<()> {
    // 获取初始文件大小
    let original_size = get_file_size_kb(&input_path)?;
    println!("原始大小: {:.2} KB", original_size);
    
    // 如果已经小于目标大小，直接复制
    if original_size <= target_size_kb {
        println!("文件已经小于目标大小，无需压缩");
        fs::copy(&input_path, &output_path)?;
        return Ok(());
    }
    
    // 获取初始帧数
    let original_frame_count = get_frame_count(&input_path)?;
    println!("原始帧数: {}", original_frame_count);
    
    // 检查gifsicle是否存在
    match Command::new("gifsicle").arg("--version").output() {
        Ok(_) => {}, // 命令存在，继续执行
        Err(_) => return Err(anyhow::anyhow!("未找到gifsicle命令，请确保已安装"))
    }
    
    // 基础优化 - 使用gifsicle的最高优化级别和更多高级选项
    let temp_file = NamedTempFile::new()?;
    let temp_file_opt = TempFile::new(temp_file);
    let temp_file_opt_path = temp_file_opt.path_str();
    
    // 使用String而不是&str，避免生命周期问题
    let input_path_str = input_path.as_ref().to_string_lossy().to_string();
    
    // 构建优化的参数列表
    let args = vec![
        "-O3",                            // 最高级别优化
        "--no-warnings",                  // 不显示警告
        "--no-conserve-memory",           // 使用更多内存以提高速度
        "--no-comments",                  // 删除注释以减小文件大小
        "--no-names",                     // 删除图像和对象名称
        "--careful",                      // 更慎重的优化，避免损坏文件
        &input_path_str,                  // 输入文件
        "-o",                             // 输出选项
        &temp_file_opt_path               // 输出文件
    ];
    
    let _output = Command::new("gifsicle")
        .args(&args)
        .output()
        .context("执行gifsicle基础优化失败")?;
    
    if !_output.status.success() {
        let stderr = String::from_utf8_lossy(&_output.stderr);
        return Err(anyhow::anyhow!("gifsicle基础优化失败: {}", stderr));
    }
    
    let opt_size = get_file_size_kb(&temp_file_opt_path)?;
    println!("基础优化后大小: {:.2} KB", opt_size);
    
    // 如果已经达到目标大小，直接复制
    if opt_size <= target_size_kb {
        fs::copy(&temp_file_opt_path, &output_path)?;
        return Ok(());
    }
    
    // 计算最小保留帧数
    let min_frames = std::cmp::max(3, (original_frame_count as f64 * min_frame_percent as f64 / 100.0) as usize);
    
    // 构建抽帧策略
    let mut strategies = Vec::new();
    
    // 从2抽1开始，最多抽到保留最小帧数
    let max_skip = std::cmp::max(2, std::cmp::min(10, 
        ((original_frame_count as f64) / (min_frames as f64)).ceil() as usize));
    
    for skip in 2..=max_skip {
        strategies.push(Strategy {
            skip,
            delay: ((100.0 * skip as f64) / original_frame_count as f64) as u16 + 10,
        });
    }
    
    // 如果帧数很多，尝试更激进的抽帧策略
    if original_frame_count > 30 {
        let aggressive_skips = [max_skip + 5, max_skip + 10];
        for &skip in &aggressive_skips {
            if original_frame_count / skip >= min_frames {
                strategies.push(Strategy {
                    skip,
                    delay: ((100.0 * skip as f64) / original_frame_count as f64) as u16 + 10,
                });
            }
        }
    }
    
    // 限制线程数，不超过策略数量
    let thread_count = std::cmp::min(threads, strategies.len());
    println!("开始使用 {} 个线程并行处理 {} 个压缩策略...", thread_count, strategies.len());
    
    // 创建通道以接收处理结果
    let (tx, rx): (Sender<StrategyResult>, Receiver<StrategyResult>) = mpsc::channel();
    
    // 创建线程池
    let input_path_arc = Arc::new(input_path_str);
    let mut handles = Vec::new();
    
    // 创建共享状态
    let shared_state = Arc::new(SharedState::new());
    
    // 设置初始最佳大小为基础优化后的大小
    shared_state.update_best_size(opt_size);
    
    for (i, chunk) in strategies.into_iter().enumerate() {
        let tx_clone = tx.clone();
        let input_path_clone = Arc::clone(&input_path_arc);
        let shared_state_clone = Arc::clone(&shared_state);
        
        // 创建线程处理这个策略
        let handle = thread::spawn(move || {
            let result = process_strategy(
                &input_path_clone,
                chunk,
                target_size_kb,
                i + 1,
                &shared_state_clone
            );
            
            // 如果这是一个好的结果，更新共享状态中的最佳大小
            if result.success && result.size < shared_state_clone.get_best_size() {
                let is_better = shared_state_clone.update_best_size(result.size);
                
                // 如果我们的结果被接受为更好的结果，并且达到了目标大小，设置found_target标志
                if is_better && result.size <= target_size_kb {
                    shared_state_clone.set_found_target();
                }
            }
            
            // 发送结果到主线程
            let _ = tx_clone.send(result);
        });
        
        handles.push(handle);
    }
    
    // 丢弃发送者以允许接收者知道何时所有发送者都已完成
    drop(tx);
    
    // 等待并收集所有策略的结果
    let mut best_size = opt_size;
    let mut best_file: Option<TempFile> = Some(temp_file_opt);
    let mut found_solution = false;
    
    // 从通道接收结果
    for result in rx.iter() {
        if !result.success {
            continue;
        }
        
        if result.size <= target_size_kb {
            // 清理之前的最佳文件（如果有的话）
            if let Some(old_file) = best_file.take() {
                let _ = old_file.cleanup();
            }
            
            best_size = result.size;
            best_file = result.file;
            found_solution = true;
            println!("找到达到目标大小的策略! 大小: {:.2} KB", best_size);
            // 设置标志，以便其他线程可以提前退出
            shared_state.set_found_target();
            break; // 提前退出循环，不再处理其他结果
        } else if result.size < best_size {
            // 清理之前的最佳文件（如果有的话）
            if let Some(old_file) = best_file.take() {
                let _ = old_file.cleanup();
            }
            
            best_size = result.size;
            best_file = result.file;
        } else if result.file.is_some() {
            // 该结果不比当前最佳结果好，清理它
            if let Some(file) = result.file {
                let _ = file.cleanup();
            }
        }
    }
    
    // 我们不再等待所有线程完成
    // 如果已经找到满足条件的结果，其他线程会自动退出
    // 如果我们想要优雅地等待，可以设置一个超时
    if found_solution {
        println!("已找到满足条件的结果，不再等待其他线程");
    } else {
        println!("尚未找到满足目标大小的结果，等待所有线程完成...");
        // 等待所有线程完成
        for handle in handles {
            let _ = handle.join();
        }
    }
    
    // 使用找到的最佳文件
    if let Some(best) = best_file {
        println!("\n复制最佳结果到输出文件...");
        fs::copy(&best.path, &output_path)
            .context("复制最佳结果到输出文件失败")?;
        
        // 复制完成后清理临时文件
        let _ = best.cleanup();
        
        let final_size = get_file_size_kb(&output_path)?;
        println!("完成! 最终大小: {:.2} KB", final_size);
    } else {
        return Err(anyhow::anyhow!("没有找到有效的优化结果"));
    }
    
    // 如果还是没达到目标大小，给出提示
    if best_size > target_size_kb {
        println!("\n无法达到目标大小 {} KB。", target_size_kb);
        println!("最接近的大小是 {:.2} KB，已保存到输出文件。", best_size);
        println!("建议尝试允许减少尺寸或颜色数量以达到更小的文件大小。");
    }
    
    Ok(())
}

fn main() -> Result<()> {
    // 解析命令行参数
    let matches = App::new("GIF压缩工具")
        .version("1.0")
        .author("Rust GIF Compressor")
        .about("压缩GIF到目标大小，保持颜色和尺寸")
        .arg(Arg::with_name("input")
            .help("输入GIF文件路径")
            .required(true)
            .index(1))
        .arg(Arg::with_name("output")
            .help("输出GIF文件路径")
            .required(true)
            .index(2))
        .arg(Arg::with_name("target")
            .long("target")
            .help("目标文件大小(KB)，默认500KB")
            .takes_value(true)
            .default_value("500"))
        .arg(Arg::with_name("min-frames")
            .long("min-frames")
            .help("保留的最小帧数百分比，默认为原始帧数的10%")
            .takes_value(true)
            .default_value("10"))
        .arg(Arg::with_name("threads")
            .long("threads")
            .help("并行处理线程数，默认为系统CPU核心数")
            .takes_value(true)
            .default_value("0"))
        .get_matches();
    
    let input = matches.value_of("input").unwrap();
    let output = matches.value_of("output").unwrap();
    let target = matches.value_of("target")
        .unwrap()
        .parse::<f64>()
        .unwrap_or(500.0);
    let min_frames = matches.value_of("min-frames")
        .unwrap()
        .parse::<u32>()
        .unwrap_or(10);
    let threads = matches.value_of("threads")
        .unwrap()
        .parse::<usize>()
        .unwrap_or(0);
    
    // 如果线程数为0，使用系统CPU核心数
    let thread_count = if threads == 0 {
        num_cpus::get()
    } else {
        threads
    };
    
    // 检查输入文件是否存在
    if !Path::new(input).exists() {
        return Err(anyhow::anyhow!("错误: 输入文件 '{}' 不存在", input));
    }
    
    // 确保目标路径的目录存在
    if let Some(parent) = Path::new(output).parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }
    
    println!("开始压缩 '{}' 到 '{}' (目标: {} KB, 线程数: {})", 
             input, output, target, thread_count);
    optimize_gif(input, output, target, min_frames, thread_count)?;
    
    Ok(())
}
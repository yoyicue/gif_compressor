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
    
    // 构建参数列表，确保所有字符串都已经拥有
    let mut gifsicle_args = vec![
        "-o".to_string(), 
        output_path_str, 
        "--delay".to_string(), 
        delay_str, 
        "--loopcount=forever".to_string()
    ];
    
    // 添加所有帧路径 (已经是String类型)
    for path in frame_paths {
        gifsicle_args.push(path);
    }
    
    // 执行gifsicle命令
    let output = Command::new("gifsicle")
        .args(&gifsicle_args)
        .output()
        .context("执行gifsicle命令失败")?;
    
    // 检查命令是否成功
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
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

/// 处理单个策略
fn process_strategy(
    input_path: &str,
    strategy: Strategy,
    target_size_kb: f64,
    thread_id: usize,
    found_target: &AtomicBool,
) -> StrategyResult {
    // 创建跟踪输出的记录器
    let output_prefix = format!("线程 {}: ", thread_id);
    let log = |msg: &str| {
        let message = format!("{}{}", output_prefix, msg);
        // 使用Mutex来确保输出不会被打断
        println!("{}", message);
    };
    
    // 如果已经找到目标，立即返回
    if found_target.load(Ordering::Relaxed) {
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
    if found_target.load(Ordering::Relaxed) {
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
    if found_target.load(Ordering::Relaxed) {
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
    if found_target.load(Ordering::Relaxed) {
        log("已有其他线程找到满足条件的结果，提前退出");
        return StrategyResult {
            size: f64::MAX,
            file: None,
            success: false,
        };
    }
    
    let temp_frames_opt_path = temp_frames_opt.path_str();
    
    let args = vec!["-O3", &temp_frames_path, "-o", &temp_frames_opt_path];
    let output = match Command::new("gifsicle")
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
    
    if !output.status.success() {
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
        found_target.store(true, Ordering::Relaxed);
        return StrategyResult {
            size: frames_size,
            file: Some(temp_frames_opt),
            success: true,
        };
    }
    
    // 跟踪当前策略下的最佳结果
    let mut best_size = frames_size;
    let mut best_file = Some(temp_frames_opt);
    
    // 尝试不同的lossy值
    for lossy_level in [30, 60, 90, 120, 150, 180, 210, 240].iter() {
        // 检查是否有线程已经找到结果
        if found_target.load(Ordering::Relaxed) {
            log("已有其他线程找到满足条件的结果，提前退出");
            return StrategyResult {
                size: best_size,
                file: best_file,
                success: true,
            };
        }
        
        let temp_final = match NamedTempFile::new() {
            Ok(file) => TempFile::new(file),
            Err(_) => {
                log(&format!("  创建lossy={}临时文件失败", lossy_level));
                continue;
            }
        };
        
        let temp_final_path = temp_final.path_str();
        
        let lossy_arg = format!("--lossy={}", lossy_level);
        let current_best_path = match &best_file {
            Some(file) => file.path_str(),
            None => continue,
        };
        
        let args = vec!["-O3", &lossy_arg, &current_best_path, "-o", &temp_final_path];
        
        let output = match Command::new("gifsicle")
            .args(&args)
            .output() {
            Ok(output) => output,
            Err(_) => {
                log(&format!("  执行gifsicle lossy={}压缩失败", lossy_level));
                continue;
            }
        };
        
        if !output.status.success() {
            log(&format!("  lossy={}压缩失败", lossy_level));
            continue;
        }
        
        let final_size = match get_file_size_kb(&temp_final_path) {
            Ok(size) => size,
            Err(_) => {
                log(&format!("  无法读取lossy={}压缩后大小", lossy_level));
                continue;
            }
        };
        
        log(&format!("  抽帧 + lossy={} 后大小: {:.2} KB", lossy_level, final_size));
        
        if final_size <= target_size_kb {
            log("  已达到目标大小!");
            // 如果当前结果比之前的好，替换并清理旧文件
            if best_size > final_size {
                if let Some(old_file) = best_file.take() {
                    let _ = old_file.cleanup(); // 清理旧文件
                }
                best_size = final_size;
                best_file = Some(temp_final);
            } else {
                // 当前结果不如之前的好，清理
                let _ = temp_final.cleanup();
            }
            // 设置标志通知其他线程已找到满足条件的结果
            found_target.store(true, Ordering::Relaxed);
            break;
        }
        
        if final_size < best_size {
            // 替换旧文件并清理
            if let Some(old_file) = best_file.take() {
                let _ = old_file.cleanup(); // 清理旧文件
            }
            best_size = final_size;
            best_file = Some(temp_final);
        } else {
            // 当前结果不如之前的好，清理
            let _ = temp_final.cleanup();
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
    
    // 基础优化 - 使用gifsicle的最高优化级别
    let temp_file = NamedTempFile::new()?;
    let temp_file_opt = TempFile::new(temp_file);
    let temp_file_opt_path = temp_file_opt.path_str();
    
    // 使用String而不是&str，避免生命周期问题
    let input_path_str = input_path.as_ref().to_string_lossy().to_string();
    let args = vec!["-O3", &input_path_str, "-o", &temp_file_opt_path];
    
    let output = Command::new("gifsicle")
        .args(&args)
        .output()
        .context("执行gifsicle基础优化失败")?;
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
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
    
    // 创建一个原子布尔值来标记是否找到满足条件的结果
    let found_target = Arc::new(AtomicBool::new(false));
    
    for (i, chunk) in strategies.into_iter().enumerate() {
        let tx_clone = tx.clone();
        let input_path_clone = Arc::clone(&input_path_arc);
        let found_target_clone = Arc::clone(&found_target);
        
        // 创建线程处理这个策略
        let handle = thread::spawn(move || {
            let result = process_strategy(
                &input_path_clone,
                chunk,
                target_size_kb,
                i + 1,
                &found_target_clone
            );
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
            found_target.store(true, Ordering::Relaxed);
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
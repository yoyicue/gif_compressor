use anyhow::{Context, Result};
use clap::{App, Arg};
use image::{codecs::gif::GifDecoder, AnimationDecoder};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::{NamedTempFile, TempPath};

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

/// 表示临时文件
struct TempGifFile {
    path: PathBuf,
    _temp_file: Option<TempPath>, // 保持临时文件存活
}

impl TempGifFile {
    fn new(temp_file: NamedTempFile) -> Self {
        let path = temp_file.path().to_path_buf();
        let _temp_file = Some(temp_file.into_temp_path()); // 转换为TempPath以防止自动删除
        Self { path, _temp_file }
    }
    
    fn path_str(&self) -> String {
        self.path.to_string_lossy().to_string()
    }
}

/// 优化GIF到目标大小
fn optimize_gif<P: AsRef<Path>, Q: AsRef<Path>>(
    input_path: P,
    output_path: Q,
    target_size_kb: f64,
    min_frame_percent: u32,
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
    let temp_file_opt = TempGifFile::new(temp_file);
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
    struct Strategy {
        skip: usize,
        delay: u16,
    }
    
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
    
    println!("开始尝试不同的抽帧和压缩策略...");
    
    // 跟踪最佳结果
    let mut best_size = opt_size;
    let mut best_file: Option<TempGifFile> = Some(temp_file_opt);
    
    // 尝试不同的策略
    for strategy in strategies {
        let skip = strategy.skip;
        let delay = strategy.delay;
        
        // 预计剩余帧数
        let expected_frames = (original_frame_count as f64 / skip as f64).ceil() as usize;
        println!("\n策略: 保留约 {} 帧 (每 {} 帧取1帧), 帧延迟: {}ms", expected_frames, skip, delay);
        
        // 使用image库提取帧
        let temp_file = NamedTempFile::new()?;
        let temp_frames = TempGifFile::new(temp_file);
        let temp_frames_path = temp_frames.path_str();
        
        if let Err(e) = extract_frames(&input_path, &temp_frames_path, skip, delay) {
            println!("  帧提取失败: {}, 跳过此策略", e);
            continue;
        }
        
        // 检查提取是否成功
        if let Ok(frames_size) = get_file_size_kb(&temp_frames_path) {
            if frames_size < 1.0 {
                println!("  帧提取生成的文件过小，跳过此策略");
                continue;
            }
        } else {
            println!("  无法读取提取的帧大小，跳过此策略");
            continue;
        }
        
        // 优化提取后的帧
        let temp_file = NamedTempFile::new()?;
        let temp_frames_opt = TempGifFile::new(temp_file);
        let temp_frames_opt_path = temp_frames_opt.path_str();
        
        let args = vec!["-O3", &temp_frames_path, "-o", &temp_frames_opt_path];
        let output = Command::new("gifsicle")
            .args(&args)
            .output()
            .context("执行gifsicle帧优化失败")?;
        
        if !output.status.success() {
            println!("  帧优化失败，跳过此策略");
            continue;
        }
        
        let frames_size = get_file_size_kb(&temp_frames_opt_path)?;
        println!("  抽帧后大小: {:.2} KB", frames_size);
        
        if frames_size <= target_size_kb {
            println!("  已达到目标大小!");
            if best_size > frames_size || best_size > target_size_kb {
                best_size = frames_size;
                best_file = Some(temp_frames_opt);
            }
            continue;
        }
        
        // 尝试不同的lossy值
        for lossy_level in [30, 60, 90, 120, 150, 180, 210, 240].iter() {
            let temp_file = NamedTempFile::new()?;
            let temp_final = TempGifFile::new(temp_file);
            let temp_final_path = temp_final.path_str();
            
            let lossy_arg = format!("--lossy={}", lossy_level);
            let args = vec!["-O3", &lossy_arg, &temp_frames_opt_path, "-o", &temp_final_path];
            
            let output = Command::new("gifsicle")
                .args(&args)
                .output()
                .context("执行gifsicle lossy压缩失败")?;
            
            if !output.status.success() {
                println!("  lossy压缩失败，跳过此lossy级别");
                continue;
            }
            
            let final_size = get_file_size_kb(&temp_final_path)?;
            println!("  抽帧 + lossy={} 后大小: {:.2} KB", lossy_level, final_size);
            
            if final_size <= target_size_kb {
                println!("  已达到目标大小!");
                if best_size > final_size {
                    best_size = final_size;
                    best_file = Some(temp_final);
                }
                break;
            }
            
            if final_size < best_size {
                best_size = final_size;
                best_file = Some(temp_final);
            }
        }
        
        // 如果这个策略下的最佳大小已经达到目标，就不再尝试其他策略
        if best_size <= target_size_kb {
            break;
        }
    }
    
    // 使用找到的最佳文件
    if let Some(best) = best_file {
        println!("\n复制最佳结果到输出文件...");
        fs::copy(&best.path, &output_path)
            .context("复制最佳结果到输出文件失败")?;
        
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
    
    println!("开始压缩 '{}' 到 '{}' (目标: {} KB)", input, output, target);
    optimize_gif(input, output, target, min_frames)?;
    
    Ok(())
}
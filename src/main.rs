use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use futures::StreamExt;
use tokio::process::Command;

// ========== 配置 ==========
const QAAC_PATH: &str = r"D:\Apps\Portable\Qaac\qaac_3.03\x64\qaac.exe";
const VBR_QUALITY: &str = "82";
const ENCODE_QUALITY: &str = "2";
const MAX_WORKERS: usize = 8;
// ==========================

fn decode_bytes(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => {
            let (cow, _, _) = encoding_rs::GBK.decode(bytes);
            cow.into_owned()
        }
    }
}

async fn convert_file(flac: PathBuf, output_dir: PathBuf) -> (PathBuf, Result<(), String>) {
    let stem = flac
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let output = output_dir.join(format!("{stem}.m4a"));

    let result = Command::new(QAAC_PATH)
        .arg("-V").arg(VBR_QUALITY)
        .arg("-q").arg(ENCODE_QUALITY)
        .arg("--copy-artwork")
        .arg(&flac)
        .arg("-o").arg(&output)
        .output()
        .await;

    let res = match result {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => {
            let stdout = decode_bytes(&out.stdout);
            let stderr = decode_bytes(&out.stderr);
            let mut msg = String::new();
            if !stdout.trim().is_empty() {
                msg.push_str(&format!("--- qaac stdout ---\n{}\n", stdout.trim_end()));
            }
            if !stderr.trim().is_empty() {
                msg.push_str(&format!("--- qaac stderr ---\n{}\n", stderr.trim_end()));
            }
            if msg.is_empty() {
                msg = format!("qaac 退出码: {:?}，无输出", out.status.code());
            }
            Err(msg)
        }
        Err(e) => Err(format!("启动 qaac 失败: {e}")),
    };

    (flac, res)
}

async fn get_input(prompt: &'static str, allow_empty: bool) -> String {
    loop {
        let value = tokio::task::spawn_blocking(move || {
            use std::io::Write;
            print!("{prompt}");
            let _ = std::io::stdout().flush();
            let mut line = String::new();
            let _ = std::io::stdin().read_line(&mut line);
            line.trim().trim_matches('"').to_string()
        })
        .await
        .unwrap_or_default();

        if !value.is_empty() || allow_empty {
            return value;
        }
        println!("⚠️ 不能为空，请重新输入");
    }
}

async fn wait_enter() {
    let _ = tokio::task::spawn_blocking(|| {
        let mut s = String::new();
        let _ = std::io::stdin().read_line(&mut s);
    })
    .await;
}

fn is_flac(p: &Path) -> bool {
    p.extension()
        .map(|e| e.eq_ignore_ascii_case("flac"))
        .unwrap_or(false)
}

fn collect_files(source: &Path) -> Result<(Vec<PathBuf>, bool), String> {
    if source.is_file() {
        if !is_flac(source) {
            return Err(format!("⚠️ 不是 FLAC 文件: {}", source.display()));
        }
        Ok((vec![source.to_path_buf()], false))
    } else if source.is_dir() {
        let mut files: Vec<(PathBuf, SystemTime)> = std::fs::read_dir(source)
            .map_err(|e| format!("❌ 读取目录失败: {e}"))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_file() && is_flac(p))
            .map(|p| {
                let mtime = p.metadata().and_then(|m| m.modified()).unwrap_or(UNIX_EPOCH);
                (p, mtime)
            })
            .collect();

        if files.is_empty() {
            return Err(format!("⚠️ 文件夹中没有 FLAC 文件: {}", source.display()));
        }
        files.sort_by_key(|(_, t)| *t);
        Ok((files.into_iter().map(|(p, _)| p).collect(), true))
    } else {
        Err("❌ 无效路径".to_string())
    }
}

#[tokio::main]
async fn main() {
    println!("{}", "=".repeat(50));
    println!("🎵 FLAC → M4A 批量转换工具 (异步并发版)");
    println!("参数: VBR {VBR_QUALITY}, -q {ENCODE_QUALITY}, 保留元数据");
    println!("并发任务数: {MAX_WORKERS}");
    println!("{}", "=".repeat(50));

    println!("\n📂 请输入 FLAC 文件或文件夹路径");
    println!("   - 单个文件: D:\\音乐\\歌.flac");
    println!("   - 文件夹: D:\\音乐\\纯音乐");
    println!("   - 支持拖拽路径到此窗口（粘贴即可）");
    let source = get_input("路径: ", false).await;
    let source_path = PathBuf::from(&source);

    if !source_path.exists() {
        println!("❌ 路径不存在: {source}");
        println!("\n按 Enter 键退出...");
        wait_enter().await;
        return;
    }

    println!("\n📁 请输入输出目录（可选，直接回车则自动创建 m4a 子目录）");
    let output = get_input("输出目录: ", true).await;
    let output_path = if output.is_empty() {
        if source_path.is_file() {
            source_path.parent().unwrap_or(Path::new(".")).join("m4a")
        } else {
            source_path.join("m4a")
        }
    } else {
        PathBuf::from(output)
    };
    println!("📁 输出目录: {}", output_path.display());

    println!("\n按 Enter 键开始转换...");
    wait_enter().await;

    let (files, from_dir) = match collect_files(&source_path) {
        Ok(v) => v,
        Err(msg) => {
            println!("{msg}");
            println!("\n按 Enter 键退出...");
            wait_enter().await;
            return;
        }
    };
    if from_dir {
        println!(
            "📁 找到 {} 个 FLAC 文件，按修改时间从旧到新转换...\n",
            files.len()
        );
    }

    if let Err(e) = std::fs::create_dir_all(&output_path) {
        println!("❌ 无法创建输出目录: {e}");
        println!("\n按 Enter 键退出...");
        wait_enter().await;
        return;
    }

    let total = files.len();
    println!("🚀 使用 {MAX_WORKERS} 个并发任务转换...\n");

    let mut stream = futures::stream::iter(files)
        .map(|f| {
            let out = output_path.clone();
            async move { convert_file(f, out).await }
        })
        .buffer_unordered(MAX_WORKERS);

    let mut success = 0usize;
    let mut done = 0usize;
    while let Some((path, result)) = stream.next().await {
        done += 1;
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();

        match result {
            Ok(()) => {
                success += 1;
                println!("  ✅ [{done}/{total}] {name} 转换成功");
            }
            Err(details) => {
                println!("  ❌ [{done}/{total}] {name} 转换失败\n{details}");
            }
        }
    }

    println!("\n🎉 转换完成！成功: {success}, 失败: {}", total - success);
    println!("📁 输出目录: {}", output_path.display());

    println!("\n按 Enter 键退出...");
    wait_enter().await;
}
